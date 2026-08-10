//! Classifying a rejected `proposeL2Output` from **typed revert data**.
//!
//! # Why not match the rendered error text
//!
//! An earlier version of this logic classified rejections by substring-matching
//! `format!("{e:?}")` of the `anyhow` error. Three rounds of review found defects of the same
//! family each time, because that approach reads an indirect signal:
//!
//! - the text depends on which L1 client answered (geth appends the revert reason to the message,
//!   others return only `data`), on the tonic/alloy version's `Display`, on capitalisation, and on
//!   how many `anyhow` context layers wrap the error;
//! - `contains("l2outputoracle:")` also matches the Rust path `OPSuccinctL2OutputOracle::` — the
//!   `::` supplies the colon — so any error whose context named the type was misread as a contract
//!   guard, and a genuine `InvalidExitCode()` verdict was silently swallowed.
//!
//! The revert data itself carries the same information exactly: `require(cond, "…")` is ABI-encoded
//! as `Error(string)` (`0x08c379a0`) and decodes back to the original string, and every other
//! revert surface is a 4-byte selector. So this module decodes rather than matches, and the
//! classifier is a plain function over `Option<&Bytes>` — no provider, no `anyhow` chain,
//! exhaustively testable.
//!
//! # Which way to err
//!
//! Only [`RelayRejection::UnsatisfiableGuard`] keeps the aggregation out of a rebuild. Everything
//! else — including revert data we cannot attribute — rebuilds.
//!
//! That asymmetry is deliberate. Leaving a rejected aggregation `Complete` keeps it counted by
//! `fetch_active_agg_proofs_count`, which stops a replacement from ever being created: the contract
//! head then freezes until someone edits the database by hand. That is the original QA3 incident.
//! Rebuilding when we did not need to costs one aggregation proof. Wasting a proof is strictly
//! preferable to stalling the chain, so anything we cannot *prove* is unrelated to the proof bytes
//! is treated as if it were.

use alloy_primitives::Bytes;
use alloy_sol_types::{ContractError, SolInterface};
use alloy_transport::{RpcError, TransportErrorKind};
use op_succinct_host_utils::{
    OPSuccinctL2OutputOracle::OPSuccinctL2OutputOracleErrors, SP1Verifier::SP1VerifierErrors,
};

/// Pull the contract's revert data out of a failed JSON-RPC call.
///
/// Deliberately does NOT use alloy's [`ErrorPayload::as_revert_data`], which first checks
/// `message.contains("revert")`. A client that returns the data without that word in its message —
/// Nethermind answers `"VM execution error."` — would then yield `None`, and a real verdict on the
/// proof would be misread as "no revert data was recoverable". `try_data_as` reads the `data` field
/// directly and never looks at the message.
///
/// [`ErrorPayload::as_revert_data`]: alloy_transport::RpcError
pub fn revert_data_of_rpc_error(err: &RpcError<TransportErrorKind>) -> Option<Bytes> {
    err.as_error_resp()?.try_data_as::<Bytes>().and_then(Result::ok)
}

/// Same, for an error already wrapped by `anyhow`.
///
/// The signer wraps with `.context(..)`, which keeps the original error in the source chain, so the
/// downcast still resolves — which matters because most deterministic reverts never reach a
/// receipt: alloy's gas filler runs `eth_estimateGas` first, so the node rejects the call up front
/// and the transaction is never broadcast.
pub fn revert_data_of_anyhow(err: &anyhow::Error) -> Option<Bytes> {
    revert_data_of_rpc_error(err.downcast_ref::<RpcError<TransportErrorKind>>()?)
}

/// The `require` messages on `proposeL2Output` that **no proof can satisfy**, verbatim from
/// `contracts/src/validity/OPSuccinctL2OutputOracle.sol`.
///
/// Two need an operator to change contract state (optimistic mode, proposer approval); the other
/// two resolve on their own — the timestamp guard as time passes, and the zero-output-root guard
/// once whatever produced an empty root is fixed, since the output root is re-read from the fetcher
/// on every resubmission rather than baked into the proof. None of the four is affected by the
/// proof bytes, so resubmitting is correct and rebuilding is pure waste.
///
/// Deliberately a list of the *unsatisfiable* ones rather than of all guards: a `require` added
/// upstream falls through to [`RelayRejection::RebuildableGuard`], which wastes a proof rather than
/// stalling the head. `guard_list_still_matches_the_contract` fails when the contract's set
/// changes, so the new one gets classified deliberately instead of by default.
const UNSATISFIABLE_GUARDS: [&str; 4] = [
    "L2OutputOracle: optimistic mode is enabled",
    "L2OutputOracle: only approved proposers can propose new outputs",
    "L2OutputOracle: cannot propose L2 output in the future",
    "L2OutputOracle: L2 output proposal cannot be the zero hash",
];

/// Why no verdict on the proof could be obtained.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoVerdictReason {
    /// Replaying the call as `eth_call` did not revert. The failure was therefore not
    /// deterministic — most likely out of gas, which cannot reproduce because the replay carries
    /// no gas limit — or state moved between the two calls.
    ReplayDidNotRevert,
    /// The replay could not reach the node.
    ReplayUnreachable,
    /// The replay did not return in time.
    ReplayTimedOut,
}

/// What the chain said when it refused `proposeL2Output`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayRejection {
    /// A verdict on the proof bytes: the same bytes will be refused on every resubmission.
    ProofRejected {
        /// The 4-byte error selector, for logs and for `cast 4byte`.
        selector: [u8; 4],
        /// Solidity name of the error.
        name: &'static str,
    },
    /// A guard that decoding proves is unrelated to the proof bytes, and that rebuilding cannot
    /// clear. Needs an operator, or time.
    UnsatisfiableGuard { message: String },
    /// A contract guard that a rebuild *can* clear — notably
    /// `block number must be greater than or equal to next expected block number`, which appears
    /// after an operator raises `submissionInterval` and is fixed by aggregating a wider range.
    /// Also the fallback for any `require` message not in [`UNSATISFIABLE_GUARDS`].
    RebuildableGuard { message: String },
    /// The contract hit a Solidity `assert` / arithmetic panic. Not something a proof can fix, but
    /// also not something we can attribute, so it rebuilds.
    ContractPanic { code: u64 },
    /// Revert data that decoded to none of the known surfaces — most likely the contract or the
    /// verifier was upgraded. Rebuilds, and logs the selector so it can be added here.
    UnknownRevert { data: Bytes },
    /// No revert data was recoverable at all.
    NoVerdict { reason: NoVerdictReason },
}

impl RelayRejection {
    /// Whether the aggregation should be failed so a fresh one is built over the same range.
    ///
    /// Everything except [`Self::UnsatisfiableGuard`] rebuilds; see the module docs for why that is
    /// the safe direction.
    pub fn should_rebuild(&self) -> bool {
        !matches!(self, Self::UnsatisfiableGuard { .. })
    }

    /// A short, stable label for logs and metrics.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ProofRejected { .. } => "proof_rejected",
            Self::UnsatisfiableGuard { .. } => "unsatisfiable_guard",
            Self::RebuildableGuard { .. } => "rebuildable_guard",
            Self::ContractPanic { .. } => "contract_panic",
            Self::UnknownRevert { .. } => "unknown_revert",
            Self::NoVerdict { .. } => "no_verdict",
        }
    }
}

/// Classify a rejected `proposeL2Output` from the revert data the node returned.
///
/// `None` means no revert data was recoverable; the caller supplies why via `no_verdict_reason`.
pub fn classify_relay_rejection(
    revert_data: Option<&Bytes>,
    no_verdict_reason: NoVerdictReason,
) -> RelayRejection {
    match revert_data {
        Some(data) => classify_revert_data(data),
        None => RelayRejection::NoVerdict { reason: no_verdict_reason },
    }
}

/// Classify revert data that is known to be present.
pub fn classify_revert_data(data: &Bytes) -> RelayRejection {
    // `ContractError` covers the two standard surfaces (`Error(string)` from `require`,
    // `Panic(u256)` from `assert`) plus the oracle's own custom errors in one decode.
    if let Ok(decoded) = ContractError::<OPSuccinctL2OutputOracleErrors>::abi_decode(data) {
        return match decoded {
            ContractError::Revert(revert) => classify_require_message(revert.reason),
            ContractError::Panic(panic) => {
                RelayRejection::ContractPanic { code: panic.code.saturating_to() }
            }
            ContractError::CustomError(err) => RelayRejection::ProofRejected {
                selector: err.selector(),
                name: oracle_error_name(&err),
            },
        };
    }

    // The verifier's errors bubble up through `verifyProof`, so they are not part of the oracle's
    // interface and need a second attempt.
    if let Ok(err) = SP1VerifierErrors::abi_decode(data) {
        return RelayRejection::ProofRejected {
            selector: err.selector(),
            name: verifier_error_name(&err),
        };
    }

    RelayRejection::UnknownRevert { data: data.clone() }
}

/// Split a decoded `require` message into "no proof can fix this" and "a rebuild can".
fn classify_require_message(message: String) -> RelayRejection {
    if UNSATISFIABLE_GUARDS.contains(&message.as_str()) {
        RelayRejection::UnsatisfiableGuard { message }
    } else {
        RelayRejection::RebuildableGuard { message }
    }
}

/// Both oracle custom errors mean the checkpointed L1 head is unusable, which a fresh aggregation
/// re-checkpoints.
fn oracle_error_name(err: &OPSuccinctL2OutputOracleErrors) -> &'static str {
    match err {
        OPSuccinctL2OutputOracleErrors::L1BlockHashNotCheckpointed(_) => {
            "L1BlockHashNotCheckpointed"
        }
        OPSuccinctL2OutputOracleErrors::L1BlockHashNotAvailable(_) => "L1BlockHashNotAvailable",
    }
}

fn verifier_error_name(err: &SP1VerifierErrors) -> &'static str {
    match err {
        SP1VerifierErrors::InvalidProof(_) => "InvalidProof",
        SP1VerifierErrors::InvalidExitCode(_) => "InvalidExitCode",
    }
}

#[cfg(test)]
mod tests {
    use alloy_sol_types::{SolError, SolValue};

    use super::*;

    /// The contract source, read at compile time so the pinned guard list is checked against the
    /// real `require`s rather than against a copy that silently drifts.
    const L2OO_SOURCE: &str =
        include_str!("../../contracts/src/validity/OPSuccinctL2OutputOracle.sol");

    /// The one guard on `proposeL2Output` that a rebuild clears.
    const REBUILDABLE_GUARD: &str =
        "L2OutputOracle: block number must be greater than or equal to next expected block number";

    /// ABI-encode a `require(false, msg)` revert exactly as a node would return it.
    fn require_revert(message: &str) -> Bytes {
        let mut data = alloy_sol_types::Revert::SELECTOR.to_vec();
        data.extend_from_slice(&message.to_string().abi_encode());
        data.into()
    }

    fn selector_only(selector: [u8; 4]) -> Bytes {
        selector.to_vec().into()
    }

    #[test]
    fn guard_list_still_matches_the_contract() {
        // Turns "these are pinned" into an actual check: a reword upstream fails here instead of
        // silently reclassifying that guard as rebuildable.
        for guard in UNSATISFIABLE_GUARDS {
            assert!(L2OO_SOURCE.contains(guard), "no longer in the contract: {guard}");
        }
        assert!(L2OO_SOURCE.contains(REBUILDABLE_GUARD), "no longer in the contract");

        // Scoped to `proposeL2Output`'s own body — the file has ~27 `L2OutputOracle: ` messages
        // overall, so counting all of them would prove nothing. Four live in the body; the fifth
        // (optimistic mode) is in the `whenNotOptimistic` modifier.
        let start =
            L2OO_SOURCE.find("function proposeL2Output").expect("proposeL2Output still exists");
        let rest = &L2OO_SOURCE[start..];
        let end = rest[1..].find("\n    function ").map_or(rest.len(), |i| i + 1);
        let body = &rest[..end];

        assert_eq!(
            body.matches("\"L2OutputOracle: ").count(),
            4,
            "proposeL2Output gained or lost a require: classify the new one as unsatisfiable or \
             leave it rebuildable, then update this count"
        );
        assert!(
            body.contains("whenNotOptimistic"),
            "the optimistic-mode guard is no longer applied to proposeL2Output"
        );
    }

    #[test]
    fn unsatisfiable_guards_do_not_rebuild() {
        for guard in UNSATISFIABLE_GUARDS {
            let r = classify_relay_rejection(
                Some(&require_revert(guard)),
                NoVerdictReason::ReplayUnreachable,
            );
            assert_eq!(
                r,
                RelayRejection::UnsatisfiableGuard { message: guard.to_string() },
                "misclassified: {guard}"
            );
            assert!(!r.should_rebuild(), "must not rebuild on: {guard}");
        }
    }

    #[test]
    fn the_block_number_guard_rebuilds() {
        // After an operator raises `submissionInterval`, an aggregation whose `end_block` came from
        // the old interval can ONLY recover by being rebuilt over a wider range. Treating it as
        // unsatisfiable would leave it `Complete` forever, blocking any replacement.
        let r = classify_relay_rejection(
            Some(&require_revert(REBUILDABLE_GUARD)),
            NoVerdictReason::ReplayUnreachable,
        );
        assert_eq!(r, RelayRejection::RebuildableGuard { message: REBUILDABLE_GUARD.to_string() });
        assert!(r.should_rebuild());
    }

    #[test]
    fn an_unlisted_require_rebuilds_rather_than_stalling() {
        // A `require` added upstream must waste a proof, not freeze the contract head.
        let r = classify_relay_rejection(
            Some(&require_revert("L2OutputOracle: some future guard")),
            NoVerdictReason::ReplayUnreachable,
        );
        assert!(matches!(r, RelayRejection::RebuildableGuard { .. }));
        assert!(r.should_rebuild());
    }

    #[test]
    fn verdicts_on_the_proof_bytes_are_recognised_by_selector() {
        // Selectors verified with `cast sig`.
        let cases: [([u8; 4], &str); 4] = [
            ([0x09, 0xbd, 0xe3, 0x39], "InvalidProof"),
            ([0x1f, 0xcf, 0x91, 0x77], "InvalidExitCode"),
            ([0x22, 0xaa, 0x3a, 0x98], "L1BlockHashNotCheckpointed"),
            ([0x84, 0xc0, 0x68, 0x64], "L1BlockHashNotAvailable"),
        ];

        for (selector, name) in cases {
            let r = classify_relay_rejection(
                Some(&selector_only(selector)),
                NoVerdictReason::ReplayUnreachable,
            );
            assert_eq!(
                r,
                RelayRejection::ProofRejected { selector, name },
                "selector {selector:?}"
            );
            assert!(r.should_rebuild());
        }
    }

    #[test]
    fn an_unrecognised_selector_rebuilds_and_keeps_the_data() {
        // The contract or verifier was upgraded. Rebuilding wastes a proof; not rebuilding would
        // freeze the head. The data is preserved so the log can name the selector.
        let data = selector_only([0x1a, 0x2b, 0x3c, 0x4d]);
        let r = classify_relay_rejection(Some(&data), NoVerdictReason::ReplayUnreachable);
        assert_eq!(r, RelayRejection::UnknownRevert { data });
        assert!(r.should_rebuild());
    }

    #[test]
    fn no_revert_data_carries_the_callers_reason_and_rebuilds() {
        for reason in [
            NoVerdictReason::ReplayDidNotRevert,
            NoVerdictReason::ReplayUnreachable,
            NoVerdictReason::ReplayTimedOut,
        ] {
            let r = classify_relay_rejection(None, reason.clone());
            assert_eq!(r, RelayRejection::NoVerdict { reason });
            // Out of gas cannot reproduce on replay (it carries no gas limit), so this bucket must
            // not be a place where a genuinely dead proof can hide forever.
            assert!(r.should_rebuild());
        }
    }

    #[test]
    fn a_solidity_panic_is_distinguished_from_a_revert() {
        let mut data = alloy_sol_types::Panic::SELECTOR.to_vec();
        data.extend_from_slice(&alloy_primitives::U256::from(0x11).abi_encode());
        let r = classify_relay_rejection(Some(&data.into()), NoVerdictReason::ReplayUnreachable);
        assert_eq!(r, RelayRejection::ContractPanic { code: 0x11 });
        assert!(r.should_rebuild());
    }

    /// The property that makes this independent of the L1 client: revert data is read from the
    /// `data` field, never gated on the message wording.
    #[test]
    fn revert_data_is_read_without_consulting_the_message() {
        use alloy_transport::RpcError;

        let invalid_exit_code = Bytes::from(vec![0x1f, 0xcf, 0x91, 0x77]);

        // geth-style: message says "execution reverted".
        let geth = RpcError::<TransportErrorKind>::ErrorResp(alloy_json_rpc::ErrorPayload {
            code: 3,
            message: "execution reverted".into(),
            data: Some(serde_json::value::to_raw_value(&invalid_exit_code).expect("serialisable")),
        });
        assert_eq!(revert_data_of_rpc_error(&geth), Some(invalid_exit_code.clone()));

        // Nethermind-style: same data, but the message never says "revert". alloy's
        // `as_revert_data()` returns None here; ours must not, or a genuine InvalidExitCode verdict
        // would be classified as "no verdict".
        let nethermind = RpcError::<TransportErrorKind>::ErrorResp(alloy_json_rpc::ErrorPayload {
            code: 3,
            message: "VM execution error.".into(),
            data: Some(serde_json::value::to_raw_value(&invalid_exit_code).expect("serialisable")),
        });
        assert_eq!(
            revert_data_of_rpc_error(&nethermind),
            Some(invalid_exit_code),
            "revert data must not depend on the client's message wording"
        );

        // A transport failure carries no error response at all.
        let transport: RpcError<TransportErrorKind> =
            TransportErrorKind::custom_str("tcp connect error");
        assert_eq!(revert_data_of_rpc_error(&transport), None);

        // An error response with no data (e.g. a plain server error) yields nothing.
        let no_data = RpcError::<TransportErrorKind>::ErrorResp(alloy_json_rpc::ErrorPayload {
            code: -32000,
            message: "nonce too low".into(),
            data: None,
        });
        assert_eq!(revert_data_of_rpc_error(&no_data), None);
    }

    #[test]
    fn kind_labels_are_distinct() {
        // These strings end up in logs and metrics, so a duplicate would silently merge two
        // different situations.
        let all = [
            RelayRejection::ProofRejected { selector: [0; 4], name: "x" }.kind(),
            RelayRejection::UnsatisfiableGuard { message: String::new() }.kind(),
            RelayRejection::RebuildableGuard { message: String::new() }.kind(),
            RelayRejection::ContractPanic { code: 0 }.kind(),
            RelayRejection::UnknownRevert { data: Bytes::new() }.kind(),
            RelayRejection::NoVerdict { reason: NoVerdictReason::ReplayTimedOut }.kind(),
        ];
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }
}
