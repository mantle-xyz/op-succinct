use alloy_primitives::B256;
use anyhow::Result;
use async_trait::async_trait;
use kona_host::single::{SingleChainHost, SingleChainHostError};
use kona_preimage::{BidirectionalChannel, Channel};
use tokio::task::JoinHandle;

use crate::{fetcher::OPSuccinctDataFetcher, witness_generation::WitnessGenerator};

#[async_trait]
pub trait PreimageServerStarter {
    async fn start_server<C>(
        &self,
        hint: C,
        preimage: C,
    ) -> Result<JoinHandle<Result<(), SingleChainHostError>>, SingleChainHostError>
    where
        C: Channel + Send + Sync + 'static;
}

#[async_trait]
impl PreimageServerStarter for SingleChainHost {
    async fn start_server<C>(
        &self,
        hint: C,
        preimage: C,
    ) -> Result<JoinHandle<Result<(), SingleChainHostError>>, SingleChainHostError>
    where
        C: Channel + Send + Sync + 'static,
    {
        self.start_server(hint, preimage).await
    }
}

#[async_trait]
pub trait OPSuccinctHost: Send + Sync + 'static {
    type Args: Send + Sync + 'static + Clone + PreimageServerStarter;
    type WitnessGenerator: WitnessGenerator + Send + Sync;

    fn witness_generator(&self) -> &Self::WitnessGenerator;

    /// Fetch the host arguments.
    ///
    /// Parameters:
    /// - `l2_start_block`: The starting L2 block number.
    /// - `l2_end_block`: The ending L2 block number.
    /// - `l1_head_hash`: Optionally supplied L1 head block hash used as the L1 origin.
    /// - `safe_db_fallback`: Flag to indicate whether to fallback to timestamp-based L1 head
    ///   estimation when SafeDB is not available.
    async fn fetch(
        &self,
        l2_start_block: u64,
        l2_end_block: u64,
        l1_head_hash: Option<B256>,
        safe_db_fallback: bool,
    ) -> Result<Self::Args>;

    /// Run the host and client program.
    ///
    /// Returns the witness which can be supplied to the zkVM.
    ///
    /// If the server task exits before the witness generation completes (e.g. due to a preimage
    /// fetch timeout causing the channel to close), we abort the witness generation immediately
    /// and return an error so the caller can retry with a fresh channel.
    async fn run(
        &self,
        args: &Self::Args,
    ) -> Result<<Self::WitnessGenerator as WitnessGenerator>::WitnessData> {
        let preimage = BidirectionalChannel::new()?;
        let hint = BidirectionalChannel::new()?;

        let server_task = args.start_server(hint.host, preimage.host).await?;

        // Race the witness generation against the server task. If the server exits first
        // (e.g. channel error / preimage timeout), the witness generator would be stuck
        // in a "Channel is closed" loop. Detect this and fail fast.
        let witness_fut = self.witness_generator().run(preimage.client, hint.client);
        tokio::pin!(witness_fut);
        tokio::pin!(server_task);

        tokio::select! {
            witness_result = &mut witness_fut => {
                // Witness generation completed (success or error). Clean up server.
                server_task.abort();
                Ok(witness_result?)
            }
            server_result = &mut server_task => {
                // Server exited before witness generation finished — channel is dead.
                // The witness generator would loop on "Channel is closed" forever.
                let err_msg = match server_result {
                    Ok(Ok(())) => "Server exited unexpectedly before witness generation completed".to_string(),
                    Ok(Err(e)) => format!("Server exited with error: {e}"),
                    Err(e) => format!("Server task panicked: {e}"),
                };
                anyhow::bail!("{err_msg}. Witness generation aborted — will retry with a fresh channel.")
            }
        }
    }

    /// Get the L1 head hash from the host args.
    fn get_l1_head_hash(&self, args: &Self::Args) -> Option<B256>;

    /// Get the finalized L2 block number. This is used to determine the highest block that can be
    /// included in a range proof.
    ///
    /// For ETH DA, this is the finalized L2 block number.
    /// For Celestia, this is the highest L2 block included in the latest Blobstream commitment.
    ///
    /// The latest proposed block number is assumed to be the highest block number that has been
    /// successfully processed by the host.
    async fn get_finalized_l2_block_number(
        &self,
        fetcher: &OPSuccinctDataFetcher,
        latest_proposed_block_number: u64,
    ) -> Result<Option<u64>>;

    /// Calculate a safe L1 head hash for the given L2 end block.
    ///
    /// This method is DA-specific:
    /// - For ETH DA: Uses simple offset logic.
    /// - For Celestia DA: Uses blobstream commitment logic to ensure data availability.
    ///
    /// Parameters:
    /// - `fetcher`: The data fetcher for accessing blockchain data.
    /// - `l2_end_block`: The ending L2 block number for the range.
    /// - `safe_db_fallback`: Whether to fallback to timestamp-based estimation when SafeDB is
    ///   unavailable.
    async fn calculate_safe_l1_head(
        &self,
        fetcher: &OPSuccinctDataFetcher,
        l2_end_block: u64,
        safe_db_fallback: bool,
    ) -> Result<B256>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Simulates the core `select!` logic in `OPSuccinctHost::run()`.
    /// When the server exits before the witness generator, we should get a fast error
    /// instead of the witness generator looping forever on "Channel is closed".
    async fn run_with_select(
        server_fut: JoinHandle<Result<(), String>>,
        witness_fut: impl std::future::Future<Output = Result<String>>,
    ) -> Result<String> {
        tokio::pin!(witness_fut);
        tokio::pin!(server_fut);

        tokio::select! {
            witness_result = &mut witness_fut => {
                server_fut.abort();
                Ok(witness_result?)
            }
            server_result = &mut server_fut => {
                let err_msg = match server_result {
                    Ok(Ok(())) => "Server exited unexpectedly".to_string(),
                    Ok(Err(e)) => format!("Server exited with error: {e}"),
                    Err(e) => format!("Server task panicked: {e}"),
                };
                anyhow::bail!("{err_msg}. Witness generation aborted.")
            }
        }
    }

    /// Test: server exits with error before witness completes → fast fail.
    /// This simulates the Beacon timeout scenario where the server's channel dies
    /// and the witness generator would otherwise loop on "Channel is closed".
    #[tokio::test]
    async fn test_server_exits_with_error_aborts_witness() {
        // Server exits quickly with an error (simulating preimage timeout → channel close)
        let server = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err("Preimage oracle error: Timeout".to_string())
        });

        // Witness generator would run forever (simulating the "Channel is closed" loop)
        let witness = async {
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                // In real code, this would be printing "Channel is closed" every iteration
            }
            #[allow(unreachable_code)]
            Ok("should never reach".to_string())
        };

        let start = std::time::Instant::now();
        let result = run_with_select(server, witness).await;
        let elapsed = start.elapsed();

        // Should fail fast (< 200ms), NOT hang forever
        assert!(result.is_err());
        assert!(elapsed < Duration::from_millis(200));
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Server exited with error"),
            "Unexpected error: {err_msg}"
        );
        assert!(err_msg.contains("Timeout"));
    }

    /// Test: server exits cleanly (Ok) before witness completes → still fast fail.
    /// Even if the server doesn't return an error, if it exits before the witness
    /// generator finishes, something is wrong.
    #[tokio::test]
    async fn test_server_exits_ok_aborts_witness() {
        let server = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(()) // Server exits without error, but too early
        });

        let witness = async {
            tokio::time::sleep(Duration::from_secs(10)).await; // Would take long
            Ok("witness data".to_string())
        };

        let result = run_with_select(server, witness).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Server exited unexpectedly"));
    }

    /// Test: witness completes before server → normal success path.
    #[tokio::test]
    async fn test_witness_completes_before_server() {
        let server = tokio::spawn(async {
            // Server stays alive until aborted
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        });

        let witness = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok("witness data".to_string())
        };

        let result = run_with_select(server, witness).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "witness data");
    }

    /// Test: witness completes with error before server → error propagated correctly.
    #[tokio::test]
    async fn test_witness_error_propagated() {
        let server = tokio::spawn(async {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(())
        });

        let witness = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Err(anyhow::anyhow!("witness generation failed"))
        };

        let result = run_with_select(server, witness).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("witness generation failed"));
    }

    /// Test: server panics → fast fail with panic info.
    #[tokio::test]
    async fn test_server_panic_aborts_witness() {
        let server = tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            panic!("server crashed");
            #[allow(unreachable_code)]
            Ok(())
        });

        let witness = async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok("should not reach".to_string())
        };

        let result = run_with_select(server, witness).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Server task panicked"));
    }
}
