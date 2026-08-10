use std::{collections::HashMap, ops::Range, str::FromStr, sync::Arc, time::Duration};

use alloy_eips::BlockId;
use alloy_primitives::{hex, Address, Bytes, B256, U256};
use alloy_provider::{network::ReceiptResponse, Provider};
use alloy_rpc_types_eth::{TransactionReceipt, TransactionRequest};
use alloy_transport::{RpcError, TransportErrorKind};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures_util::{stream, StreamExt, TryStreamExt};
use op_succinct_client_utils::{boot::hash_rollup_config, types::u32_to_u8};
use op_succinct_elfs::AGGREGATION_ELF;
use op_succinct_host_utils::{
    fetcher::OPSuccinctDataFetcher,
    host::OPSuccinctHost,
    metrics::MetricsGauge,
    network::{determine_network_mode, get_network_signer},
    DisputeGameFactory::DisputeGameFactoryInstance as DisputeGameFactoryContract,
    OPSuccinctL2OutputOracle::OPSuccinctL2OutputOracleInstance as OPSuccinctL2OOContract,
};
use op_succinct_proof_utils::{
    cluster_poll_proof, cluster_setup_keys, get_range_elf_embedded, is_cluster_mode,
    reconstruct_proof_request, ClusterProofConfig, ClusterProofHandle, ClusterProofHandleJson,
};
use op_succinct_signer_utils::SignerLock;
use sp1_sdk::{
    network::{
        proto::types::{ExecutionStatus, FulfillmentStatus},
        NetworkMode,
    },
    Elf, HashableKey, NetworkProver, Prover, ProverClient, ProvingKey, SP1Proof,
    SP1ProofWithPublicValues,
};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::{
    db::{DriverDBClient, OPSuccinctRequest, RequestMode, RequestStatus, RequestType},
    find_gaps, get_latest_proposed_block_number, get_ranges_to_prove_by_blocks,
    get_ranges_to_prove_by_gas,
    relay_rejection::{
        classify_revert_data, revert_data_of_anyhow, revert_data_of_rpc_error, NoVerdictReason,
        RelayRejection,
    },
    CommitmentConfig, ContractConfig, OPSuccinctProofRequester, ProgramConfig,
    RequestExecutionStatistics, RequesterConfig, ValidityGauge,
};

/// Number of consecutive poll failures before a cluster proof is marked as permanently failed.
const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 3;

/// Choose the L1 block to checkpoint for an aggregation.
///
/// The aggregation guest walks L1 headers back from the checkpointed head *by hash* and requires
/// every range proof's `l1Head` to lie on that chain, so the checkpoint must be at or after
/// `batch_max_l1_head` (the largest `l1Head` among the aggregated range proofs). A reorg-stable
/// `safe` head is floored at that value:
///
/// - `safe` keeps the checkpoint inside the EVM `blockhash` window and immune to tip reorgs. Under
///   `L1_BLOCK_TAG=finalized|safe`, range `l1Head`s are <= safe, so `safe` is the selected block.
/// - The floor guarantees coverage under `L1_BLOCK_TAG=latest`, where a range `l1Head` can be newer
///   than `safe`. Note the floor is by *number* while the guest enforces by *hash*, so the two
///   agree only on the canonical chain: a boot `l1Head` above `safe` that is later orphaned stays
///   reorg-exposed — inherent to `latest`, not resolved here.
///
/// `None` (no completed range proof has a recorded `l1_head_block_number`) falls back to `safe`.
///
/// [UPSTREAM #923] Backported verbatim from succinctlabs/op-succinct#923 (upstream v3.10.0), which
/// replaced `BlockId::latest()` here. That PR's own description records the failure it fixes as
/// having "Hit 3× on Mantle": the checkpoint head is pinned by hash while the guest's header range
/// is fetched by number, so a tip reorg between the two orphans the checkpoint and the guest
/// rejects the input, wasting one aggregation proof. Kept under the upstream name so a future sync
/// can drop this copy cleanly.
fn select_checkpoint_block_number(safe_block_number: u64, batch_max_l1_head: Option<u64>) -> u64 {
    batch_max_l1_head.map_or(safe_block_number, |max| max.max(safe_block_number))
}

/// The status a classified rejection moves the request to, or `None` to leave it where it is.
///
/// Split out from `handle_relay_rejection` because that function needs a chain, a database and a
/// metrics recorder, so its decision was untestable — and a mutation testing pass found that every
/// single change to it went unnoticed, including flipping the condition that decides whether the
/// request is failed at all. That flip returns the proposer to the incident this whole path exists
/// to prevent: a rejected aggregation left `Complete` keeps counting toward
/// `fetch_active_agg_proofs_count`, so no replacement is ever built and the contract head freezes.
///
/// The return is a concrete status rather than a bool so that changing the target — `Failed` to
/// anything else — also fails the table test.
fn rejection_action(rejection: &RelayRejection) -> Option<RequestStatus> {
    rejection.should_rebuild().then_some(RequestStatus::Failed)
}

/// Value for `AggProofBlockedByContractGuard`, given what [`rejection_action`] decided.
///
/// Leaving the request alone and needing an operator are the same condition: no proof this
/// proposer can build would satisfy the guard that rejected this one. A separate function only so
/// the table test covers the polarity — inverted, this gauge sends an operator to change contract
/// state during a routine rebuild, and to wait out a rebuild while the contract is what is stuck.
fn guard_gauge_value(action: Option<RequestStatus>) -> f64 {
    if action.is_none() {
        1.0
    } else {
        0.0
    }
}

/// A cached checkpoint from a previous aggregation request, plus what the contract says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CachedCheckpoint {
    /// The L1 block hash the database recorded.
    hash: B256,
    /// The L1 block number the database recorded.
    number: u64,
    /// `historicBlockHashes(number)` read back from the contract. `B256::ZERO` means the contract
    /// has no checkpoint at that number.
    onchain_hash: B256,
}

/// Why a cached checkpoint could not be reused. Carried so the log names the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecheckpointReason {
    /// No prior aggregation request to inherit a checkpoint from.
    NoCachedCheckpoint,
    /// The contract has no checkpoint at that block number.
    NotOnChain,
    /// The contract checkpointed a different hash at that number than the database recorded — the
    /// block was reorged out between writing the row and the checkpoint transaction executing.
    HashMismatch,
    /// [UPSTREAM #923] Valid on chain, but below the batch's max `l1Head`.
    BelowBatchMaxL1Head,
}

/// Where a batch's checkpointed L1 block should come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckpointPlan {
    /// The cached checkpoint is still valid on chain and high enough for the guest.
    Reuse { hash: B256, number: u64 },
    /// Nothing usable is cached: read `anchor` and checkpoint it.
    Fresh { anchor: BlockId, reason: RecheckpointReason },
}

/// Decide whether a cached checkpoint can be reused, from what was already read.
///
/// Split out of `create_aggregation_proofs` so every rejection reason — and, critically, the
/// anchor a fresh checkpoint is taken from — is assertable. A mutation pass found that reverting
/// [UPSTREAM #923] by changing that anchor back to `BlockId::latest()` left the whole suite green.
fn checkpoint_plan(
    cached: Option<CachedCheckpoint>,
    batch_max_l1_head: Option<u64>,
) -> CheckpointPlan {
    // [UPSTREAM #923] `safe` rather than `latest`: the header is read by number, so a tip reorg
    // between reading it and the checkpoint transaction executing orphans the checkpoint and wastes
    // one aggregation proof. Observed 3x on Mantle.
    let fresh = |reason| CheckpointPlan::Fresh { anchor: BlockId::safe(), reason };

    let Some(cached) = cached else {
        return fresh(RecheckpointReason::NoCachedCheckpoint);
    };

    if cached.onchain_hash == B256::ZERO {
        fresh(RecheckpointReason::NotOnChain)
    } else if cached.onchain_hash != cached.hash {
        fresh(RecheckpointReason::HashMismatch)
    } else if batch_max_l1_head.is_some_and(|max| cached.number < max) {
        // A matching hash only proves the block was not reorged out — not that the guest can reach
        // every range proof's `l1Head` from it. Reusing a checkpoint below the batch's max would
        // fail the guest's header-chain assertion on every attempt, and because a reused checkpoint
        // is copied verbatim into each rebuilt row, that failure would repeat indefinitely.
        fresh(RecheckpointReason::BelowBatchMaxL1Head)
    } else {
        CheckpointPlan::Reuse { hash: cached.hash, number: cached.number }
    }
}

/// Whether a pass's `submit_agg_proofs` moved the contract's `latestBlockNumber()` forward.
///
/// [MANTLE] `create_aggregation_proofs` reads that head to decide where the next aggregation
/// starts, and `fetch_active_agg_proofs_count` deliberately excludes `Relayed` rows (see
/// `db/client.rs`). So if a relay lands and create then reads a head from an L1 endpoint that is
/// a few blocks behind, the just-relayed range is neither reflected in the head nor counted as
/// active, and the pass builds a second aggregation over it — proven, then rejected on relay, and
/// wasted. Skipping create for one `LOOP_INTERVAL` after a successful relay closes that window.
///
/// A rejection is `No`: it does not advance the head, so create must still run — that is what
/// builds the replacement for the request `handle_relay_rejection` just failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvancedContractHead {
    Yes,
    No,
}

/// What the L1 node did with a `proposeL2Output` transaction, decided from the send result alone.
///
/// Split out of [`Proposer::relay_aggregation_proof`] because that function needs a chain, a
/// signer and a database: without this, the branch that decides whether a failure is
/// classifiable or must be retried unchanged had no test at all.
#[derive(Debug)]
enum SendOutcome {
    /// Mined and succeeded.
    Landed(B256),
    /// Mined and reverted. A receipt carries no revert data, so the reason needs a replay.
    MinedReverted(Box<TransactionReceipt>),
    /// Rejected before broadcast, carrying the revert data the node returned. This is the common
    /// case rather than the exception: alloy's gas filler runs `eth_estimateGas` first, so a
    /// deterministic revert is caught there and the transaction never reaches a block.
    RejectedBeforeBroadcast(Bytes),
    /// Never delivered — nonce, funds, a dead RPC. Nothing to classify; retry unchanged.
    Undelivered(anyhow::Error),
}

/// See [`SendOutcome`].
fn send_outcome(result: Result<TransactionReceipt>) -> SendOutcome {
    match result {
        Ok(receipt) if receipt.status() => SendOutcome::Landed(receipt.transaction_hash()),
        Ok(receipt) => SendOutcome::MinedReverted(Box::new(receipt)),
        Err(e) => match revert_data_of_anyhow(&e) {
            Some(data) => SendOutcome::RejectedBeforeBroadcast(data),
            None => SendOutcome::Undelivered(e),
        },
    }
}

/// The outcome of a bounded `eth_call` replay: the inner `Result` is the call, the outer is the
/// timeout around it. Spelled out because `Result` in this module is `anyhow`'s.
type ReplayResult = std::result::Result<
    std::result::Result<Bytes, RpcError<TransportErrorKind>>,
    tokio::time::error::Elapsed,
>;

/// Read a rejection out of a replayed `eth_call`. See [`Proposer::classify_by_replay`].
///
/// Split out for the same reason as [`send_outcome`]: every arm here is a distinct operator-facing
/// diagnosis, and none of them were reachable from a test while this lived inside the `await`.
fn replay_verdict(replay: ReplayResult) -> RelayRejection {
    match replay {
        Ok(Err(e)) => match revert_data_of_rpc_error(&e) {
            Some(data) => classify_revert_data(&data),
            None => RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayUnreachable },
        },
        Ok(Ok(_)) => RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayDidNotRevert },
        Err(_) => RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayTimedOut },
    }
}

/// Whether an aggregation proof landed on chain, or was refused and why.
///
/// A refusal is an outcome rather than an error: `Err` from `relay_aggregation_proof` is reserved
/// for a transaction that was never delivered, which is the only case where retrying unchanged is
/// the right response.
#[derive(Debug)]
enum RelayOutcome {
    Relayed(B256),
    Rejected(RelayRejection),
}

/// Marker the self-hosted proof-router/gateway attaches to an admission-shed
/// gRPC `Status` (trailer + message prefix) when its concurrency pool is full.
/// The Succinct proving network never returns this, so keying on it leaves the
/// Succinct path completely unchanged. Kept in sync with the gateway/router.
const ADMISSION_SHED_MARKER: &str = "x-sp1-admission-shed";

/// Whether a failed proof-request task was a self-hosted admission shed (the
/// prover pool was momentarily full) rather than a genuine proof/range failure.
/// A shed is transient: the request must be retried as-is, NOT marked Failed —
/// marking it Failed counts toward range bisection and needlessly fragments the
/// range (the prover is just busy, not the range too big). The marker travels
/// in the tonic `Status` message + metadata, so it appears in the error's debug
/// rendering regardless of any anyhow wrapping.
fn is_admission_shed_error(e: &anyhow::Error) -> bool {
    format!("{e:?}").contains(ADMISSION_SHED_MARKER)
}

/// Whether a failed proof-request task failed because the prover backend was
/// unreachable / transiently unavailable (a transport or connectivity fault)
/// rather than because the proof itself failed. gRPC maps every transport /
/// connectivity fault — a dead backend, a reset connection, a DNS failure, a
/// "tcp connect error", or the router reporting all backends unavailable — to
/// `UNAVAILABLE`, which is a retryable transient condition.
///
/// Such a failure must be retried as-is, NOT marked Failed: marking it Failed
/// feeds range bisection, which needlessly fragments a range that is perfectly
/// fine — the backend was simply unreachable. A range only needs bisecting when
/// the proof fails *deterministically* (execution unexecutable / too big), which
/// surfaces as a different, NON-transport error — the sp1-sdk `network::Error`
/// enum, never a `tonic::Status` — so it is not matched here. This holds for both
/// the self-hosted and Succinct paths.
///
/// Primary signal is the TYPED gRPC code: the sp1-sdk surfaces RPC failures as an
/// `anyhow::Error` carrying a downcastable `tonic::Status` (its own `retry.rs`
/// classifies the identical way), so `code() == Unavailable` catches every
/// transport fault robustly, is immune to `Status` Display-format drift across
/// tonic versions, and can't be tripped by an unrelated error that merely
/// mentions the text. The string fallback runs only when the error is NOT a
/// downcastable `Status` of our tonic version (a future SDK tonic bump that
/// de-unifies the type, or a pre-`Status` connect failure), so a plain
/// "tcp connect error" still never bisects.
fn is_transient_transport_error(e: &anyhow::Error) -> bool {
    if let Some(status) = e.downcast_ref::<tonic::Status>() {
        return status.code() == tonic::Code::Unavailable;
    }
    let rendered = format!("{e:?}");
    rendered.contains("status: Unavailable") ||
        rendered.contains("tcp connect error") ||
        rendered.contains("error trying to connect")
}

/// Configuration for the driver.
pub struct DriverConfig {
    pub network_prover: Option<Arc<NetworkProver>>,
    pub fetcher: Arc<OPSuccinctDataFetcher>,
    pub driver_db_client: Arc<DriverDBClient>,
    pub signer: SignerLock,
    pub loop_interval: u64,
}
/// Type alias for a map of task IDs to their join handles and associated requests
pub type TaskMap = HashMap<i64, (tokio::task::JoinHandle<Result<()>>, OPSuccinctRequest)>;

pub struct Proposer<P, H: OPSuccinctHost>
where
    P: Provider + 'static,
{
    driver_config: DriverConfig,
    contract_config: ContractConfig<P>,
    program_config: ProgramConfig,
    requester_config: RequesterConfig,
    proof_requester: Arc<OPSuccinctProofRequester<H>>,
    tasks: Arc<Mutex<TaskMap>>,
}

impl<P, H: OPSuccinctHost> Proposer<P, H>
where
    P: Provider + 'static + Clone,
{
    pub async fn new(
        provider: P,
        db_client: Arc<DriverDBClient>,
        fetcher: Arc<OPSuccinctDataFetcher>,
        requester_config: RequesterConfig,
        signer: SignerLock,
        loop_interval: u64,
        host: Arc<H>,
    ) -> Result<Self> {
        // This check prevents users from running multiple proposers for the same chain at the same
        // time.
        let is_locked = db_client
            .is_chain_locked(
                requester_config.l1_chain_id,
                requester_config.l2_chain_id,
                Duration::from_secs(loop_interval),
            )
            .await?;
        if is_locked {
            return Err(anyhow!(
                "There is another proposer for the same chain connected to the database. Only one proposer can be connected to the database for a chain at a time."
            ));
        }

        // Add the chain lock to the database.
        db_client
            .add_chain_lock(requester_config.l1_chain_id, requester_config.l2_chain_id)
            .await?;

        let is_cluster = is_cluster_mode();

        let cluster_config =
            if is_cluster { Some(Arc::new(ClusterProofConfig::from_env().await?)) } else { None };
        let cluster_handles: Arc<Mutex<HashMap<i64, ClusterProofHandle>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let (range_pk, range_vk, agg_pk, agg_vk, network_prover) = if is_cluster {
            let (range_pk, range_vk, agg_pk, agg_vk) = cluster_setup_keys().await?;
            (range_pk, range_vk, agg_pk, agg_vk, None)
        } else {
            let network_signer = get_network_signer(requester_config.use_kms_requester).await?;
            let network_mode = determine_network_mode(
                requester_config.range_proof_strategy,
                requester_config.agg_proof_strategy,
            )?;
            let network_prover = Arc::new(
                ProverClient::builder()
                    .network_for(network_mode)
                    .signer(network_signer)
                    .build()
                    .await,
            );
            let range_pk = network_prover.setup(Elf::Static(get_range_elf_embedded())).await?;
            let range_vk = range_pk.verifying_key().clone();
            let agg_pk = network_prover.setup(Elf::Static(AGGREGATION_ELF)).await?;
            let agg_vk = agg_pk.verifying_key().clone();
            (range_pk, range_vk, agg_pk, agg_vk, Some(network_prover))
        };

        let range_vkey_commitment = B256::from(u32_to_u8(range_vk.vk.hash_u32()));
        let agg_vkey_hash = B256::from_str(&agg_vk.bytes32())?;
        let rollup_config_hash = hash_rollup_config(
            fetcher
                .rollup_config
                .as_ref()
                .ok_or_else(|| anyhow!("Rollup config must be set to initialize the proposer."))?,
        );

        let program_config = ProgramConfig {
            range_vk: Arc::new(range_vk),
            range_pk: Arc::new(range_pk),
            agg_vk: Arc::new(agg_vk),
            agg_pk: Arc::new(agg_pk),
            commitments: CommitmentConfig {
                range_vkey_commitment,
                agg_vkey_hash,
                rollup_config_hash,
            },
        };
        program_config.log();

        let proof_requester = Arc::new(OPSuccinctProofRequester::new(
            host,
            network_prover.clone(),
            fetcher.clone(),
            db_client.clone(),
            program_config.clone(),
            requester_config.mock,
            is_cluster,
            cluster_config,
            cluster_handles,
            requester_config.range_proof_strategy,
            requester_config.agg_proof_strategy,
            requester_config.agg_proof_mode,
            requester_config.safe_db_fallback,
            requester_config.max_price_per_pgu,
            requester_config.proving_timeout,
            requester_config.witnessgen_timeout,
            requester_config.range_cycle_limit,
            requester_config.range_gas_limit,
            requester_config.agg_cycle_limit,
            requester_config.agg_gas_limit,
            requester_config.whitelist.clone(),
            requester_config.min_auction_period,
            requester_config.auction_timeout,
        )?);

        let l2oo_contract =
            OPSuccinctL2OOContract::new(requester_config.l2oo_address, provider.clone());

        let dgf_contract =
            DisputeGameFactoryContract::new(requester_config.dgf_address, provider.clone());

        let proposer = Proposer {
            driver_config: DriverConfig {
                network_prover,
                fetcher,
                driver_db_client: db_client,
                signer,
                loop_interval,
            },
            contract_config: ContractConfig {
                l2oo_address: requester_config.l2oo_address,
                dgf_address: requester_config.dgf_address,
                l2oo_contract,
                dgf_contract,
            },
            program_config,
            requester_config,
            proof_requester,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        };
        Ok(proposer)
    }

    /// Use the in-memory index of the highest block number to add new ranges to the database.
    #[tracing::instrument(name = "proposer.add_new_ranges", skip(self))]
    pub async fn add_new_ranges(&self) -> Result<()> {
        // Get the latest proposed block number on the contract.
        let latest_proposed_block_number = get_latest_proposed_block_number(
            self.contract_config.l2oo_address,
            self.driver_config.fetcher.as_ref(),
        )
        .await?;

        let finalized_block_number = match self
            .proof_requester
            .host
            .get_finalized_l2_block_number(
                self.driver_config.fetcher.as_ref(),
                latest_proposed_block_number,
            )
            .await?
        {
            Some(block_number) => {
                tracing::debug!("Found finalized block number: {}", block_number);
                block_number
            }
            None => {
                tracing::debug!("No new finalized block number found since last proposed block. No new range proof requests will be added.");
                return Ok(());
            }
        };

        // Get all active (non-failed) requests with the same commitment config and start block >=
        // latest_proposed_block_number. These requests are non-overlapping.
        let mut requests = self
            .driver_config
            .driver_db_client
            .fetch_ranges_after_block(
                &[
                    RequestStatus::Unrequested,
                    RequestStatus::WitnessGeneration,
                    RequestStatus::Execution,
                    RequestStatus::Prove,
                    RequestStatus::Complete,
                ],
                latest_proposed_block_number as i64,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        // Sort the requests by start block.
        requests.sort_by_key(|r| r.0);

        let disjoint_ranges = find_gaps(
            latest_proposed_block_number as i64,
            finalized_block_number as i64,
            &requests,
        );

        let ranges_to_prove = if self.requester_config.evm_gas_limit > 0 {
            // Use gas-based splitting
            let mut all_block_infos = std::collections::HashMap::new();
            for &Range { start, end } in &disjoint_ranges {
                if start < end {
                    let block_data = self
                        .driver_config
                        .fetcher
                        .get_l2_block_data_range(start as u64, end as u64)
                        .await?;

                    for block_info in block_data {
                        all_block_infos.insert(block_info.block_number as i64, block_info);
                    }
                }
            }

            get_ranges_to_prove_by_gas(
                &disjoint_ranges,
                self.requester_config.evm_gas_limit,
                self.requester_config.range_proof_interval as i64,
                &all_block_infos,
            )?
        } else {
            // Use block-based splitting
            get_ranges_to_prove_by_blocks(
                &disjoint_ranges,
                self.requester_config.range_proof_interval as i64,
            )
        };

        if ranges_to_prove.is_empty() {
            warn!("No range proof requests inserted into the database.")
        } else {
            info!("Inserting {} range proof requests into the database.", ranges_to_prove.len());

            // Create range proof requests for the ranges to prove in parallel
            let new_range_requests = stream::iter(ranges_to_prove)
                .map(|range| {
                    let mode = if self.requester_config.mock {
                        RequestMode::Mock
                    } else {
                        RequestMode::Real
                    };
                    OPSuccinctRequest::create_range_request(
                        mode,
                        range.start,
                        range.end,
                        self.program_config.commitments.range_vkey_commitment,
                        self.program_config.commitments.rollup_config_hash,
                        self.requester_config.l1_chain_id,
                        self.requester_config.l2_chain_id,
                        self.driver_config.fetcher.clone(),
                    )
                })
                .buffered(10) // Do 10 at a time, otherwise it's too slow when fetching the block range data.
                .try_collect::<Vec<OPSuccinctRequest>>()
                .await?;

            // Insert the new range proof requests into the database.
            self.driver_config.driver_db_client.insert_requests(&new_range_requests).await?;

            // Log details for each created range proof request.
            for request in &new_range_requests {
                debug!(
                    start_block = request.start_block,
                    end_block = request.end_block,
                    "Range proof request created and inserted into database"
                );
            }
        }

        Ok(())
    }

    /// Handle all proof requests in the Prove state.
    ///
    /// No-op in mock mode (proofs are generated synchronously).
    /// In cluster mode, polls each request via `process_cluster_proof_status`.
    /// In network mode, polls each request via `process_proof_request_status`.
    #[tracing::instrument(name = "proposer.handle_proving_requests", skip(self))]
    pub async fn handle_proving_requests(&self) -> Result<()> {
        if self.proof_requester.is_synchronous_proving() {
            return Ok(());
        }

        let prove_requests = self
            .driver_config
            .driver_db_client
            .fetch_requests_by_status(
                RequestStatus::Prove,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        for request in prove_requests {
            if self.proof_requester.cluster {
                // Cluster mode: catch errors per-request so a single failed poll doesn't
                // abort processing of remaining Prove requests.
                if let Err(e) = self.process_cluster_proof_status(request).await {
                    warn!(error = ?e, "Error processing cluster proof status");
                }
            } else {
                self.process_proof_request_status(request).await?;
            }
        }

        Ok(())
    }

    /// Process a single OP Succinct request's proof status.
    #[tracing::instrument(name = "proposer.process_proof_request_status", skip(self, request))]
    pub async fn process_proof_request_status(&self, request: OPSuccinctRequest) -> Result<()> {
        let network_prover = self
            .driver_config
            .network_prover
            .as_ref()
            .context("network_prover required for proof status polling")?;

        if let Some(proof_request_id) = request.proof_request_id.as_ref() {
            let proof_request_id = B256::from_slice(proof_request_id);
            let (status, proof) = self
                .network_call_with_timeout(
                    network_prover.get_proof_status(proof_request_id),
                    "waiting for proof status",
                    &request,
                )
                .await?;

            let request_details = self
                .network_call_with_timeout(
                    network_prover.get_proof_request(proof_request_id),
                    "waiting for proof request details",
                    &request,
                )
                .await?;

            // Check if current time exceeds deadline. If so, the proof has timed out.
            let current_time =
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();

            // Cancel the request in the network if the auction timeout is exceeded.
            if let Some(request_details) = request_details {
                let auction_deadline =
                    request_details.created_at + self.requester_config.auction_timeout;
                if network_prover.network_mode() == NetworkMode::Mainnet &&
                    request_details.fulfillment_status == FulfillmentStatus::Requested as i32 &&
                    current_time > auction_deadline
                {
                    // Cancel the request in the network.
                    self.network_call_with_timeout(
                        network_prover.cancel_request(proof_request_id),
                        "cancelling proof request",
                        &request,
                    )
                    .await?;

                    // Mark the request as cancelled in the database.
                    match self.proof_requester.handle_cancelled_request(request.clone()).await {
                        Ok(_) => ValidityGauge::ProofRequestRetryCount.increment(1.0),
                        Err(e) => {
                            ValidityGauge::RetryErrorCount.increment(1.0);
                            return Err(e);
                        }
                    }

                    ValidityGauge::ProofRequestTimeoutErrorCount.increment(1.0);

                    warn!(
                        proof_id = request.id,
                        start_block = request.start_block,
                        end_block = request.end_block,
                        req_type = ?request.req_type,
                        auction_deadline,
                        current_time,
                        "Proof request auction deadline exceeded"
                    );

                    return Ok(());
                }
            }

            if current_time > status.deadline() {
                match self
                    .proof_requester
                    .handle_failed_request(request.clone(), status.execution_status())
                    .await
                {
                    Ok(_) => ValidityGauge::ProofRequestRetryCount.increment(1.0),
                    Err(e) => {
                        ValidityGauge::RetryErrorCount.increment(1.0);
                        return Err(e);
                    }
                }

                ValidityGauge::ProofRequestTimeoutErrorCount.increment(1.0);

                warn!(
                    proof_id = request.id,
                    start_block = request.start_block,
                    end_block = request.end_block,
                    req_type = ?request.req_type,
                    deadline = status.deadline(),
                    current_time,
                    "Proof request timed out"
                );

                return Ok(());
            }

            // If the proof request has been fulfilled, update the request to status Complete and
            // add the proof bytes to the database.
            if status.fulfillment_status() == FulfillmentStatus::Fulfilled as i32 {
                let proof: SP1ProofWithPublicValues = proof.ok_or_else(|| {
                    anyhow!(
                        "Network reported Fulfilled but returned no proof for request {}",
                        request.id
                    )
                })?;

                let proof_bytes = match proof.proof {
                    // If it's a compressed proof, serialize with bincode.
                    SP1Proof::Compressed(_) => bincode::serialize(&proof)?,
                    // If it's Groth16 or PLONK, get the on-chain proof bytes.
                    SP1Proof::Groth16(_) | SP1Proof::Plonk(_) => proof.bytes(),
                    SP1Proof::Core(_) => return Err(anyhow!("Core proofs are not supported.")),
                };

                // Add the completed proof to the database.
                self.driver_config
                    .driver_db_client
                    .update_proof_to_complete(request.id, &proof_bytes)
                    .await?;
                // Update the prove_duration based on the current time and the proof_request_time.
                self.driver_config.driver_db_client.update_prove_duration(request.id).await?;

                if let Some(proof_request) = self
                    .network_call_with_timeout(
                        network_prover.get_proof_request(proof_request_id),
                        "fetching execution statistics",
                        &request,
                    )
                    .await?
                {
                    let execution_statistics = RequestExecutionStatistics::from(&proof_request);

                    // Write the execution data to the database.
                    self.driver_config
                        .driver_db_client
                        .insert_execution_statistics(
                            request.id,
                            serde_json::to_value(execution_statistics)?,
                            0,
                        )
                        .await?;
                }

                // Log completion of range and aggregation proofs.
                match request.req_type {
                    RequestType::Range => {
                        info!(
                            proof_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            proof_request_time = ?request.proof_request_time,
                            total_tx_fees = %request.total_tx_fees,
                            total_transactions = request.total_nb_transactions,
                            witnessgen_duration_s = request.witnessgen_duration,
                            prove_duration_s = request.prove_duration,
                            total_eth_gas_used = request.total_eth_gas_used,
                            total_l1_fees = %request.total_l1_fees,
                            "Range proof completed successfully"
                        );
                    }
                    RequestType::Aggregation => {
                        info!(
                            proof_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            witnessgen_duration_s = request.witnessgen_duration,
                            prove_duration_s = request.prove_duration,
                            "Aggregation proof completed successfully"
                        );
                    }
                }
            } else if status.fulfillment_status() == FulfillmentStatus::Unfulfillable as i32 {
                // Log failure of range and aggregation proofs.
                match request.req_type {
                    RequestType::Range => {
                        warn!(
                            proof_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            proof_request_time = ?request.proof_request_time,
                            total_tx_fees = %request.total_tx_fees,
                            total_transactions = request.total_nb_transactions,
                            witnessgen_duration_s = request.witnessgen_duration,
                            total_eth_gas_used = request.total_eth_gas_used,
                            total_l1_fees = %request.total_l1_fees,
                            execution_status = ?status.execution_status(),
                            "Range proof request failed - unfulfillable"
                        );
                    }
                    RequestType::Aggregation => {
                        warn!(
                            proof_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            witnessgen_duration_s = request.witnessgen_duration,
                            execution_status = ?status.execution_status(),
                            "Aggregation proof request failed - unfulfillable"
                        );
                    }
                }

                self.proof_requester
                    .handle_failed_request(request, status.execution_status())
                    .await?;
                ValidityGauge::ProofRequestRetryCount.increment(1.0);
            }
        } else {
            // There should never be a proof request in Prove status without a proof request id.
            tracing::warn!(id = request.id, start_block = request.start_block, end_block = request.end_block, req_type = ?request.req_type, "Request has no proof request id");
        }

        Ok(())
    }

    /// Fail a cluster proof request: increment the appropriate error gauge, mark
    /// the request as failed (with potential split-retry), and remove the in-memory handle.
    async fn fail_cluster_request(&self, request: &OPSuccinctRequest) -> Result<()> {
        // Remove the in-memory handle first so it doesn't leak if the DB update below fails.
        self.proof_requester.cluster_handles.lock().await.remove(&request.id);

        match request.req_type {
            RequestType::Range => ValidityGauge::RangeProofRequestErrorCount.increment(1.0),
            RequestType::Aggregation => ValidityGauge::AggProofRequestErrorCount.increment(1.0),
        }

        match self
            .proof_requester
            .handle_failed_request(
                request.clone(),
                ExecutionStatus::UnspecifiedExecutionStatus as i32,
            )
            .await
        {
            Ok(_) => ValidityGauge::ProofRequestRetryCount.increment(1.0),
            Err(e) => {
                ValidityGauge::RetryErrorCount.increment(1.0);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Process a single cluster proof request's status by polling the cluster.
    ///
    /// Handles:
    /// - Handle lookup from in-memory map, or reconstruction from DB on restart
    /// - Wall-clock timeout via `proof_request_time`
    /// - Proof completion (serialize and store)
    /// - Transient vs permanent poll errors (3 consecutive failures = permanent)
    #[tracing::instrument(name = "proposer.process_cluster_proof_status", skip(self, request))]
    async fn process_cluster_proof_status(&self, request: OPSuccinctRequest) -> Result<()> {
        let cluster_config = self
            .proof_requester
            .cluster_config
            .as_ref()
            .context("cluster_config required for cluster proof polling")?;

        // 1. Wall-clock timeout check — runs before handle lookup/reconstruction so we skip
        //    unnecessary deserialization and lock acquisition for already-timed-out proofs.
        if let Some(proof_request_time) = request.proof_request_time {
            let elapsed = Utc::now().naive_utc() - proof_request_time;
            if elapsed.num_seconds().max(0) > self.proof_requester.proving_timeout as i64 {
                warn!(
                    request_id = request.id,
                    start_block = request.start_block,
                    end_block = request.end_block,
                    req_type = ?request.req_type,
                    elapsed_secs = elapsed.num_seconds(),
                    proving_timeout = self.proof_requester.proving_timeout,
                    "Cluster proof exceeded wall-clock timeout"
                );

                self.fail_cluster_request(&request).await?;
                ValidityGauge::ProofRequestTimeoutErrorCount.increment(1.0);

                return Ok(());
            }
        }

        // 2. Look up or reconstruct the proof handle. Lock is acquired narrowly: once for the
        //    lookup, and (on miss) once more for insert.
        let proof_request = {
            let cached = self
                .proof_requester
                .cluster_handles
                .lock()
                .await
                .get(&request.id)
                .map(|h| h.proof_request.clone());

            if let Some(pr) = cached {
                pr
            } else {
                // Reconstruct from DB (restart recovery).
                let handle_json_value = request.cluster_proof_handle.as_ref().ok_or_else(|| {
                    anyhow!(
                        "Cluster proof request {} in Prove status has no cluster_proof_handle",
                        request.id
                    )
                })?;
                let handle_json: ClusterProofHandleJson =
                    serde_json::from_value(handle_json_value.clone())?;

                let proof_request_time = request.proof_request_time.ok_or_else(|| {
                    anyhow!(
                        "Cluster proof request {} in Prove status has no proof_request_time",
                        request.id
                    )
                })?;

                let elapsed = Utc::now().naive_utc() - proof_request_time;
                let total_timeout = Duration::from_secs(self.proof_requester.proving_timeout);
                let remaining = total_timeout
                    .checked_sub(Duration::from_secs(elapsed.num_seconds().max(0) as u64))
                    .unwrap_or(Duration::ZERO);

                let proof_request = reconstruct_proof_request(&handle_json, remaining);

                self.proof_requester.cluster_handles.lock().await.insert(
                    request.id,
                    ClusterProofHandle {
                        proof_request: proof_request.clone(),
                        consecutive_poll_failures: 0,
                    },
                );

                info!(
                    request_id = request.id,
                    start_block = request.start_block,
                    end_block = request.end_block,
                    remaining_timeout_secs = remaining.as_secs(),
                    "Reconstructed cluster proof handle from DB"
                );

                proof_request
            }
        };

        // 3. Poll the cluster for proof status.
        match cluster_poll_proof(cluster_config, proof_request).await {
            Ok(Some(results)) => {
                // Proof complete — convert and store.
                let proof = SP1ProofWithPublicValues::from(results.proof);

                let proof_bytes = match proof.proof {
                    SP1Proof::Compressed(_) => bincode::serialize(&proof)?,
                    SP1Proof::Groth16(_) | SP1Proof::Plonk(_) => proof.bytes(),
                    SP1Proof::Core(_) => return Err(anyhow!("Core proofs are not supported.")),
                };

                self.driver_config
                    .driver_db_client
                    .update_proof_to_complete(request.id, &proof_bytes)
                    .await?;
                self.driver_config.driver_db_client.update_prove_duration(request.id).await?;

                let prove_duration_s = request
                    .proof_request_time
                    .map(|t| (Utc::now().naive_utc() - t).num_seconds())
                    .unwrap_or(0);

                match request.req_type {
                    RequestType::Range => {
                        info!(
                            request_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            prove_duration_s,
                            total_tx_fees = %request.total_tx_fees,
                            total_transactions = request.total_nb_transactions,
                            witnessgen_duration_s = request.witnessgen_duration,
                            total_eth_gas_used = request.total_eth_gas_used,
                            total_l1_fees = %request.total_l1_fees,
                            "Range proof completed via cluster"
                        );
                    }
                    RequestType::Aggregation => {
                        info!(
                            request_id = request.id,
                            start_block = request.start_block,
                            end_block = request.end_block,
                            prove_duration_s,
                            witnessgen_duration_s = request.witnessgen_duration,
                            "Aggregation proof completed via cluster"
                        );
                    }
                }

                self.proof_requester.cluster_handles.lock().await.remove(&request.id);
            }
            Ok(None) => {
                // Still pending — reset failure counter.
                let mut handles = self.proof_requester.cluster_handles.lock().await;
                if let Some(handle) = handles.get_mut(&request.id) {
                    handle.consecutive_poll_failures = 0;
                }
            }
            Err(e) => {
                // Poll error — distinguish transient vs permanent.
                // Acquire the lock once and both increment + optionally remove in the same scope.
                let should_fail = {
                    let mut handles = self.proof_requester.cluster_handles.lock().await;
                    if let Some(handle) = handles.get_mut(&request.id) {
                        handle.consecutive_poll_failures += 1;
                        handle.consecutive_poll_failures >= MAX_CONSECUTIVE_POLL_FAILURES
                    } else {
                        true
                    }
                };

                if should_fail {
                    warn!(
                        request_id = request.id,
                        start_block = request.start_block,
                        end_block = request.end_block,
                        req_type = ?request.req_type,
                        error = %e,
                        "Cluster proof poll failed permanently"
                    );

                    self.fail_cluster_request(&request).await?;
                } else {
                    warn!(
                        request_id = request.id,
                        start_block = request.start_block,
                        end_block = request.end_block,
                        req_type = ?request.req_type,
                        error = %e,
                        "Cluster proof poll failed transiently, will retry next iteration"
                    );
                }
            }
        }

        Ok(())
    }

    /// Create aggregation proofs based on the completed range proofs. The range proofs must be
    /// contiguous and have the same range vkey commitment. Assumes that the range proof retry
    /// logic guarantees that there is not two potential contiguous chains of range proofs.
    ///
    /// Only creates an Aggregation proof if there's not an Aggregation proof in progress with the
    /// same start block.
    #[tracing::instrument(name = "proposer.create_aggregation_proofs", skip(self))]
    pub async fn create_aggregation_proofs(&self) -> Result<()> {
        // Check if there's an Aggregation proof with the same start block AND range verification
        // key commitment AND aggregation vkey. If so, return.
        let latest_proposed_block_number = get_latest_proposed_block_number(
            self.contract_config.l2oo_address,
            self.driver_config.fetcher.as_ref(),
        )
        .await? as i64;

        // Get all active Aggregation proofs with the same start block, range vkey commitment, and
        // aggregation vkey.
        let active_agg_proofs_count = self
            .driver_config
            .driver_db_client
            .fetch_active_agg_proofs_count(
                latest_proposed_block_number as i64,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        if active_agg_proofs_count > 0 {
            tracing::debug!("There is already an Aggregation proof queued with the same start block, range vkey commitment, and aggregation vkey.");
            return Ok(());
        }

        // Get the completed range proofs with a start block greater than the latest proposed block
        // number. These blocks are sorted.
        let completed_range_proofs = self
            .driver_config
            .driver_db_client
            .fetch_completed_ranges(
                &self.program_config.commitments,
                latest_proposed_block_number as i64,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        // Get the highest block number of the completed range proofs.
        let highest_proven_contiguous_block_number = match self
            .get_highest_proven_contiguous_block(completed_range_proofs)?
        {
            Some(block) => block,
            None => return Ok(()), /* No completed range proofs contiguous to the latest proposed
                                    * block number, so no need to create an aggregation proof. */
        };

        // Get the submission interval from the contract.
        let contract_submission_interval: u64 =
            self.contract_config.l2oo_contract.submissionInterval().call().await?.to::<u64>();

        // Use the submission interval from the contract if it's greater than the one in the
        // proposer config.
        let submission_interval =
            contract_submission_interval.max(self.requester_config.submission_interval) as i64;

        debug!("Submission interval for aggregation proof: {}.", submission_interval);

        // If the highest proven contiguous block number is greater than the latest proposed block
        // number plus the submission interval, create an aggregation proof.
        if (highest_proven_contiguous_block_number - latest_proposed_block_number) >=
            submission_interval
        {
            // If an aggregation request with the same start block and end block and commitment
            // config exists, there's no need to checkpoint the L1 block hash.
            // Use the existing L1 block hash from the existing request.
            let existing_request = self
                .driver_config
                .driver_db_client
                .fetch_failed_agg_request_with_checkpointed_block_hash(
                    latest_proposed_block_number,
                    highest_proven_contiguous_block_number,
                    &self.program_config.commitments,
                    self.requester_config.l1_chain_id,
                    self.requester_config.l2_chain_id,
                )
                .await?;

            // [UPSTREAM #923] The L1 head the guest must be able to walk back to: the largest
            // `l1Head` among the range proofs this aggregation will consume. Gates both the reuse
            // path below and the fresh checkpoint.
            let batch_max_l1_head = self
                .driver_config
                .driver_db_client
                .get_max_l1_head_block_number_for_range(
                    latest_proposed_block_number,
                    highest_proven_contiguous_block_number,
                    &self.program_config.commitments,
                    self.requester_config.l1_chain_id,
                    self.requester_config.l2_chain_id,
                )
                .await?
                .map(u64::try_from)
                .transpose()
                .context("Range proof l1_head_block_number is negative")?;

            // If there's an existing aggregation request with the same start block, end block, and
            // commitment config, try to reuse its checkpoint as long as it still matches the
            // on-chain mapping.
            let cached = match existing_request {
                Some(existing_request) => {
                    let number = u64::try_from(existing_request.1)
                        .context("Existing checkpointed L1 block number is negative")?;
                    let onchain_hash = self
                        .contract_config
                        .l2oo_contract
                        .historicBlockHashes(U256::from(number))
                        .call()
                        .await?
                        .0;
                    Some(CachedCheckpoint {
                        hash: B256::from_slice(&existing_request.0),
                        number,
                        onchain_hash: onchain_hash.into(),
                    })
                }
                None => None,
            };

            let (checkpointed_l1_block_hash, checkpointed_l1_block_number) =
                match checkpoint_plan(cached, batch_max_l1_head) {
                    CheckpointPlan::Reuse { hash, number } => {
                        debug!(
                            block_number = number,
                            ?hash,
                            "Reusing cached checkpointed L1 block hash."
                        );
                        (hash, number as i64)
                    }
                    CheckpointPlan::Fresh { anchor, reason } => {
                        warn!(
                            ?reason,
                            ?cached,
                            ?batch_max_l1_head,
                            "No reusable checkpoint; taking a fresh one."
                        );

                        // [UPSTREAM #923] Floor the reorg-stable anchor at the batch's max l1Head
                        // so the aggregation guest's header walk covers
                        // every range proof. See
                        // `select_checkpoint_block_number`.
                        let anchor_header =
                            self.driver_config.fetcher.get_l1_header(anchor).await?;

                        let checkpoint_number =
                            select_checkpoint_block_number(anchor_header.number, batch_max_l1_head);

                        let checkpoint_header = if checkpoint_number == anchor_header.number {
                            anchor_header
                        } else {
                            self.driver_config
                                .fetcher
                                .get_l1_header(checkpoint_number.into())
                                .await?
                        };

                        // Checkpoint the L1 block hash.
                        let transaction_request = self
                            .contract_config
                            .l2oo_contract
                            .checkpointBlockHash(U256::from(checkpoint_header.number))
                            .into_transaction_request();

                        let receipt = self
                            .driver_config
                            .signer
                            .send_transaction_request_with_timeout(
                                self.driver_config.fetcher.as_ref().rpc_config.l1_rpc.clone(),
                                transaction_request,
                                self.requester_config.tx_confirmation_timeout,
                            )
                            .await?;

                        // If transaction reverted, log the error.
                        if !receipt.status() {
                            return Err(anyhow!(
                                "Checkpoint block transaction reverted: {:?}",
                                receipt
                            ));
                        }

                        info!(
                            block_number = checkpoint_header.number,
                            "Checkpointed a fresh L1 block hash."
                        );

                        (checkpoint_header.hash_slow(), checkpoint_header.number as i64)
                    }
                };

            // Create an aggregation proof request to cover the range with the checkpointed L1 block
            // hash.
            let agg_request = OPSuccinctRequest::new_agg_request(
                if self.requester_config.mock { RequestMode::Mock } else { RequestMode::Real },
                latest_proposed_block_number,
                highest_proven_contiguous_block_number,
                self.program_config.commitments.range_vkey_commitment,
                self.program_config.commitments.agg_vkey_hash,
                self.program_config.commitments.rollup_config_hash,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
                checkpointed_l1_block_number,
                checkpointed_l1_block_hash,
                self.driver_config.signer.address(),
            );

            self.driver_config.driver_db_client.insert_request(&agg_request).await?;

            info!(
                start_block = agg_request.start_block,
                end_block = agg_request.end_block,
                "Aggregation proof request created and inserted into database"
            );
        }

        Ok(())
    }

    /// Request all unrequested proofs up to MAX_CONCURRENT_PROOF_REQUESTS. If there are already
    /// MAX_CONCURRENT_PROOF_REQUESTS proofs in WitnessGeneration, Execute, and Prove status,
    /// return. If there are already MAX_CONCURRENT_WITNESS_GEN proofs in WitnessGeneration or
    /// Execute status, return.
    ///
    /// Note: In the future, submit up to MAX_CONCURRENT_PROOF_REQUESTS at a time. Don't do one per
    /// loop.
    #[tracing::instrument(name = "proposer.request_queued_proofs", skip(self))]
    async fn request_queued_proofs(&self) -> Result<()> {
        let commitments = self.program_config.commitments.clone();
        let l1_chain_id = self.requester_config.l1_chain_id;
        let l2_chain_id = self.requester_config.l2_chain_id;

        let witness_gen_count = self
            .driver_config
            .driver_db_client
            .fetch_request_count(
                RequestStatus::WitnessGeneration,
                &commitments,
                l1_chain_id,
                l2_chain_id,
            )
            .await?;

        let execution_count = self
            .driver_config
            .driver_db_client
            .fetch_request_count(RequestStatus::Execution, &commitments, l1_chain_id, l2_chain_id)
            .await?;

        let prove_count = self
            .driver_config
            .driver_db_client
            .fetch_request_count(RequestStatus::Prove, &commitments, l1_chain_id, l2_chain_id)
            .await?;

        // If there are already MAX_CONCURRENT_PROOF_REQUESTS proofs in WitnessGeneration, Execute,
        // and Prove status, return.
        if witness_gen_count + execution_count + prove_count >=
            self.requester_config.max_concurrent_proof_requests as i64
        {
            debug!("There are already MAX_CONCURRENT_PROOF_REQUESTS proofs in WitnessGeneration, Execute, and Prove status.");
            return Ok(());
        }

        // If there are already MAX_CONCURRENT_WITNESS_GEN proofs in WitnessGeneration status,
        // return.
        if witness_gen_count >= self.requester_config.max_concurrent_witness_gen as i64 {
            debug!(
                "There are already MAX_CONCURRENT_WITNESS_GEN proofs in WitnessGeneration status."
            );
            return Ok(());
        }

        if let Some(request) = self.get_next_unrequested_proof().await? {
            // Guard: a request can be Unrequested yet still have a finished-but-not-yet-reaped task
            // in the map (e.g. one a witnessgen timeout just reset to Unrequested). Skip it this
            // cycle so we never spawn a duplicate task or overwrite its map entry —
            // handle_ongoing_tasks reaps it next iteration and it is then picked up cleanly.
            if self.tasks.lock().await.contains_key(&request.id) {
                return Ok(());
            }
            info!(
                request_id = request.id,
                request_type = ?request.req_type,
                start_block = request.start_block,
                end_block = request.end_block,
                "Making proof request"
            );
            let request_clone = request.clone();
            let proof_requester = self.proof_requester.clone();
            let handle =
                tokio::spawn(
                    async move { proof_requester.make_proof_request(request_clone).await },
                );
            self.tasks.lock().await.insert(request.id, (handle, request));
        }

        Ok(())
    }

    /// Get the next unrequested proof from the database.
    ///
    /// If there is an Aggregation proof with the same start block, range vkey commitment, and
    /// aggregation vkey, return that. Otherwise, return a range proof with the lowest start
    /// block.
    async fn get_next_unrequested_proof(&self) -> Result<Option<OPSuccinctRequest>> {
        let latest_proposed_block_number = get_latest_proposed_block_number(
            self.contract_config.l2oo_address,
            self.driver_config.fetcher.as_ref(),
        )
        .await?;

        let unreq_agg_request = self
            .driver_config
            .driver_db_client
            .fetch_unrequested_agg_proof(
                latest_proposed_block_number as i64,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        if let Some(unreq_agg_request) = unreq_agg_request {
            // Fetch consecutive range proofs from the database associated with the aggregation
            // proof request.
            let range_proofs = self
                .proof_requester
                .db_client
                .get_consecutive_complete_range_proofs(
                    unreq_agg_request.start_block,
                    unreq_agg_request.end_block,
                    &self.program_config.commitments,
                    self.requester_config.l1_chain_id,
                    self.requester_config.l2_chain_id,
                )
                .await?;

            // Validate the aggregation proof request
            match self.validate_aggregation_request(&range_proofs, &unreq_agg_request).await {
                true => {
                    debug!(
                        "Aggregation request validated successfully: start_block={}, end_block={}",
                        unreq_agg_request.start_block, unreq_agg_request.end_block
                    );
                    return Ok(Some(unreq_agg_request));
                }
                false => {
                    debug!(
                        "Aggregation request validation failed, moving to range proofs: start_block={}, end_block={}",
                        unreq_agg_request.start_block, unreq_agg_request.end_block
                    );
                    ValidityGauge::AggProofValidationErrorCount.increment(1.0);
                    // Validation failed, continue to try fetching range proofs
                }
            }
        }

        let unreq_range_request = self
            .driver_config
            .driver_db_client
            .fetch_first_unrequested_range_proof(
                latest_proposed_block_number as i64,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        if let Some(unreq_range_request) = unreq_range_request {
            return Ok(Some(unreq_range_request));
        }

        Ok(None)
    }

    /// Validates an aggregation proof request by checking that:
    /// 1. There are no gaps between consecutive range proofs
    /// 2. There are no duplicate/overlapping range proofs
    /// 3. The range proofs cover the entire block range
    pub async fn validate_aggregation_request(
        &self,
        range_proofs: &[OPSuccinctRequest],
        agg_request: &OPSuccinctRequest,
    ) -> bool {
        debug!(
            "Validating aggregation proof request: start_block={}, end_block={}",
            agg_request.start_block, agg_request.end_block
        );

        // Log all constituent range proofs
        for (i, proof) in range_proofs.iter().enumerate() {
            debug!(
                "Range proof {}: start_block={}, end_block={}",
                i, proof.start_block, proof.end_block
            );
        }

        // If no range proofs found, validation fails
        if range_proofs.is_empty() {
            warn!(
                start_block = ?agg_request.start_block,
                end_block = ?agg_request.end_block,
                commitments = ?self.program_config.commitments,
                "No consecutive span proof range found for request"
            );
            return false;
        }

        let first_range_proof_request =
            range_proofs.first().expect("Range proofs should not be empty");

        let last_range_proof_request =
            range_proofs.last().expect("Range proofs should not be empty");

        if first_range_proof_request.start_block != agg_request.start_block {
            warn!(
                expected_start_block = ?agg_request.start_block,
                actual_start_block = ?first_range_proof_request.start_block,
                commitments = ?self.program_config.commitments,
                "Range proofs start block does not match aggregation request"
            );

            return false;
        }

        if last_range_proof_request.end_block != agg_request.end_block {
            warn!(
                expected_end_block = ?agg_request.end_block,
                actual_end_block = ?last_range_proof_request.end_block,
                commitments = ?self.program_config.commitments,
                "Range proofs end block does not match aggregation request"
            );
            return false;
        }

        // Check for gaps and duplicates / overlaps between consecutive proofs
        for i in 1..range_proofs.len() {
            let prev_proof = &range_proofs[i - 1];
            let curr_proof = &range_proofs[i];

            // Check for gap
            if prev_proof.end_block < curr_proof.start_block {
                debug!(
                    "Gap detected: proof {} ends at {} but proof {} starts at {}",
                    i - 1,
                    prev_proof.end_block,
                    i,
                    curr_proof.start_block
                );
                return false;
            }

            // Check for overlap (duplicate blocks)
            if prev_proof.end_block > curr_proof.start_block {
                debug!(
                    "Overlap detected: proof {} ends at {} but proof {} starts at {}",
                    i - 1,
                    prev_proof.end_block,
                    i,
                    curr_proof.start_block
                );
                return false;
            }
        }

        // All validation checks passed
        debug!(
            "Aggregation request validated successfully with {} consecutive range proofs",
            range_proofs.len()
        );
        true
    }

    /// Relay all completed aggregation proofs to the contract.
    #[tracing::instrument(name = "proposer.submit_agg_proofs", skip(self))]
    async fn submit_agg_proofs(&self) -> Result<AdvancedContractHead> {
        // Cleared unconditionally and up front rather than on each exit path. Every early return
        // below either propagates (`?`) or reports no rejection, so setting it here is what keeps
        // a stale 1 from outliving the guard that set it — an operator who clears optimistic mode
        // while the L1 endpoint happens to be down would otherwise keep reading "blocked by a
        // contract guard" for as long as the endpoint stays down.
        ValidityGauge::AggProofBlockedByContractGuard.set(0.0);

        let latest_proposed_block_number = get_latest_proposed_block_number(
            self.contract_config.l2oo_address,
            self.driver_config.fetcher.as_ref(),
        )
        .await?;

        // See if there is an aggregation proof that is complete for this start block. NOTE: There
        // should only be one "pending" aggregation proof at a time for a specific start block.
        let completed_agg_proof = self
            .driver_config
            .driver_db_client
            .fetch_completed_agg_proof_after_block(
                latest_proposed_block_number as i64,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        // If there are no completed aggregation proofs, do nothing.
        let completed_agg_proof = match completed_agg_proof {
            Some(proof) => proof,
            None => return Ok(AdvancedContractHead::No),
        };

        let transaction_hash = match self.relay_aggregation_proof(&completed_agg_proof).await {
            Ok(RelayOutcome::Relayed(transaction_hash)) => transaction_hash,
            Ok(RelayOutcome::Rejected(rejection)) => {
                ValidityGauge::RelayAggProofErrorCount.increment(1.0);
                self.handle_relay_rejection(&completed_agg_proof, rejection).await;
                // Deliberately Ok: a rejection is an outcome, not an error. Returning Err would
                // make `run` retry after a fixed 10s instead of `LOOP_INTERVAL`,
                // and would skip `update_chain_lock` at the end of the iteration —
                // letting the lock lapse and a second proposer start alongside this
                // one.
                //
                // A rejection does not advance the contract head, so the caller must still create
                // this pass — that is what builds the replacement `handle_relay_rejection` just
                // made room for.
                return Ok(AdvancedContractHead::No);
            }
            Err(e) => {
                // No revert data at all: the transaction was never delivered (nonce, funds, a dead
                // RPC). That says nothing about the proof, so it keeps the existing retry
                // behaviour.
                ValidityGauge::RelayAggProofErrorCount.increment(1.0);
                return Err(e);
            }
        };

        info!("Relayed aggregation proof. Transaction hash: {:?}", transaction_hash);

        // Update the request to status RELAYED.
        self.driver_config
            .driver_db_client
            .update_request_to_relayed(
                completed_agg_proof.id,
                transaction_hash,
                self.contract_config.l2oo_address,
            )
            .await?;

        Ok(AdvancedContractHead::Yes)
    }

    /// Log a rejected aggregation and, unless the chain's own state is what refused it, fail the
    /// request so the next loop builds a replacement.
    ///
    /// Only [`RelayRejection::UnsatisfiableGuard`] keeps the request `Complete`. See
    /// `relay_rejection`'s module docs for why every other class — including revert data we cannot
    /// attribute — rebuilds instead: leaving a rejected proof `Complete` keeps it counted by
    /// `fetch_active_agg_proofs_count`, which blocks a replacement from ever being created and
    /// freezes the contract head until someone edits the database by hand.
    async fn handle_relay_rejection(&self, agg: &OPSuccinctRequest, rejection: RelayRejection) {
        let id = agg.id;
        let start_block = agg.start_block;
        let end_block = agg.end_block;
        let checkpointed_l1_block = agg.checkpointed_l1_block_number;
        let kind = rejection.kind();

        match &rejection {
            RelayRejection::ProofRejected { selector, name } => warn!(
                request_id = id, start_block, end_block, kind,
                error = name,
                selector = %format!("0x{}", hex::encode(selector)),
                ?checkpointed_l1_block,
                "The SP1 verifier rejected this aggregation proof; failing it so a new one is built. \
                 ACTION: a rebuild over the same range produces the same bytes, so if this repeats, \
                 the input is at fault rather than the proving run — inspect the covered range \
                 proofs (a non-zero guest exit code surfaces here as InvalidExitCode) and confirm \
                 the deployed verifier matches the aggregation vkey."
            ),
            RelayRejection::CheckpointUnusable { selector, name } => warn!(
                request_id = id, start_block, end_block, kind,
                error = name,
                selector = %format!("0x{}", hex::encode(selector)),
                ?checkpointed_l1_block,
                "The checkpointed L1 head this aggregation was built against is unusable; failing \
                 it so a new one re-checkpoints. The proof bytes are NOT at fault. ACTION: none \
                 required if this clears on the rebuild. If it repeats, check whether the \
                 checkpoint transaction is being confirmed, whether an L1 reorg removed it, and \
                 whether the L1 endpoint answering `historicBlockHashes` lags behind the one that \
                 sent it — a lagging node reports this for a checkpoint that does exist."
            ),
            RelayRejection::UnsatisfiableGuard { message } => warn!(
                request_id = id, start_block, end_block, kind,
                guard = %message,
                "proposeL2Output is refused by a contract guard that no proof can satisfy; NOT \
                 rebuilding. The proof itself is valid and stays Complete for resubmission. \
                 ACTION: an operator has to change contract state (or, for the timestamp guard, \
                 simply wait). Rebuilding would burn an aggregation proof for nothing."
            ),
            RelayRejection::RebuildableGuard { message } => warn!(
                request_id = id, start_block, end_block, kind,
                guard = %message,
                "A contract guard refused this aggregation, but a rebuild can clear it; failing it \
                 so a new one is built over a current range. ACTION: usually none — this is the \
                 expected path after submissionInterval is raised. If the guard is one you do not \
                 recognise, add it to UNSATISFIABLE_GUARDS if a rebuild cannot fix it."
            ),
            RelayRejection::ContractPanic { code } => warn!(
                request_id = id, start_block, end_block, kind,
                panic_code = %format!("0x{code:02x}"),
                "proposeL2Output hit a Solidity panic; failing the aggregation so a new one is \
                 built. ACTION: a panic is a contract bug, not a proof problem — decode the code \
                 (0x11 overflow, 0x12 division by zero, 0x01 assert) and check the oracle's state."
            ),
            RelayRejection::UnknownRevert { data } => warn!(
                request_id = id, start_block, end_block, kind,
                selector = %format!("0x{}", hex::encode(data.get(..4).unwrap_or_default())),
                revert_data = %format!("0x{}", hex::encode(data)),
                "proposeL2Output reverted with an error this build cannot decode; failing the \
                 aggregation so a new one is built rather than stalling the head. ACTION: the \
                 contract or verifier was likely upgraded — decode with `cast 4byte <selector>` and \
                 declare it in utils/host/src/contract.rs so it is classified precisely next time."
            ),
            RelayRejection::NoVerdict { reason } => warn!(
                request_id = id, start_block, end_block, kind,
                cause = ?reason,
                "The relay failed but no revert reason was recoverable; failing the aggregation so \
                 a new one is built rather than risking a permanent stall. ACTION: for \
                 ReplayDidNotRevert the likely cause is out of gas — the replay carries no gas \
                 limit so it cannot reproduce; compare the receipt's gas_used against its gas_limit."
            ),
        }

        let action = rejection_action(&rejection);
        ValidityGauge::AggProofBlockedByContractGuard.set(guard_gauge_value(action));

        let Some(next_status) = action else {
            return;
        };

        // Do not propagate a DB error: a failed transition just means the next loop resubmits and
        // is rejected again, which is recoverable, whereas losing the classification above
        // from the logs is what makes an incident take a day to diagnose.
        match self.driver_config.driver_db_client.update_request_status(id, next_status).await {
            // Counted only once the transition actually landed. Incrementing before the write would
            // make this gauge — the only evidence of a rebuild loop — read high on a DB failure
            // that rebuilt nothing, and tick again on every retry.
            Ok(_) => ValidityGauge::AggProofRebuiltAfterRejection.increment(1.0),
            Err(db_err) => warn!(
                request_id = id,
                error = ?db_err,
                "Could not mark the rejected aggregation as Failed; it will be resubmitted and \
                 rejected again until this transition succeeds"
            ),
        }
    }

    /// Submit the transaction to create a validity dispute game.
    ///
    /// If the DGF address is set, use it to create a new validity dispute game that will resolve
    /// with the proof. Otherwise, propose the L2 output.
    async fn relay_aggregation_proof(
        &self,
        completed_agg_proof: &OPSuccinctRequest,
    ) -> Result<RelayOutcome> {
        // Get the output at the end block of the last completed aggregation proof.
        let output = self
            .driver_config
            .fetcher
            .get_l2_output_at_block(completed_agg_proof.end_block as u64)
            .await?;

        // [MANTLE] v117 contracts ship no dispute-game implementation (Phase 3 dropped
        // Fault Proof + the OPSuccinctValidityDisputeGame type 6 contract). The DGF
        // path is unreachable on this build; surface a clear config error rather than
        // letting the call revert with an opaque "no implementation for game type" on
        // chain. Clear `DGF_ADDRESS` to use the L2OutputOracle directly.
        if self.contract_config.dgf_address != Address::ZERO {
            return Err(anyhow!(
                "DGF_ADDRESS is set ({}) but the v117 contract baseline ships no \
                 dispute-game contracts; clear DGF_ADDRESS to propose via the \
                 L2OutputOracle directly",
                self.contract_config.dgf_address,
            ));
        }

        // Propose the L2 output to the L2OutputOracle directly.
        let transaction_request = self
            .contract_config
            .l2oo_contract
            .proposeL2Output(
                output.output_root,
                U256::from(completed_agg_proof.end_block),
                U256::from(completed_agg_proof.checkpointed_l1_block_number.unwrap()),
                completed_agg_proof.proof.clone().unwrap().into(),
            )
            .into_transaction_request();

        // Cloned before sending because the signer consumes the request, and a transaction that
        // reverts on chain has to be replayed to recover why.
        let replay_request = transaction_request.clone();

        let sent = self
            .driver_config
            .signer
            .send_transaction_request_with_timeout(
                self.driver_config.fetcher.as_ref().rpc_config.l1_rpc.clone(),
                transaction_request,
                self.requester_config.tx_confirmation_timeout,
            )
            .await;

        match send_outcome(sent) {
            SendOutcome::Landed(tx_hash) => Ok(RelayOutcome::Relayed(tx_hash)),

            SendOutcome::MinedReverted(receipt) => {
                // Logged here because the receipt does not survive into `handle_relay_rejection`,
                // and these are the only numbers that can confirm the most likely
                // cause of a mined-and-reverted proposal: out of gas. The replay
                // below carries no gas limit, so it cannot reproduce that case and
                // will report `ReplayDidNotRevert` — at which point `gas_used ==
                // gas_limit` here is the evidence.
                warn!(
                    request_id = completed_agg_proof.id,
                    start_block = completed_agg_proof.start_block,
                    end_block = completed_agg_proof.end_block,
                    tx_hash = ?receipt.transaction_hash(),
                    block_number = ?receipt.block_number,
                    gas_used = receipt.gas_used,
                    effective_gas_price = receipt.effective_gas_price,
                    "proposeL2Output was mined but reverted; replaying it to recover the reason"
                );

                Ok(RelayOutcome::Rejected(self.classify_by_replay(replay_request, &receipt).await))
            }

            SendOutcome::RejectedBeforeBroadcast(data) => {
                Ok(RelayOutcome::Rejected(classify_revert_data(&data)))
            }

            SendOutcome::Undelivered(e) => Err(e),
        }
    }

    /// Recover a mined-and-reverted transaction's rejection by replaying it as `eth_call`.
    ///
    /// Replayed at the mined block and as the same sender, so the call observes the closest state
    /// we can address. Two caveats are inherent and bounded rather than fixed here: `eth_call`
    /// at block N sees N's post-state rather than the exact pre-state of our slot, so a
    /// same-block transaction that changed guard state can mislead the attribution; and the
    /// cloned request predates the signer's gas filling, so an out-of-gas failure cannot
    /// reproduce and lands in [`NoVerdictReason::ReplayDidNotRevert`].
    async fn classify_by_replay(
        &self,
        mut replay_request: TransactionRequest,
        receipt: &TransactionReceipt,
    ) -> RelayRejection {
        replay_request.from = Some(receipt.from);
        let block = receipt.block_number.map_or(BlockId::latest(), BlockId::number);

        // Bounded: the provider is built on a plain reqwest client with no request timeout, so an
        // L1 endpoint that accepts the connection and then stalls would hang the whole
        // proposer loop here without logging anything.
        let replay = tokio::time::timeout(
            Duration::from_secs(self.requester_config.network_calls_timeout),
            self.contract_config.l2oo_contract.provider().call(replay_request).block(block),
        )
        .await;

        replay_verdict(replay)
    }

    /// Validate the requester config matches the contract.
    async fn validate_contract_config(&self) -> Result<()> {
        // [MANTLE] v117 stores vkeys/rollup-config-hash as direct fields rather than
        // behind an `opSuccinctConfigs(_configName)` mapping — read them as three
        // separate calls. See contracts/src/validity/OPSuccinctL2OutputOracle.sol.
        let contract_agg_vkey_hash =
            self.contract_config.l2oo_contract.aggregationVkey().call().await?.0;
        let contract_range_vkey_commitment =
            self.contract_config.l2oo_contract.rangeVkeyCommitment().call().await?.0;
        let contract_rollup_config_hash =
            self.contract_config.l2oo_contract.rollupConfigHash().call().await?.0;

        let rollup_config_hash_match =
            contract_rollup_config_hash == self.program_config.commitments.rollup_config_hash;
        let agg_vkey_hash_match =
            contract_agg_vkey_hash == self.program_config.commitments.agg_vkey_hash;
        let range_vkey_commitment_match =
            contract_range_vkey_commitment == self.program_config.commitments.range_vkey_commitment;

        if !rollup_config_hash_match || !agg_vkey_hash_match || !range_vkey_commitment_match {
            tracing::error!(
                rollup_config_hash_match = rollup_config_hash_match,
                agg_vkey_hash_match = agg_vkey_hash_match,
                range_vkey_commitment_match = range_vkey_commitment_match,
                "Config mismatches detected."
            );

            if !rollup_config_hash_match {
                tracing::error!(
                    received = ?contract_rollup_config_hash,
                    expected = ?self.program_config.commitments.rollup_config_hash,
                    "Rollup config hash mismatch"
                );
            }

            if !agg_vkey_hash_match {
                tracing::error!(
                    received = ?contract_agg_vkey_hash,
                    expected = ?self.program_config.commitments.agg_vkey_hash,
                    "Aggregation vkey hash mismatch"
                );
            }

            if !range_vkey_commitment_match {
                tracing::error!(
                    received = ?contract_range_vkey_commitment,
                    expected = ?self.program_config.commitments.range_vkey_commitment,
                    "Range vkey commitment mismatch"
                );
            }

            return Err(anyhow::anyhow!("Config mismatches detected. Please run {{cargo run --bin config --release -- --env-file ENV_FILE}} to get the expected config for your contract."));
        }

        Ok(())
    }

    /// Set orphaned tasks to status FAILED. If a task is in the database in status Execution or
    /// WitnessGeneration but not in the tasks map, set it to status FAILED.
    async fn set_orphaned_tasks_to_failed(&self) -> Result<()> {
        let witnessgen_requests = self
            .driver_config
            .driver_db_client
            .fetch_requests_by_status(
                RequestStatus::WitnessGeneration,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        let execution_requests = self
            .driver_config
            .driver_db_client
            .fetch_requests_by_status(
                RequestStatus::Execution,
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        let requests = [witnessgen_requests, execution_requests].concat();

        // If a task is in the database in status Execution or WitnessGeneration but not in the
        // tasks map, set it to status FAILED.
        for request in requests {
            if !self.tasks.lock().await.contains_key(&request.id) {
                tracing::warn!(
                    request_id = request.id,
                    request_type = ?request.req_type,
                    "Task is in the database in status Execution or WitnessGeneration but not in the tasks map, setting to status FAILED."
                );
                self.driver_config
                    .driver_db_client
                    .update_request_status(request.id, RequestStatus::Failed)
                    .await?;
            }
        }

        Ok(())
    }

    /// Handle the ongoing witness generation and execution tasks.
    async fn handle_ongoing_tasks(&self) -> Result<()> {
        let mut tasks = self.tasks.lock().await;
        let mut completed = Vec::new();

        // Check and process completed tasks
        for (id, (handle, _)) in tasks.iter() {
            if handle.is_finished() {
                completed.push(*id);
            }
        }

        // Process completed tasks - this will properly await and drop them
        for id in completed {
            if let Some((handle, request)) = tasks.remove(&id) {
                // First await the handle to properly clean up the task.
                match handle.await {
                    Ok(result) => {
                        if let Err(e) = result {
                            warn!(
                                request_id = request.id,
                                request_type = ?request.req_type,
                                error = ?e,
                                "Task failed with error"
                            );
                            // Some failures must NOT bisect the range: a
                            // self-hosted admission shed (prover pool momentarily
                            // full) and a transient transport failure (the
                            // backend was unreachable — e.g. the gateway is down,
                            // gRPC `Unavailable` / "tcp connect error"). In both
                            // cases the request never produced a proof, so the
                            // range is fine; marking it Failed would feed range
                            // bisection and needlessly fragment it. Reset it to
                            // Unrequested and retry the SAME range next loop.
                            let no_bisect_reason = if is_admission_shed_error(&e) {
                                Some("self-hosted admission shed (prover pool full)")
                            } else if is_transient_transport_error(&e) {
                                Some("transient transport failure (backend unreachable)")
                            } else {
                                None
                            };
                            if let Some(reason) = no_bisect_reason {
                                warn!(
                                    request_id = request.id,
                                    request_type = ?request.req_type,
                                    reason,
                                    "resetting to Unrequested for retry (no bisection)"
                                );
                                if let Err(reset_err) = self
                                    .driver_config
                                    .driver_db_client
                                    .update_request_status(request.id, RequestStatus::Unrequested)
                                    .await
                                {
                                    warn!(
                                        error = ?reset_err,
                                        "Failed to reset request to Unrequested"
                                    );
                                }
                                continue;
                            }
                            // Now safe to retry as original task is cleaned up
                            match self
                                .proof_requester
                                .handle_failed_request(
                                    request,
                                    ExecutionStatus::UnspecifiedExecutionStatus as i32,
                                )
                                .await
                            {
                                Ok(_) => {
                                    ValidityGauge::ProofRequestRetryCount.increment(1.0);
                                }
                                Err(retry_err) => {
                                    warn!(error = ?retry_err, "Failed to retry request");
                                    ValidityGauge::RetryErrorCount.increment(1.0);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            request_id = request.id,
                            request_type = ?request.req_type,
                            error = ?e,
                            "Task panicked"
                        );
                        // Now safe to retry as original task is cleaned up
                        match self
                            .proof_requester
                            .handle_failed_request(
                                request,
                                ExecutionStatus::UnspecifiedExecutionStatus as i32,
                            )
                            .await
                        {
                            Ok(_) => {
                                ValidityGauge::ProofRequestRetryCount.increment(1.0);
                            }
                            Err(retry_err) => {
                                warn!(error = ?retry_err, "Failed to retry request after panic");
                                ValidityGauge::RetryErrorCount.increment(1.0);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Initialize the proposer by cleaning up stale requests and creating new range proof requests
    /// for the proposer with the given chain ID.
    ///
    /// This function performs several key tasks:
    /// 1. Validates that the proposer's config matches the contract
    /// 2. Deletes unrecoverable requests (UNREQUESTED, EXECUTION, WITNESS_GENERATION)
    /// 3. Cancels PROVE requests with mismatched commitment configs
    /// 4. Identifies gaps between the latest proposed block and finalized block
    /// 5. Creates new range proof requests to cover those gaps
    ///
    /// The goal is to ensure the database is in a clean state and all block ranges
    /// between the latest proposed block and finalized block have corresponding requests.
    #[tracing::instrument(name = "proposer.initialize_proposer", skip(self))]
    async fn initialize_proposer(&self) -> Result<()> {
        // Validate the requester config matches the contract.
        self.validate_contract_config()
            .await
            .context("Failed to validate the requester config matches the contract.")?;

        // Delete all requests for the same chain ID that are of status UNREQUESTED, EXECUTION or
        // WITNESS_GENERATION as they're unrecoverable.
        self.driver_config
            .driver_db_client
            .delete_all_requests_with_statuses(
                &[
                    RequestStatus::Unrequested,
                    RequestStatus::Execution,
                    RequestStatus::WitnessGeneration,
                ],
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        // Cancel all requests in PROVE state for the same chain id's that have a different
        // commitment config.
        self.driver_config
            .driver_db_client
            .cancel_prove_requests_with_different_commitment_config(
                &self.program_config.commitments,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        info!("Deleted all unrequested, execution, and witness generation requests and canceled all prove requests with different commitment configs.");

        Ok(())
    }

    /// Fetch and log the proposer metrics.
    async fn log_proposer_metrics(&self) -> Result<()> {
        // Get the latest proposed block number on the contract.
        let latest_proposed_block_number = get_latest_proposed_block_number(
            self.contract_config.l2oo_address,
            self.driver_config.fetcher.as_ref(),
        )
        .await?;

        // Get all completed range proofs from the database.
        let completed_range_proofs = self
            .driver_config
            .driver_db_client
            .fetch_completed_ranges(
                &self.program_config.commitments,
                latest_proposed_block_number as i64,
                self.requester_config.l1_chain_id,
                self.requester_config.l2_chain_id,
            )
            .await?;

        // Get the highest proven contiguous block.
        let highest_block_number = self
            .get_highest_proven_contiguous_block(completed_range_proofs)?
            .map_or(latest_proposed_block_number, |block| block as u64);

        // Fetch request counts for different statuses
        let commitments = &self.program_config.commitments;
        let l1_chain_id = self.requester_config.l1_chain_id;
        let l2_chain_id = self.requester_config.l2_chain_id;
        let db_client = &self.driver_config.driver_db_client;

        // Define statuses and their corresponding variable names
        let (
            num_unrequested_requests,
            num_prove_requests,
            num_execution_requests,
            num_witness_generation_requests,
        ) = (
            db_client
                .fetch_request_count(
                    RequestStatus::Unrequested,
                    commitments,
                    l1_chain_id,
                    l2_chain_id,
                )
                .await?,
            db_client
                .fetch_request_count(RequestStatus::Prove, commitments, l1_chain_id, l2_chain_id)
                .await?,
            db_client
                .fetch_request_count(
                    RequestStatus::Execution,
                    commitments,
                    l1_chain_id,
                    l2_chain_id,
                )
                .await?,
            db_client
                .fetch_request_count(
                    RequestStatus::WitnessGeneration,
                    commitments,
                    l1_chain_id,
                    l2_chain_id,
                )
                .await?,
        );

        // Log metrics
        info!(
            target: "proposer_metrics",
            "unrequested={num_unrequested_requests} prove={num_prove_requests} execution={num_execution_requests} witness_generation={num_witness_generation_requests} highest_contiguous_proven_block={highest_block_number} latest_proposed_block={latest_proposed_block_number}"
        );

        // Update gauges for proof counts
        ValidityGauge::CurrentUnrequestedProofs.set(num_unrequested_requests as f64);
        ValidityGauge::CurrentProvingProofs.set(num_prove_requests as f64);
        ValidityGauge::CurrentWitnessgenProofs.set(num_witness_generation_requests as f64);
        ValidityGauge::CurrentExecuteProofs.set(num_execution_requests as f64);
        ValidityGauge::HighestProvenContiguousBlock.set(highest_block_number as f64);
        ValidityGauge::LatestContractL2Block.set(latest_proposed_block_number as f64);

        // Get and set L2 block metrics
        let fetcher = &self.proof_requester.fetcher;
        ValidityGauge::L2UnsafeHeadBlock
            .set(fetcher.get_l2_header(BlockId::latest()).await?.number as f64);
        ValidityGauge::L2FinalizedBlock
            .set(fetcher.get_l2_header(BlockId::finalized()).await?.number as f64);

        // Get submission interval from contract and set gauge
        let contract_submission_interval: u64 =
            self.contract_config.l2oo_contract.submissionInterval().call().await?.try_into()?;

        let submission_interval =
            contract_submission_interval.max(self.requester_config.submission_interval);
        ValidityGauge::MinBlockToProveToAgg
            .set((latest_proposed_block_number + submission_interval) as f64);

        Ok(())
    }

    #[tracing::instrument(name = "proposer.run", skip(self))]
    pub async fn run(&self) -> Result<()> {
        // Handle the case where the proposer is being re-started and the proposer state needs to be
        // updated.
        self.initialize_proposer().await?;

        // Initialize the metrics gauges.
        ValidityGauge::init_all();

        // Loop interval in seconds.
        loop {
            // Wrap the entire loop body in a match to handle errors
            match self.run_loop_iteration().await {
                Ok(_) => {
                    // Normal sleep between iterations
                    tokio::time::sleep(Duration::from_secs(self.driver_config.loop_interval)).await;
                }
                Err(e) => {
                    // Log the error
                    tracing::error!("Error in proposer loop: {:?}", e);
                    // Update the error gauge
                    ValidityGauge::TotalErrorCount.increment(1.0);
                    // Pause for 10 seconds before restarting
                    tracing::debug!("Pausing for 10 seconds before restarting the process");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            }
        }
    }

    // Run a single loop of the validity proposer.
    async fn run_loop_iteration(&self) -> Result<()> {
        // Validate the requester config matches the contract.
        self.validate_contract_config().await?;

        // Log the proposer metrics.
        self.log_proposer_metrics().await?;

        // Handle the ongoing tasks.
        self.handle_ongoing_tasks().await?;

        // Set orphaned WitnessGeneration and Execution tasks to status Failed.
        self.set_orphaned_tasks_to_failed().await?;

        // Get all proof statuses of all requests in the proving state.
        self.handle_proving_requests().await?;

        // Add new range requests to the database.
        self.add_new_ranges().await?;

        // [MANTLE] The next two steps are the only ones in this iteration that broadcast an L1
        // transaction, and both are allowed to fail without aborting the pass. Every other step
        // still propagates.
        //
        // The rule is: a step that sends an L1 transaction which can revert for reasons that
        // resolve on their own is logged and skipped; a step that only reads or writes our
        // own database propagates. An L1 revert says nothing about whether the rest of the
        // iteration can make progress, and aborting costs far more than the step that
        // failed — every later step is skipped, including `request_queued_proofs`, which is
        // what keeps range proofs being produced at all, and `update_chain_lock`, whose
        // lease is exactly `LOOP_INTERVAL`. `run` also drops onto its 10s error path
        // instead of the configured interval.
        //
        // Concretely, both have a revert that waits on the chain rather than on us:
        // `checkpointBlockHash` reads `blockhash()`, which covers only the last 256 L1 blocks, so a
        // `safe` head outside that window during an L1 finality stall reverts until `safe` catches
        // up; and `proposeL2Output` can fail to reach the required confirmations within
        // `TX_CONFIRMATION_TIMEOUT` while L1 is congested.
        //
        // [MANTLE] Submit runs BEFORE create, and a sync must keep this order. It is NOT free to
        // choose: `handle_relay_rejection` moves the rejected row from `Complete` to `Failed`
        // within this very pass, and `fetch_active_agg_proofs_count` counts `Complete` while
        // `fetch_failed_agg_request_with_checkpointed_block_hash` reads `Failed`. Running submit
        // first is therefore what lets the same pass build the replacement for a rejected
        // aggregation; reverting to create-then-submit costs a full `LOOP_INTERVAL` per rejection.
        let advanced = match self.submit_agg_proofs().await {
            Ok(advanced) => advanced,
            Err(e) => {
                ValidityGauge::TotalErrorCount.increment(1.0);
                error!(
                    error = ?e,
                    "Could not submit an aggregation proof this pass; the iteration continues so \
                     range proofs are still requested and the chain lock is renewed. Note this is \
                     NOT a rejection by the chain — those are classified and handled inside \
                     `submit_agg_proofs` — but a failure to deliver the transaction at all. \
                     ACTION: if it persists, check L1 congestion against TX_CONFIRMATION_TIMEOUT, \
                     the proposer's balance and nonce, and that DGF_ADDRESS is unset on this \
                     contract baseline."
                );
                // Nothing was delivered, so the head did not move and create must still run.
                AdvancedContractHead::No
            }
        };

        // Create aggregation proofs based on the completed range proofs. Checkpoints the block hash
        // associated with the aggregation proof in advance.
        //
        // [MANTLE] Skipped for one pass after a successful relay — see [`AdvancedContractHead`].
        if advanced == AdvancedContractHead::Yes {
            debug!(
                "Relayed an aggregation this pass; deferring creation for one interval so the \
                 contract head is read after it has propagated."
            );
        } else if let Err(e) = self.create_aggregation_proofs().await {
            ValidityGauge::TotalErrorCount.increment(1.0);
            error!(
                error = ?e,
                "Could not create an aggregation proof this pass; the iteration continues so range \
                 proofs are still requested and the chain lock is renewed. ACTION: usually none — \
                 this retries next loop. If it persists, check that the L1 `safe` head is advancing \
                 and within 256 blocks of `latest` (checkpointBlockHash reverts otherwise), and \
                 that the checkpoint transaction is being confirmed."
            );
        }

        // Request all unrequested proofs from the prover network.
        self.request_queued_proofs().await?;

        // Update the chain lock.
        self.proof_requester
            .db_client
            .update_chain_lock(self.requester_config.l1_chain_id, self.requester_config.l2_chain_id)
            .await?;

        Ok(())
    }

    /// Get the highest block number at the end of the largest contiguous range of completed range
    /// proofs. Returns None if there are no completed range proofs.
    fn get_highest_proven_contiguous_block(
        &self,
        completed_range_proofs: Vec<(i64, i64)>,
    ) -> Result<Option<i64>> {
        Ok(highest_proven_contiguous_block(&completed_range_proofs))
    }

    /// Wrap a network prover call with timeout, logging, and metrics.
    async fn network_call_with_timeout<F, T>(
        &self,
        future: F,
        operation_name: &str,
        request: &OPSuccinctRequest,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        match tokio::time::timeout(
            Duration::from_secs(self.requester_config.network_calls_timeout),
            future,
        )
        .await
        {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(network_error)) => {
                warn!(
                    request_id = request.id,
                    start_block = request.start_block,
                    end_block = request.end_block,
                    operation = operation_name,
                    error = %network_error,
                    "Network error during operation"
                );
                Err(anyhow!(
                    "Network error {} for request {} (start_block={}, end_block={}): {}",
                    operation_name,
                    request.id,
                    request.start_block,
                    request.end_block,
                    network_error
                ))
            }
            Err(_) => {
                warn!(
                    request_id = request.id,
                    start_block = request.start_block,
                    end_block = request.end_block,
                    operation = operation_name,
                    timeout_secs = self.requester_config.network_calls_timeout,
                    "Network call timeout"
                );
                ValidityGauge::NetworkCallTimeoutCount.increment(1.0);
                Err(anyhow!(
                    "Timeout after {}s {} for request {} (start_block={}, end_block={})",
                    self.requester_config.network_calls_timeout,
                    operation_name,
                    request.id,
                    request.start_block,
                    request.end_block
                ))
            }
        }
    }
}

/// Highest block reachable by a contiguous chain of completed range proofs, starting from the
/// first proof and stopping at the first gap (a proof whose start block != the running end).
/// Returns None when there are no completed range proofs.
///
/// `completed_range_proofs` must be sorted by start block (as `fetch_completed_ranges` returns
/// them). Extracted as a free function so the contiguity semantics — which gate aggregation
/// proof creation — can be unit-tested without constructing a full `Proposer`.
fn highest_proven_contiguous_block(completed_range_proofs: &[(i64, i64)]) -> Option<i64> {
    let (first, rest) = completed_range_proofs.split_first()?;
    let mut current_end = first.1;
    for &(start, end) in rest {
        if start != current_end {
            break;
        }
        current_end = end;
    }
    Some(current_end)
}

#[cfg(test)]
mod rejection_action_tests {
    use alloy_primitives::Bytes;

    use super::{guard_gauge_value, rejection_action, RequestStatus};
    use crate::relay_rejection::{NoVerdictReason, RelayRejection};

    /// Pins the policy for every variant, so a change to it has to be deliberate.
    ///
    /// Exactly one rejection class keeps the request where it is. Everything else — including
    /// revert data this build cannot decode, and a failure whose reason could not be recovered
    /// at all — is failed so the next pass rebuilds. Wasting one aggregation proof is strictly
    /// preferable to leaving a rejected one `Complete`, which blocks any replacement from being
    /// created and freezes the contract head until someone edits the database by hand.
    #[test]
    fn only_an_unsatisfiable_guard_leaves_the_request_alone() {
        let rebuild = Some(RequestStatus::Failed);
        let park = None;

        let cases = [
            (
                RelayRejection::ProofRejected { selector: [0x09, 0xbd, 0xe3, 0x39], name: "x" },
                &rebuild,
            ),
            (
                RelayRejection::CheckpointUnusable {
                    selector: [0x22, 0xaa, 0x3a, 0x98],
                    name: "x",
                },
                &rebuild,
            ),
            (RelayRejection::RebuildableGuard { message: "m".into() }, &rebuild),
            (RelayRejection::ContractPanic { code: 0x11 }, &rebuild),
            (RelayRejection::UnknownRevert { data: Bytes::from(vec![0x1a]) }, &rebuild),
            (RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayTimedOut }, &rebuild),
            // The one exception.
            (RelayRejection::UnsatisfiableGuard { message: "m".into() }, &park),
        ];

        for (rejection, expected) in cases {
            assert_eq!(rejection_action(&rejection), *expected, "{}", rejection.kind());

            // The gauge an operator pages on must agree with what actually happened to the request.
            let expected_gauge = if expected.is_none() { 1.0 } else { 0.0 };
            assert_eq!(
                guard_gauge_value(*expected),
                expected_gauge,
                "gauge disagrees with the action for {}",
                rejection.kind()
            );
        }
    }
}

/// [UPSTREAM #923] Tests for the checkpoint selection this backport introduced.
#[cfg(test)]
mod checkpoint_selection_tests {
    use super::select_checkpoint_block_number;

    #[test]
    fn falls_back_to_safe_when_no_range_proof_records_an_l1_head() {
        // Proofs predating the `l1_head_block_number` column. `safe` is still reorg-stable, which
        // is the whole point of moving off `latest`.
        assert_eq!(select_checkpoint_block_number(1000, None), 1000);
    }

    #[test]
    fn uses_safe_when_it_already_covers_the_batch() {
        // The normal case under L1_BLOCK_TAG=finalized|safe: every range l1Head is at or below
        // safe.
        assert_eq!(select_checkpoint_block_number(1000, Some(900)), 1000);
        // Exactly equal is covered — the guest starts at the checkpoint itself, so it only has to
        // reach that head, not exceed it.
        assert_eq!(select_checkpoint_block_number(1000, Some(1000)), 1000);
    }

    #[test]
    fn floors_at_the_batch_max_when_a_range_head_is_newer_than_safe() {
        // Reachable under L1_BLOCK_TAG=latest. Without the floor the guest could not walk back to
        // that range proof's l1Head and the aggregation would fail on its header-chain assertion.
        assert_eq!(select_checkpoint_block_number(1000, Some(1005)), 1005);
    }
}

#[cfg(test)]
mod send_outcome_tests {
    use alloy_primitives::{Bytes, B256};
    use alloy_provider::network::ReceiptResponse;
    use alloy_rpc_types_eth::TransactionReceipt;
    use alloy_transport::{RpcError, TransportErrorKind};

    use super::{replay_verdict, send_outcome, SendOutcome};
    use crate::relay_rejection::{NoVerdictReason, RelayRejection};

    /// Shaped after an `eth_getTransactionReceipt` response (EIP-1559, no logs); the values are
    /// synthetic.
    ///
    /// Deserialising is a partial check, not a total one: dropping `status`, `gasUsed` or `logs`
    /// fails the parse, but alloy defaults `blockNumber` and `effectiveGasPrice` and ignores
    /// unknown fields. Those two are exactly what the mined-and-reverted log reports, so
    /// `a_reverted_receipt_carries_the_fields_the_out_of_gas_diagnosis_needs` asserts them
    /// directly rather than trusting the parse.
    fn receipt(status: &str) -> TransactionReceipt {
        let json = format!(
            r#"{{
                "type": "0x2",
                "status": "{status}",
                "cumulativeGasUsed": "0x1a2b3c",
                "logs": [],
                "logsBloom": "0x{zeros}",
                "transactionHash": "0x{tx:0>64}",
                "transactionIndex": "0x4",
                "blockHash": "0x{block:0>64}",
                "blockNumber": "0x10f2c",
                "gasUsed": "0x7a120",
                "effectiveGasPrice": "0x3b9aca00",
                "from": "0x1111111111111111111111111111111111111111",
                "to": "0x2222222222222222222222222222222222222222",
                "contractAddress": null
            }}"#,
            zeros = "0".repeat(512),
            tx = "aa",
            block = "bb",
        );
        serde_json::from_str(&json).expect("fixture matches the receipt schema")
    }

    fn rpc_error_carrying(data: &Bytes) -> anyhow::Error {
        anyhow::Error::new(RpcError::<TransportErrorKind>::ErrorResp(
            alloy_json_rpc::ErrorPayload {
                code: 3,
                message: "execution reverted".into(),
                data: Some(serde_json::value::to_raw_value(data).expect("serialisable")),
            },
        ))
    }

    #[test]
    fn a_successful_receipt_reports_the_hash_that_landed() {
        let out = send_outcome(Ok(receipt("0x1")));
        let SendOutcome::Landed(tx_hash) = out else { panic!("expected Landed, got {out:?}") };
        // Not just "some hash": a wrong hash sends an operator to the wrong transaction.
        assert_eq!(tx_hash, receipt("0x1").transaction_hash());
        assert_ne!(tx_hash, B256::ZERO);
    }

    /// The mined-and-reverted `warn!` is the only place these two numbers reach an operator, and
    /// `gas_used == gas_limit` is the evidence for out of gas — the one cause the replay cannot
    /// reproduce, since the cloned request predates the signer's gas filling. A receipt that
    /// silently defaulted them would make that log say nothing.
    #[test]
    fn a_reverted_receipt_carries_the_fields_the_out_of_gas_diagnosis_needs() {
        let receipt = receipt("0x0");
        assert_eq!(receipt.block_number, Some(0x10f2c));
        assert_eq!(receipt.effective_gas_price, 0x3b9aca00);
        assert_eq!(receipt.gas_used, 0x7a120);
    }

    #[test]
    fn a_reverted_receipt_is_not_treated_as_success() {
        // The failure this guards is silent: `send_transaction_request_with_timeout` returns `Ok`
        // for a mined-and-reverted transaction, so dropping the status check would mark the
        // aggregation relayed and advance the proposer past an output that never landed.
        assert!(matches!(send_outcome(Ok(receipt("0x0"))), SendOutcome::MinedReverted(_)));
    }

    #[test]
    fn an_error_carrying_revert_data_is_classifiable_without_a_replay() {
        // The common path: alloy's gas filler runs `eth_estimateGas` before broadcast, so a
        // deterministic revert never reaches a block and the data arrives on the error itself.
        let data = Bytes::from(vec![0x09, 0xbd, 0xe3, 0x39]);
        let out = send_outcome(Err(rpc_error_carrying(&data)));
        let SendOutcome::RejectedBeforeBroadcast(got) = out else {
            panic!("expected RejectedBeforeBroadcast, got {out:?}")
        };
        assert_eq!(got, data);
    }

    #[test]
    fn an_error_without_revert_data_is_retried_rather_than_classified() {
        // Nonce, funds, a dead RPC. Failing the request here would burn an aggregation proof for
        // an infrastructure hiccup that the next pass resolves on its own.
        let out = send_outcome(Err(anyhow::anyhow!("nonce too low")));
        assert!(matches!(out, SendOutcome::Undelivered(_)), "got {out:?}");
    }

    #[tokio::test]
    async fn a_replay_that_reverts_is_classified_from_its_data() {
        let data = Bytes::from(vec![0x09, 0xbd, 0xe3, 0x39]);
        let err = RpcError::<TransportErrorKind>::ErrorResp(alloy_json_rpc::ErrorPayload {
            code: 3,
            message: "execution reverted".into(),
            data: Some(serde_json::value::to_raw_value(&data).expect("serialisable")),
        });
        assert_eq!(
            replay_verdict(Ok(Err(err))),
            RelayRejection::ProofRejected {
                selector: [0x09, 0xbd, 0xe3, 0x39],
                name: "InvalidProof"
            }
        );
    }

    #[tokio::test]
    async fn a_replay_that_fails_without_data_is_reported_as_unreachable() {
        // A transport failure says nothing about the proof, so it must not be recorded as one.
        let err = RpcError::<TransportErrorKind>::Transport(TransportErrorKind::BackendGone);
        assert_eq!(
            replay_verdict(Ok(Err(err))),
            RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayUnreachable }
        );
    }

    #[tokio::test]
    async fn a_replay_that_succeeds_is_reported_as_such() {
        // Reached when the on-chain failure was out of gas: the cloned request predates the
        // signer's gas filling, so the replay has no gas limit to run out of. Distinguishing this
        // from a revert is what points an operator at the receipt's gas numbers.
        assert_eq!(
            replay_verdict(Ok(Ok(Bytes::new()))),
            RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayDidNotRevert }
        );
    }

    #[tokio::test]
    async fn a_replay_that_times_out_is_reported_as_such() {
        // The provider has no request timeout of its own, so a stalled L1 endpoint would otherwise
        // hang the proposer loop here. `Elapsed` is constructed the only way it can be.
        let elapsed = tokio::time::timeout(
            std::time::Duration::ZERO,
            std::future::pending::<std::result::Result<Bytes, RpcError<TransportErrorKind>>>(),
        )
        .await
        .expect_err("a zero timeout elapses immediately");

        assert_eq!(
            replay_verdict(Err(elapsed)),
            RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayTimedOut }
        );
    }
}

/// [UPSTREAM #923] Tests for the reuse-or-recheckpoint decision.
#[cfg(test)]
mod checkpoint_plan_tests {
    use alloy_eips::{BlockId, BlockNumberOrTag};
    use alloy_primitives::B256;

    use super::{checkpoint_plan, CachedCheckpoint, CheckpointPlan, RecheckpointReason};

    const HASH: B256 = B256::repeat_byte(0xab);
    const OTHER: B256 = B256::repeat_byte(0xcd);

    fn cached(number: u64, onchain_hash: B256) -> Option<CachedCheckpoint> {
        Some(CachedCheckpoint { hash: HASH, number, onchain_hash })
    }

    fn reason_of(plan: CheckpointPlan) -> RecheckpointReason {
        match plan {
            CheckpointPlan::Fresh { reason, .. } => reason,
            CheckpointPlan::Reuse { .. } => panic!("expected a fresh checkpoint, got {plan:?}"),
        }
    }

    /// The assertion that makes reverting [UPSTREAM #923] visible.
    ///
    /// The header is fetched by number and then checkpointed in a separate transaction. Anchoring
    /// that read at `latest` means a tip reorg between the two orphans the checkpoint, and the
    /// aggregation proof built on it is wasted — hit 3x on Mantle before the backport. Nothing
    /// downstream can detect the difference, so this is the only place it can be caught.
    #[test]
    fn a_fresh_checkpoint_is_always_anchored_at_a_reorg_stable_block() {
        let plans = [
            checkpoint_plan(None, None),
            checkpoint_plan(cached(100, B256::ZERO), None),
            checkpoint_plan(cached(100, OTHER), None),
            checkpoint_plan(cached(100, HASH), Some(200)),
        ];

        for plan in plans {
            let CheckpointPlan::Fresh { anchor, reason } = plan else {
                panic!("expected a fresh checkpoint, got {plan:?}");
            };
            assert_eq!(
                anchor,
                BlockId::Number(BlockNumberOrTag::Safe),
                "{reason:?} must anchor at `safe`; `latest` reorgs out from under the checkpoint"
            );
        }
    }

    #[test]
    fn a_checkpoint_the_contract_never_recorded_is_not_reused() {
        // `historicBlockHashes` returns zero for an unset entry, which is also what a lagging L1
        // endpoint returns for a checkpoint that does exist. Either way the proposal would revert
        // with `L1BlockHashNotCheckpointed`, so re-checkpointing is the only way forward.
        assert_eq!(
            reason_of(checkpoint_plan(cached(100, B256::ZERO), None)),
            RecheckpointReason::NotOnChain
        );
    }

    #[test]
    fn a_checkpoint_the_contract_disagrees_with_is_not_reused() {
        // The L1 block was reorged out between writing the row and the checkpoint transaction
        // executing. The guest would derive from a header that is no longer canonical.
        assert_eq!(
            reason_of(checkpoint_plan(cached(100, OTHER), None)),
            RecheckpointReason::HashMismatch
        );
    }

    /// The reuse path's own contribution to [UPSTREAM #923].
    ///
    /// A checkpoint can be genuinely on chain and still be useless: if it sits below a range
    /// proof's `l1Head`, the aggregation guest cannot walk its header chain back far enough and
    /// fails its assertion. Because a reused checkpoint is copied verbatim into every rebuilt row,
    /// accepting it here would repeat that failure indefinitely.
    #[test]
    fn a_valid_checkpoint_below_the_batch_max_l1_head_is_not_reused() {
        assert_eq!(
            reason_of(checkpoint_plan(cached(100, HASH), Some(101))),
            RecheckpointReason::BelowBatchMaxL1Head
        );
    }

    #[test]
    fn a_valid_checkpoint_that_covers_the_batch_is_reused() {
        let reuse = CheckpointPlan::Reuse { hash: HASH, number: 100 };

        // Strictly above, exactly equal — the guest starts at the checkpoint itself, so it only has
        // to reach the batch's max head, not exceed it — and no recorded head at all.
        assert_eq!(checkpoint_plan(cached(100, HASH), Some(99)), reuse);
        assert_eq!(checkpoint_plan(cached(100, HASH), Some(100)), reuse);
        assert_eq!(checkpoint_plan(cached(100, HASH), None), reuse);
    }

    #[test]
    fn no_prior_request_means_no_checkpoint_to_inherit() {
        assert_eq!(
            reason_of(checkpoint_plan(None, Some(100))),
            RecheckpointReason::NoCachedCheckpoint
        );
    }
}

#[cfg(test)]
mod contiguous_block_tests {
    use super::highest_proven_contiguous_block;

    #[test]
    fn empty_returns_none() {
        assert_eq!(highest_proven_contiguous_block(&[]), None);
    }

    #[test]
    fn single_range_returns_its_end() {
        assert_eq!(highest_proven_contiguous_block(&[(100, 200)]), Some(200));
    }

    #[test]
    fn fully_contiguous_chain_returns_last_end() {
        assert_eq!(
            highest_proven_contiguous_block(&[(100, 200), (200, 300), (300, 400)]),
            Some(400)
        );
    }

    #[test]
    fn stops_at_first_gap() {
        // 300 != 400: the chain breaks after the second range, so the third is not counted.
        assert_eq!(
            highest_proven_contiguous_block(&[(100, 200), (200, 300), (400, 500)]),
            Some(300)
        );
    }

    #[test]
    fn gap_immediately_after_first_range() {
        // Second range does not start at 200, so only the first range is contiguous.
        assert_eq!(highest_proven_contiguous_block(&[(100, 200), (300, 400)]), Some(200));
    }

    #[test]
    fn overlap_is_treated_as_a_gap() {
        // Overlapping (not exactly adjacent) ranges break contiguity: 200 != 250.
        assert_eq!(highest_proven_contiguous_block(&[(100, 250), (200, 300)]), Some(250));
    }
}

#[cfg(test)]
mod admission_shed_tests {
    use super::{is_admission_shed_error, is_transient_transport_error};

    #[test]
    fn detects_self_hosted_admission_shed_only() {
        // A self-hosted gateway shed carries the marker in the Status message +
        // metadata (which surface in the error's debug rendering).
        let shed = anyhow::anyhow!(
            "status: Unavailable, message: \"x-sp1-admission-shed: self-hosted range proof pool at capacity; retry shortly\""
        );
        assert!(is_admission_shed_error(&shed));

        // The Succinct network never emits this marker — must NOT be treated as
        // a shed (its path stays unchanged: genuine failure handling).
        let succinct_unavailable = anyhow::anyhow!(
            "status: Unavailable, message: \"succinct network temporarily unavailable\""
        );
        assert!(!is_admission_shed_error(&succinct_unavailable));

        // A genuine proof/range failure is not a shed either.
        let real_failure = anyhow::anyhow!("proof generation failed: execution unexecutable");
        assert!(!is_admission_shed_error(&real_failure));

        // The marker is detected even through anyhow context wrapping.
        let wrapped = shed.context("make_proof_request failed");
        assert!(is_admission_shed_error(&wrapped));
    }

    #[test]
    fn typed_status_code_is_the_primary_transport_signal() {
        // A REAL tonic::Status — the type the sp1-sdk actually produces and
        // carries in the anyhow error — is classified by its typed CODE, not its
        // text, so it is robust to Display drift and to incidental substrings.
        let unavailable: anyhow::Error = tonic::Status::unavailable("backend down").into();
        assert!(
            is_transient_transport_error(&unavailable),
            "a real UNAVAILABLE Status must be transient (no bisect)"
        );

        // The router "all backends down" shed carries NO tcp text; the typed code
        // still classifies it transient — exactly the case a string net would miss
        // if the Status Display format ever drifts across tonic versions.
        let cbs_open: anyhow::Error =
            tonic::Status::unavailable("all backends unavailable; both circuit breakers open")
                .into();
        assert!(is_transient_transport_error(&cbs_open));

        // A non-Unavailable Status is NOT transport-class and must still bisect
        // (Internal stands in for a genuine backend/proof fault). This also proves
        // the typed path can't be tripped by unrelated text.
        let internal: anyhow::Error =
            tonic::Status::internal("proof failed: tcp connect error mentioned in message").into();
        assert!(
            !is_transient_transport_error(&internal),
            "a non-Unavailable Status must not be treated as transient"
        );

        // Typed detection survives anyhow context wrapping (downcast walks the chain).
        let wrapped = anyhow::Error::from(tonic::Status::unavailable("x"))
            .context("request_range_proof failed");
        assert!(is_transient_transport_error(&wrapped));

        // A deterministic, non-Status error (no downcastable Status, no tcp text)
        // is not transport-class → bisect.
        let deterministic = anyhow::anyhow!("execution unexecutable: range too large");
        assert!(!is_transient_transport_error(&deterministic));
    }

    #[test]
    fn detects_transient_transport_errors() {
        // These use hand-written strings (NOT real `tonic::Status`), so they
        // exercise the STRING FALLBACK path — the typed path is covered above.
        // The production symptom: the network-gateway is down, so the router (or
        // the SDK) surfaces a gRPC UNAVAILABLE with a tcp connect error. This
        // must be treated as retryable-without-bisection.
        let gateway_down =
            anyhow::anyhow!("status: Unavailable, message: \"tcp connect error\", details: []");
        assert!(is_transient_transport_error(&gateway_down));

        // A Succinct-side transient unavailable is also transport-class: retry,
        // don't bisect (the range is fine, the backend was momentarily down).
        let succinct_unavailable = anyhow::anyhow!(
            "status: Unavailable, message: \"succinct network temporarily unavailable\""
        );
        assert!(is_transient_transport_error(&succinct_unavailable));

        // Lower-level connect failures (before a gRPC status is formed).
        let connect_err =
            anyhow::anyhow!("error trying to connect: tcp connect error: Connection refused");
        assert!(is_transient_transport_error(&connect_err));

        // Detected through anyhow context wrapping too.
        let wrapped = gateway_down.context("make_proof_request failed");
        assert!(is_transient_transport_error(&wrapped));

        // A genuine deterministic proof failure is NOT transport-class — it must
        // still bisect the range.
        let unexecutable = anyhow::anyhow!("proof generation failed: execution unexecutable");
        assert!(!is_transient_transport_error(&unexecutable));

        // An admission shed carries UNAVAILABLE, so it is also transport-class —
        // the handler checks the shed predicate first for a distinct log, but
        // either way the range is not bisected.
        let shed = anyhow::anyhow!(
            "status: Unavailable, message: \"x-sp1-admission-shed: pool at capacity\""
        );
        assert!(is_transient_transport_error(&shed));
    }
}
