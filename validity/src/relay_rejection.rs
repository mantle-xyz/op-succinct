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
/// stalling the head. `every_guard_on_the_validity_path_is_classified_exactly_once` fails when the
/// contract's set changes, so the new one gets classified deliberately instead of by default.
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
    /// A verdict from the SP1 verifier on the proof bytes: the same bytes will be refused on every
    /// resubmission.
    ProofRejected {
        /// The 4-byte error selector, for logs and for `cast 4byte`.
        selector: [u8; 4],
        /// Solidity name of the error.
        name: &'static str,
    },
    /// The checkpointed L1 head this aggregation was built against is unusable: the contract holds
    /// no hash at that block number, or the block is outside the `blockhash` window.
    ///
    /// Kept separate from [`Self::ProofRejected`] because the proof bytes are not at fault — the
    /// same bytes are accepted once a usable checkpoint exists. It still rebuilds, since
    /// rebuilding is what re-checkpoints; the distinction is what the operator is told to look
    /// at.
    CheckpointUnusable {
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
    /// The contract hit a Solidity `assert` / arithmetic panic.
    ///
    /// Not attributable either way: a panic usually means contract state is inconsistent, but a
    /// verifier decoding malformed proof bytes can also panic. It rebuilds, on the same "cannot
    /// prove it is harmless" reasoning as the rest.
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
            Self::CheckpointUnusable { .. } => "checkpoint_unusable",
            Self::UnsatisfiableGuard { .. } => "unsatisfiable_guard",
            Self::RebuildableGuard { .. } => "rebuildable_guard",
            Self::ContractPanic { .. } => "contract_panic",
            Self::UnknownRevert { .. } => "unknown_revert",
            Self::NoVerdict { .. } => "no_verdict",
        }
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
            ContractError::CustomError(err) => RelayRejection::CheckpointUnusable {
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
/// re-checkpoints. `L1BlockHashNotCheckpointed` means the contract holds no hash at that number;
/// `L1BlockHashNotAvailable` can only come from `checkpointBlockHash` itself, so seeing it on the
/// relay path means the aggregation carried a checkpoint that was never successfully written.
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

    /// The body of `proposeL2Output`, from its signature to the next top-level `function`.
    ///
    /// Scoped because the file carries ~27 `L2OutputOracle: ` messages overall — several of the
    /// guards also appear verbatim in the optimistic-mode overload, so a whole-file search would be
    /// satisfied by a copy that the validity path never reaches.
    fn propose_l2_output_body() -> &'static str {
        let start =
            L2OO_SOURCE.find("function proposeL2Output").expect("proposeL2Output still exists");
        let rest = &L2OO_SOURCE[start..];
        let end = rest[1..].find("\n    function ").map_or(rest.len(), |i| i + 1);
        &rest[..end]
    }

    /// The body of the `whenNotOptimistic` modifier, from its signature to its closing brace.
    fn when_not_optimistic_body() -> &'static str {
        let start = L2OO_SOURCE
            .find("modifier whenNotOptimistic")
            .expect("whenNotOptimistic modifier still exists");
        let rest = &L2OO_SOURCE[start..];
        // Modifiers are short; the first `\n    }` at the contract's indentation level closes it.
        let end = rest.find("\n    }").map_or(rest.len(), |i| i + 1);
        &rest[..end]
    }

    /// Every `L2OutputOracle: …` literal reachable from the validity `proposeL2Output`: the ones in
    /// its own body plus those in the `whenNotOptimistic` modifier it applies.
    ///
    /// Both scopes are scanned in full. An earlier version took only the first two `"`-delimited
    /// pieces after the modifier's signature, which meant a `require` ADDED to the modifier was
    /// invisible — it would fall through to `RebuildableGuard` and burn an aggregation proof every
    /// pass, while this test stayed green.
    fn guards_on_the_validity_path() -> Vec<&'static str> {
        let mut found: Vec<&str> = propose_l2_output_body()
            .split('"')
            .chain(when_not_optimistic_body().split('"'))
            .filter(|s| s.starts_with("L2OutputOracle: "))
            .collect();

        found.sort_unstable();
        found
    }

    #[test]
    fn every_guard_on_the_validity_path_is_classified_exactly_once() {
        // Set equality, not `contains`. Three weaker forms of this check were all satisfiable by a
        // broken list: substring matching accepts a message the contract has since EXTENDED with a
        // suffix; a whole-file search is satisfied by the optimistic-mode overload's copy even
        // after the validity one is reworded; and counting alone cannot see a member
        // removed from `UNSATISFIABLE_GUARDS` (which would silently downgrade that guard to
        // "rebuild", burning an aggregation proof every loop for e.g. optimistic mode).
        let mut expected: Vec<&str> =
            UNSATISFIABLE_GUARDS.iter().copied().chain([REBUILDABLE_GUARD]).collect();
        expected.sort_unstable();

        assert_eq!(
            guards_on_the_validity_path(),
            expected,
            "every require reachable from proposeL2Output must be classified exactly once: add a \
             new one to UNSATISFIABLE_GUARDS if no proof can satisfy it, or leave it out to let it \
             rebuild — and update a reworded one to match the contract verbatim"
        );

        assert!(
            propose_l2_output_body().contains("whenNotOptimistic"),
            "the optimistic-mode guard is no longer applied to proposeL2Output"
        );
    }

    #[test]
    fn unsatisfiable_guards_do_not_rebuild() {
        for guard in UNSATISFIABLE_GUARDS {
            let r = classify_revert_data(&require_revert(guard));
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
        let r = classify_revert_data(&require_revert(REBUILDABLE_GUARD));
        assert_eq!(r, RelayRejection::RebuildableGuard { message: REBUILDABLE_GUARD.to_string() });
        assert!(r.should_rebuild());
    }

    #[test]
    fn an_unlisted_require_rebuilds_rather_than_stalling() {
        // A `require` added upstream must waste a proof, not freeze the contract head.
        let r = classify_revert_data(&require_revert("L2OutputOracle: some future guard"));
        assert!(matches!(r, RelayRejection::RebuildableGuard { .. }));
        assert!(r.should_rebuild());
    }

    #[test]
    fn verifier_verdicts_on_the_proof_bytes_are_recognised_by_selector() {
        // Selectors verified with `cast sig`. These two come from the SP1 verifier and bubble up
        // through `verifyProof`, so they really are statements about the proof bytes.
        let cases: [([u8; 4], &str); 2] = [
            ([0x09, 0xbd, 0xe3, 0x39], "InvalidProof"),
            ([0x1f, 0xcf, 0x91, 0x77], "InvalidExitCode"),
        ];

        for (selector, name) in cases {
            let r = classify_revert_data(&selector_only(selector));
            assert_eq!(
                r,
                RelayRejection::ProofRejected { selector, name },
                "selector {selector:?}"
            );
            assert!(r.should_rebuild());
        }
    }

    #[test]
    fn oracle_checkpoint_errors_are_not_proof_verdicts() {
        // Both mean the checkpointed L1 head is unusable. They rebuild — that is what
        // re-checkpoints — but they must NOT be reported as a verdict on the proof: the
        // same bytes are accepted once a usable checkpoint exists, and telling an operator
        // to go check the aggregation vkey would send them somewhere with nothing to find.
        // A lagging L1 endpoint answering `historicBlockHashes` produces the first of these
        // for a checkpoint that does exist.
        let cases: [([u8; 4], &str); 2] = [
            ([0x22, 0xaa, 0x3a, 0x98], "L1BlockHashNotCheckpointed"),
            ([0x84, 0xc0, 0x68, 0x64], "L1BlockHashNotAvailable"),
        ];

        for (selector, name) in cases {
            let r = classify_revert_data(&selector_only(selector));
            assert_eq!(
                r,
                RelayRejection::CheckpointUnusable { selector, name },
                "selector {selector:?}"
            );
            assert!(r.should_rebuild(), "rebuilding is what re-checkpoints");
        }
    }

    #[test]
    fn an_unrecognised_selector_rebuilds_and_keeps_the_data() {
        // The contract or verifier was upgraded. Rebuilding wastes a proof; not rebuilding would
        // freeze the head. The data is preserved so the log can name the selector.
        let data = selector_only([0x1a, 0x2b, 0x3c, 0x4d]);
        let r = classify_revert_data(&data);
        assert_eq!(r, RelayRejection::UnknownRevert { data });
        assert!(r.should_rebuild());
    }

    #[test]
    fn a_missing_verdict_rebuilds_whatever_the_reason() {
        for reason in [
            NoVerdictReason::ReplayDidNotRevert,
            NoVerdictReason::ReplayUnreachable,
            NoVerdictReason::ReplayTimedOut,
        ] {
            // Out of gas cannot reproduce on replay (it carries no gas limit), so this bucket must
            // not be a place where a genuinely dead proof can hide forever.
            assert!(
                RelayRejection::NoVerdict { reason: reason.clone() }.should_rebuild(),
                "{reason:?} must rebuild"
            );
        }
    }

    #[test]
    fn a_solidity_panic_is_distinguished_from_a_revert() {
        let mut data = alloy_sol_types::Panic::SELECTOR.to_vec();
        data.extend_from_slice(&alloy_primitives::U256::from(0x11).abi_encode());
        let r = classify_revert_data(&data.into());
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

    /// `revert_data_of_anyhow` is the production entry point for the common path — alloy's gas
    /// filler rejects a deterministic revert at `eth_estimateGas`, so the data arrives on the send
    /// error rather than on a receipt. It had no test at all, and replacing its body with `None`
    /// silently returns the proposer to the pre-fix behaviour (resubmit the same bytes forever).
    #[test]
    fn anyhow_context_layers_do_not_hide_the_revert_data() {
        use anyhow::Context;

        let data = Bytes::from(vec![0x1f, 0xcf, 0x91, 0x77]);
        let payload = || alloy_json_rpc::ErrorPayload {
            code: 3,
            message: "execution reverted".into(),
            data: Some(serde_json::value::to_raw_value(&data).expect("serialisable")),
        };

        // One layer, matching `utils/signer/src/lib.rs`'s `.context("Failed to send
        // transaction")?`.
        let one = Err::<(), _>(RpcError::<TransportErrorKind>::ErrorResp(payload()))
            .context("Failed to send transaction")
            .unwrap_err();
        assert_eq!(revert_data_of_anyhow(&one), Some(data.clone()));

        // Nested layers must also resolve, so a caller adding its own context cannot blind the
        // classifier.
        let two = Err::<(), _>(RpcError::<TransportErrorKind>::ErrorResp(payload()))
            .context("Failed to send transaction")
            .context("relaying aggregation proof")
            .unwrap_err();
        assert_eq!(revert_data_of_anyhow(&two), Some(data));

        // No revert data: nonce, funds, a dead RPC. The caller must retry unchanged rather than
        // spend an aggregation proof.
        assert_eq!(revert_data_of_anyhow(&anyhow::anyhow!("nonce too low")), None);
    }

    /// Pins a real limitation as an assertion rather than a comment.
    ///
    /// `anyhow`'s `downcast_ref` walks its own context layers, NOT `std::error::Error::source()`,
    /// so an `RpcError` nested inside another concrete error type is invisible here.
    ///
    /// Note what the fix would NOT be. Walking `source()` does not recover this case either:
    /// `PendingTransactionError::TransportError` is `#[error(transparent)]`, so the `source()`
    /// thiserror generates forwards *past* the `RpcError` to whatever is inside it — the
    /// `RpcError` itself never appears on the chain. Reading data out of that shape needs an
    /// explicit `downcast_ref::<PendingTransactionError>()` per wrapper type.
    ///
    /// None of this bites today: the relay path's errors come from `fill` / `send_transaction`,
    /// which propagate `RpcError` directly, and `PendingTransactionError` only arises from
    /// `get_receipt()` — by which point the transaction is mined and carries no revert data.
    #[test]
    fn revert_data_is_invisible_through_a_concrete_error_wrapper() {
        let data = Bytes::from(vec![0x1f, 0xcf, 0x91, 0x77]);
        let inner = RpcError::<TransportErrorKind>::ErrorResp(alloy_json_rpc::ErrorPayload {
            code: 3,
            message: "execution reverted".into(),
            data: Some(serde_json::value::to_raw_value(&data).expect("serialisable")),
        });

        let wrapped =
            anyhow::Error::new(alloy_provider::PendingTransactionError::TransportError(inner));
        assert_eq!(
            revert_data_of_anyhow(&wrapped),
            None,
            "known limitation: anyhow's downcast does not traverse Error::source()"
        );
    }

    #[test]
    fn kind_labels_are_distinct() {
        // These strings end up in logs and metrics, so a duplicate would silently merge two
        // different situations.
        let all = [
            RelayRejection::ProofRejected { selector: [0; 4], name: "x" }.kind(),
            RelayRejection::CheckpointUnusable { selector: [0; 4], name: "x" }.kind(),
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
