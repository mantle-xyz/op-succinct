//! Compatibility types for older op-node versions
//!
//! This module provides compatibility structures for op-node responses that may
//! be missing newer fields like `cross_unsafe_l2` and `local_safe_l2` in `sync_status`.

use alloy_primitives::B256;
use kona_protocol::L2BlockInfo;
use serde::{Deserialize, Serialize};

/// Compatible SyncStatus that supports older op-node versions.
///
/// Older versions may not include `cross_unsafe_l2` and `local_safe_l2` fields,
/// so these are marked as Option to allow deserialization from both old and new formats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CompatSyncStatus {
    /// The current L1 block.
    pub current_l1: BlockInfoCompat,
    /// The current L1 finalized block.
    pub current_l1_finalized: BlockInfoCompat,
    /// The L1 head block ref.
    pub head_l1: BlockInfoCompat,
    /// The L1 safe head block ref.
    pub safe_l1: BlockInfoCompat,
    /// The finalized L1 block ref.
    pub finalized_l1: BlockInfoCompat,
    /// The unsafe L2 block ref.
    pub unsafe_l2: L2BlockInfo,
    /// The safe L2 block ref.
    pub safe_l2: L2BlockInfo,
    /// The finalized L2 block ref.
    pub finalized_l2: L2BlockInfo,
    /// Cross unsafe L2 block ref (optional for backward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_unsafe_l2: Option<L2BlockInfo>,
    /// Local safe L2 block ref (optional for backward compatibility).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_safe_l2: Option<L2BlockInfo>,
    /// Queued unsafe L2 block ref (optional, may be present in some versions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_unsafe_l2: Option<L2BlockInfo>,
    /// Engine sync target (optional, may be present in some versions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_sync_target: Option<L2BlockInfo>,
}

/// Compatible BlockInfo structure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockInfoCompat {
    /// The block hash
    pub hash: B256,
    /// The block number
    pub number: u64,
    /// The parent block hash
    #[serde(rename = "parentHash")]
    pub parent_hash: B256,
    /// The block timestamp
    pub timestamp: u64,
}

/// Compatible OutputResponse that supports older op-node versions.
///
/// This structure can deserialize from both old and new op-node response formats.
/// Only the fields we actually use are required; others are optional for compatibility.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatOutputResponse {
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
    /// The status of the node sync (compatible with old and new formats).
    pub sync_status: CompatSyncStatus,
}
