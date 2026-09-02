//! [MANTLE] Clear the proposer's stuck L1 transactions by replacing them with no-op self-sends.
//!
//! Why this exists as a tool rather than a `cast` one-liner: production signs with GCP Cloud HSM,
//! so there is no private key to hand to `cast`. This reuses the exact signer the proposer uses
//! (`Signer::from_env()`), while setting the gas fields explicitly — the automatic estimation is
//! what produced the unmineable transactions in the first place.
//!
//! Replacing rather than waiting is the right move for `checkpointBlockHash` specifically: it
//! reads `blockhash()`, which only covers the last 256 L1 blocks, so a checkpoint transaction
//! stuck for more than ~51 minutes is guaranteed to revert even if it were mined. Paying to
//! speed those up buys nothing; the proposer will re-checkpoint against a current block.
//!
//! Defaults to a dry run. Nothing is broadcast without `--execute`.
//!
//! Configuration is read from the process environment; `--env-file` is only a convenience for
//! local use and is skipped silently when the file is absent. That is what makes this usable
//! inside a production pod, where `HSM_API_NAME`, `HSM_CREDENTIALS` and `L1_RPC` are already
//! injected as environment variables and `kubectl exec` inherits them — no `.env` file needed.
//!
//! ```bash
//! # local
//! cargo run --release --bin clear-stuck-txs -- --env-file .env
//!
//! # in a pod (env already present); dry run first, then execute
//! kubectl exec -it <pod> -- /usr/local/bin/clear-stuck-txs
//! kubectl exec -it <pod> -- /usr/local/bin/clear-stuck-txs --execute
//! ```
//!
//! Stop the proposer before running with `--execute`, or it will keep issuing transactions and
//! race for the same nonces.
use alloy_eips::BlockId;
use alloy_network::TransactionBuilder;
use alloy_primitives::U256;
use alloy_provider::{Provider, ProviderBuilder};
use alloy_rpc_types_eth::TransactionRequest;
use anyhow::{Context, Result};
use clap::Parser;
use op_succinct_host_utils::logger::setup_logger;
use op_succinct_signer_utils::Signer;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to the environment file holding the signer and RPC configuration.
    #[arg(long, default_value = ".env")]
    env_file: String,

    /// First nonce to replace. Defaults to the account's next mineable nonce.
    #[arg(long)]
    start_nonce: Option<u64>,

    /// Last nonce to replace (inclusive). Defaults to the highest pending nonce.
    #[arg(long)]
    end_nonce: Option<u64>,

    /// maxFeePerGas for the replacements, in gwei.
    ///
    /// Must clear both the current base fee and 110% of each stuck transaction's value for the
    /// replacement to be accepted.
    #[arg(long, default_value_t = 15)]
    max_fee_gwei: u64,

    /// maxPriorityFeePerGas for the replacements, in gwei.
    #[arg(long, default_value_t = 2)]
    priority_fee_gwei: u64,

    /// Seconds to wait for each replacement to confirm before the signer escalates its fees.
    #[arg(long, default_value_t = 120)]
    timeout_secs: u64,

    /// Broadcast. Without this the tool only prints what it would do.
    #[arg(long, default_value_t = false)]
    execute: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Must come before any TLS use. rustls 0.23 refuses to pick a provider when the crate graph
    // carries both `ring` and `aws-lc-rs` (it does here), and panics at the first TLS handshake
    // instead — reached via the GCP KMS client and the L1 RPC. `ring` matches what
    // `validity/bin/validity.rs` installs, so this binary negotiates TLS exactly like the
    // proposer it ships beside.
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|e| anyhow::anyhow!("Failed to install default crypto provider: {e:?}"))?;

    let args = Args::parse();
    dotenv::from_filename(&args.env_file).ok();
    // Without a subscriber the signer's escalation warnings are silently dropped, and an operator
    // watching this run would not see that a transaction had to be re-sent at a higher price.
    setup_logger();

    let l1_rpc: reqwest::Url = std::env::var("L1_RPC")
        .context("L1_RPC must be set")?
        .parse()
        .context("L1_RPC is not a valid URL")?;

    let signer = Signer::from_env().await.context("Failed to build signer from environment")?;
    let address = signer.address();

    let provider = ProviderBuilder::new().connect_http(l1_rpc.clone());

    let latest = provider.get_transaction_count(address).block_id(BlockId::latest()).await?;
    let pending = provider.get_transaction_count(address).block_id(BlockId::pending()).await?;
    let base_fee = provider.get_gas_price().await?;

    println!("signer            : {address}");
    println!("nonce (latest)    : {latest}");
    println!("nonce (pending)   : {pending}");
    println!("stuck transactions: {}", pending.saturating_sub(latest));
    println!("current gas price : {:.4} gwei", base_fee as f64 / 1e9);

    if pending <= latest {
        println!("\nNothing stuck — the account has no queued transactions.");
        return Ok(());
    }

    let start = args.start_nonce.unwrap_or(latest);
    let end = args.end_nonce.unwrap_or(pending - 1);
    anyhow::ensure!(start <= end, "start_nonce {start} is above end_nonce {end}");
    anyhow::ensure!(
        start >= latest,
        "start_nonce {start} is below the next mineable nonce {latest}; those are already mined"
    );

    let max_fee = args.max_fee_gwei as u128 * 1_000_000_000;
    let priority_fee = args.priority_fee_gwei as u128 * 1_000_000_000;
    anyhow::ensure!(priority_fee <= max_fee, "priority fee must not exceed max fee");

    println!(
        "\nPlan: replace nonces {start}..={end} ({} transactions) with 0-value self-sends",
        end - start + 1
    );
    println!("      maxFeePerGas         {} gwei", args.max_fee_gwei);
    println!("      maxPriorityFeePerGas {} gwei", args.priority_fee_gwei);
    println!(
        "      cost ceiling         ~{:.6} ETH",
        (end - start + 1) as f64 * 21_000.0 * args.max_fee_gwei as f64 / 1e9
    );

    if !args.execute {
        println!("\nDry run. Re-run with --execute to broadcast.");
        println!("Stop the proposer first, or it will keep issuing transactions and race for");
        println!("these nonces.");
        return Ok(());
    }

    for nonce in start..=end {
        let request = TransactionRequest::default()
            .with_to(address)
            .with_value(U256::ZERO)
            .with_nonce(nonce)
            .with_gas_limit(21_000)
            .with_max_fee_per_gas(max_fee)
            .with_max_priority_fee_per_gas(priority_fee);

        print!("nonce {nonce}: replacing ... ");
        match signer
            .send_transaction_request_with_timeout(l1_rpc.clone(), request, args.timeout_secs)
            .await
        {
            Ok(receipt) => println!("mined in block {:?}", receipt.block_number),
            // Keep going: a single nonce failing (already mined, or the replacement rejected as
            // underpriced) must not leave the rest of the queue stuck.
            Err(e) => println!("FAILED: {e:#}"),
        }
    }

    let latest_after = provider.get_transaction_count(address).block_id(BlockId::latest()).await?;
    let pending_after =
        provider.get_transaction_count(address).block_id(BlockId::pending()).await?;
    println!("\nnonce (latest) : {latest_after}");
    println!("nonce (pending): {pending_after}");

    // Detect a proposer that was never stopped. Having replaced through `end`, the pending nonce
    // should not exceed end + 1; anything beyond that was queued by someone else while we worked,
    // and the two of us are now competing for nonces. Checked rather than merely documented,
    // because "stop the proposer first" is exactly the step that gets skipped under pressure.
    if pending_after > end + 1 {
        println!();
        println!(
            "WARNING: {} transaction(s) appeared from another sender while this ran — the \
             proposer is probably still running.",
            pending_after - (end + 1)
        );
        println!("Stop it, then re-run this tool; otherwise you will keep racing it for nonces.");
    }
    if pending_after == latest_after {
        println!("Queue is clear. Restart the proposer.");
    } else {
        println!(
            "Still {} queued; re-run to replace the remainder.",
            pending_after.saturating_sub(latest_after)
        );
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(())
}
