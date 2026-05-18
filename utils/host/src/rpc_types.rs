//! RPC Types for Optimism Rollup
//!
//! These types are copied from kona-rpc to avoid bringing in rollup-boost dependencies
//! which cause transitive alloy version conflicts.
//!
//! [MANTLE] `SyncStatus` here differs from kona-rpc by one detail: `cross_unsafe_l2` and
//! `local_safe_l2` are wrapped in `Option<L2BlockInfo>` and given `#[serde(default)]`
//! so the host can deserialize `optimism_outputAtBlock` responses from Mantle production
//! op-node, which predates OP-Stack Interop and does not emit those two fields. This is
//! the same intent as origin/main's `5efd6ead feat: add compatibility layer for older
//! op-node versions`, but folded into this existing local-schema module rather than
//! added as a parallel `compat.rs` — `rpc_types.rs` already exists to host a relaxed
//! copy of kona-rpc, so the natural move is to relax the field shape here. Drop the
//! Option wrappers when Mantle ops bumps prod op-node past Interop.

use alloy_eips::BlockNumHash;
use alloy_primitives::B256;
use kona_protocol::{BlockInfo, L2BlockInfo};
use serde::{Deserialize, Serialize};

/// The [`SyncStatus`][ss] of an Optimism Rollup Node, relaxed for older op-node versions.
///
/// `cross_unsafe_l2` and `local_safe_l2` are post-Interop additions; pre-Interop
/// op-node deployments (incl. Mantle production at time of writing) omit them.
///
/// [ss]: https://github.com/ethereum-optimism/optimism/blob/develop/op-service/eth/sync_status.go#L5
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SyncStatus {
    /// The current L1 block.
    pub current_l1: BlockInfo,
    /// The current L1 finalized block (legacy / deprecated; mirrors `finalized_l1`).
    pub current_l1_finalized: BlockInfo,
    /// The L1 head block ref.
    pub head_l1: BlockInfo,
    /// The L1 safe head block ref.
    pub safe_l1: BlockInfo,
    /// The finalized L1 block ref.
    pub finalized_l1: BlockInfo,
    /// The unsafe L2 block ref.
    pub unsafe_l2: L2BlockInfo,
    /// The safe L2 block ref.
    pub safe_l2: L2BlockInfo,
    /// The finalized L2 block ref.
    pub finalized_l2: L2BlockInfo,
    /// Cross-unsafe L2 block ref (post-Interop; optional for backward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_unsafe_l2: Option<L2BlockInfo>,
    /// Local safe L2 block ref (post-Interop; optional for backward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_safe_l2: Option<L2BlockInfo>,
}

/// An [output response][or] for Optimism Rollup.
///
/// [or]: https://github.com/ethereum-optimism/optimism/blob/f20b92d3eb379355c876502c4f28e72a91ab902f/op-service/eth/output.go#L10-L17
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputResponse {
    /// The output version.
    pub version: B256,
    /// The output root hash.
    pub output_root: B256,
    /// A reference to the L2 block.
    pub block_ref: L2BlockInfo,
    /// The withdrawal storage root.
    pub withdrawal_storage_root: B256,
    /// The state root.
    pub state_root: B256,
    /// The status of the node sync.
    pub sync_status: SyncStatus,
}

/// The safe head response.
///
/// <https://github.com/ethereum-optimism/optimism/blob/77c91d09eaa44d2c53bec60eb89c5c55737bc325/op-service/eth/output.go#L19-L22>
/// Note: the optimism "eth.BlockID" type is number,hash <https://github.com/ethereum-optimism/optimism/blob/77c91d09eaa44d2c53bec60eb89c5c55737bc325/op-service/eth/id.go#L10-L13>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeHeadResponse {
    /// The L1 block.
    pub l1_block: BlockNumHash,
    /// The safe head.
    pub safe_head: BlockNumHash,
}
