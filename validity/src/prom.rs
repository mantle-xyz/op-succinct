use std::time::Duration;

use strum::EnumMessage;
use strum_macros::{Display, EnumIter};

use op_succinct_host_utils::metrics::MetricsGauge;

use crate::RequestType;

// Define an enum for all validity metrics gauges.
#[derive(Debug, Clone, Copy, Display, EnumIter, EnumMessage)]
pub enum ValidityGauge {
    // Proof status gauges
    #[strum(
        serialize = "succinct_current_unrequested_proofs",
        message = "Number of proofs currently unrequested"
    )]
    CurrentUnrequestedProofs,
    #[strum(
        serialize = "succinct_current_proving_proofs",
        message = "Number of proofs currently being proved"
    )]
    CurrentProvingProofs,
    #[strum(
        serialize = "succinct_current_witnessgen_proofs",
        message = "Number of proofs currently in witness generation"
    )]
    CurrentWitnessgenProofs,
    #[strum(
        serialize = "succinct_current_execute_proofs",
        message = "Number of proofs currently being executed"
    )]
    CurrentExecuteProofs,

    // Proposer gauges
    #[strum(
        serialize = "succinct_highest_proven_contiguous_block",
        message = "Highest proven contiguous block"
    )]
    HighestProvenContiguousBlock,
    #[strum(
        serialize = "succinct_latest_contract_l2_block",
        message = "Latest L2 block number from the contract"
    )]
    LatestContractL2Block,
    #[strum(serialize = "succinct_l2_unsafe_head_block", message = "L2 unsafe head block number")]
    L2UnsafeHeadBlock,
    #[strum(
        serialize = "succinct_l2_finalized_block",
        message = "L2 block returned by eth_getBlockByNumber(\"finalized\"); literal L2 finalized, \
                   unaffected by L1_BLOCK_TAG/L1_CONFIRMATIONS"
    )]
    L2FinalizedBlock,
    #[strum(
        serialize = "succinct_l2_max_provable_block",
        message = "Maximum L2 block the host is willing to anchor a proof against under the \
                   current backend + L1 selection. Matches L2 finalized under default \
                   Ethereum/EigenDA; reflects the L2 safe head at the configured L1 anchor under \
                   non-default Ethereum/EigenDA; reflects the Blobstream-resolved max provable L2 \
                   block under Celestia."
    )]
    L2MaxProvableBlock,
    #[strum(
        serialize = "succinct_min_block_to_prove_to_agg",
        message = "Minimum block number required to prove for aggregation"
    )]
    MinBlockToProveToAgg,
    #[strum(
        serialize = "succinct_proof_request_retry_count",
        message = "Number of proof request retries"
    )]
    ProofRequestRetryCount,

    // Error gauges
    #[strum(serialize = "succinct_total_error_count", message = "Number of total errors")]
    TotalErrorCount,
    #[strum(
        serialize = "succinct_proof_request_timeout_error_count",
        message = "Number of proof request timeout errors"
    )]
    ProofRequestTimeoutErrorCount,
    #[strum(serialize = "succinct_retry_error_count", message = "Number of retry errors")]
    RetryErrorCount,
    #[strum(
        serialize = "succinct_witnessgen_error_count",
        message = "Number of witness generation errors"
    )]
    WitnessgenErrorCount,
    #[strum(
        serialize = "succinct_witnessgen_timeout_count",
        message = "Number of witness generation timeouts"
    )]
    WitnessgenTimeoutCount,
    #[strum(serialize = "succinct_execution_error_count", message = "Number of execution errors")]
    ExecutionErrorCount,
    #[strum(
        serialize = "succinct_range_proof_request_error_count",
        message = "Number of range proof request errors"
    )]
    RangeProofRequestErrorCount,
    #[strum(
        serialize = "succinct_agg_proof_request_error_count",
        message = "Number of aggregation proof request errors"
    )]
    AggProofRequestErrorCount,
    #[strum(
        serialize = "succinct_agg_proof_validation_error_count",
        message = "Number of aggregation proof validation errors"
    )]
    AggProofValidationErrorCount,
    #[strum(
        serialize = "succinct_relay_agg_proof_error_count",
        message = "Number of relay aggregation proof errors"
    )]
    RelayAggProofErrorCount,
    #[strum(
        serialize = "succinct_network_call_timeout_count",
        message = "Number of network prover call timeouts"
    )]
    NetworkCallTimeoutCount,
    // [MANTLE] Both gauges below belong to the relay-rejection path, which upstream does not
    // have. Nothing will conflict on a sync; keep them. See MANTLE_CHANGES.md §3.9.
    #[strum(
        serialize = "succinct_agg_proof_blocked_by_contract_guard",
        message = "1 while proposeL2Output is refused by a contract guard no proof can satisfy (optimistic mode, proposer approval, future timestamp, zero output root); needs operator action, not a new proof. Written on every submit_agg_proofs pass that reaches a verdict — relayed, rejected, or undelivered — so it cannot outlive the guard that set it. A pass that aborts earlier (a dead L1 endpoint fails validate_contract_config first) leaves the previous value, so read it alongside succinct_total_error_count."
    )]
    AggProofBlockedByContractGuard,
    #[strum(
        serialize = "succinct_agg_proof_rebuilt_after_rejection_count",
        message = "Number of aggregation proofs failed and rebuilt because the chain rejected the previous one"
    )]
    AggProofRebuiltAfterRejection,

    // [MANTLE] Chain lock lease and main-loop progress. See chain_lock.rs / MANTLE_CHANGES.md
    // §3.10d. A healthy heartbeat does not mean a healthy proposer: alert on
    // `time() - succinct_loop_last_progress_timestamp` as well.
    #[strum(
        serialize = "succinct_chain_lock_held",
        message = "1 while this process holds the chain lock lease and may send L1 transactions"
    )]
    ChainLockHeld,
    #[strum(
        serialize = "succinct_chain_lock_lease_remaining_secs",
        message = "Seconds left before this process's local chain lock deadline (lease minus safety margin), as of the last renewal"
    )]
    ChainLockLeaseRemainingSecs,
    #[strum(
        serialize = "succinct_chain_lock_renew_failures",
        message = "Number of chain lock renewals that errored or timed out (a lost lease is logged and ends the process)"
    )]
    ChainLockRenewFailures,
    #[strum(
        serialize = "succinct_loop_last_progress_timestamp",
        message = "Unix time at which the main loop last completed an iteration (or started)"
    )]
    LoopLastProgressTimestamp,
}

impl MetricsGauge for ValidityGauge {}

// [MANTLE] Per-stage latency of the proving pipeline, as Prometheus histograms (the gauges above
// can only hold one value, and witness generation runs several requests at once). Explicit
// buckets are installed by `init_metrics_with_buckets` via `HISTOGRAM_BUCKETS`; without them the
// exporter would render these as summaries, which `histogram_quantile` cannot use.
// See MANTLE_CHANGES.md §3.13.

/// Wall time of witness generation per request, until the stdin is built. For a range: host
/// fetch + native run + stdin build; for an aggregation: loading the range proofs from the DB +
/// stdin build.
/// Labels: `req_type` = `range` | `aggregation`; `outcome` = `ok` | `error` | `timeout`.
pub const WITNESSGEN_DURATION_SECONDS: &str = "succinct_witnessgen_duration_seconds";

/// Wall time of handing a proof to the prover — one library call per backend, measured whole:
/// - `network` (sp1-sdk `request()`): program registration, auction params (mainnet), stdin upload,
///   request creation. No local execution: limits are explicit and ranges skip simulation.
/// - `cluster` (sp1-cluster-utils `create_request`): ELF + stdin upload, request creation.
///
/// Labels: `req_type`; `backend` = `network` | `cluster`; `outcome` = `ok` | `error`.
pub const PROOF_SUBMIT_DURATION_SECONDS: &str = "succinct_proof_submit_duration_seconds";

/// Bucket bounds in seconds. Witness generation of a 450-block range takes ~20s, an 1800-block
/// one proportionally longer (hence 45/90), and is cut off by `WITNESSGEN_TIMEOUT` (14400s by
/// default). Submission is seconds unless it stalls; a failing one can run sp1-sdk's per-step
/// retry budget (120s each for artifact creation, upload and request), hence the 600s bucket.
pub const HISTOGRAM_BUCKETS: &[(&str, &[f64])] = &[
    (
        WITNESSGEN_DURATION_SECONDS,
        &[
            5.0, 10.0, 20.0, 30.0, 45.0, 60.0, 90.0, 120.0, 300.0, 600.0, 1200.0, 1800.0, 3600.0,
            7200.0, 14400.0,
        ],
    ),
    (
        PROOF_SUBMIT_DURATION_SECONDS,
        &[0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0, 600.0],
    ),
];

/// How a timed stage ended (`outcome` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageOutcome {
    Ok,
    Error,
    /// Only witness generation has its own timeout.
    Timeout,
}

impl StageOutcome {
    pub fn of<T, E>(result: &Result<T, E>) -> Self {
        if result.is_ok() {
            Self::Ok
        } else {
            Self::Error
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }
}

/// Which prover backend a proof was submitted to (`backend` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofBackend {
    Network,
    Cluster,
}

impl ProofBackend {
    fn label(self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Cluster => "cluster",
        }
    }
}

pub fn describe_histograms() {
    metrics::describe_histogram!(
        WITNESSGEN_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "Witness generation wall time per request, by req_type and outcome (ok/error/timeout)"
    );
    metrics::describe_histogram!(
        PROOF_SUBMIT_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "Proof submission wall time (network: program registration + auction params + stdin upload + request creation; cluster: ELF/stdin upload + request creation), by req_type, backend and outcome"
    );
}

fn req_type_label(req_type: RequestType) -> &'static str {
    match req_type {
        RequestType::Range => "range",
        RequestType::Aggregation => "aggregation",
    }
}

pub fn record_witnessgen_duration(req_type: RequestType, outcome: StageOutcome, d: Duration) {
    metrics::histogram!(
        WITNESSGEN_DURATION_SECONDS,
        "req_type" => req_type_label(req_type),
        "outcome" => outcome.label(),
    )
    .record(d.as_secs_f64());
}

pub fn record_proof_submit_duration(
    req_type: RequestType,
    backend: ProofBackend,
    outcome: StageOutcome,
    d: Duration,
) {
    metrics::histogram!(
        PROOF_SUBMIT_DURATION_SECONDS,
        "req_type" => req_type_label(req_type),
        "backend" => backend.label(),
        "outcome" => outcome.label(),
    )
    .record(d.as_secs_f64());
}

#[cfg(test)]
mod histogram_tests {
    use super::*;
    use op_succinct_host_utils::metrics::prometheus_builder;

    /// The buckets must reach the exporter, or Grafana gets a summary it cannot aggregate.
    /// Uses the same builder `init_metrics_with_buckets` installs, and the same descriptions.
    #[test]
    fn stage_latencies_render_as_prometheus_histograms() {
        for (name, buckets) in HISTOGRAM_BUCKETS {
            assert!(buckets.windows(2).all(|w| w[0] < w[1]), "{name}: buckets must ascend");
        }
        let recorder = prometheus_builder(HISTOGRAM_BUCKETS).build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            describe_histograms();
            record_witnessgen_duration(
                RequestType::Range,
                StageOutcome::Error,
                Duration::from_secs(21),
            );
            record_proof_submit_duration(
                RequestType::Aggregation,
                ProofBackend::Network,
                StageOutcome::Ok,
                Duration::from_millis(1500),
            );
        });
        let out = handle.render();
        for name in [WITNESSGEN_DURATION_SECONDS, PROOF_SUBMIT_DURATION_SECONDS] {
            assert!(out.contains(&format!("# TYPE {name} histogram")), "{out}");
            assert!(out.contains(&format!("# HELP {name} ")), "{out}");
        }
        assert!(
            out.contains(r#"succinct_witnessgen_duration_seconds_bucket{req_type="range",outcome="error",le="30"} 1"#),
            "{out}"
        );
        assert!(
            out.contains(r#"succinct_proof_submit_duration_seconds_bucket{req_type="aggregation",backend="network",outcome="ok",le="2"} 1"#),
            "{out}"
        );
    }
}
