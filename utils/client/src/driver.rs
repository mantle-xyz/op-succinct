//! Contains the [MultiBlockDerivationDriver] struct, which handles the [L2PayloadAttributes]
//! derivation process.
//!
//! [L2PayloadAttributes]: kona_derive::types::L2PayloadAttributes

use alloy_consensus::{Header, Sealed};
use anyhow::{anyhow, Result};
use core::fmt::Debug;

use op_alloy_protocol::{BlockInfo, L2BlockInfo};
use op_alloy_rpc_types_engine::OpAttributesWithParent;


/// The [MultiBlockDerivationDriver] struct is responsible for handling the [L2PayloadAttributes]
/// derivation process.
///
/// It contains an inner [OraclePipeline] that is used to derive the attributes, backed by
/// oracle-based data sources.
///
/// [L2PayloadAttributes]: kona_derive::types::L2PayloadAttributes
#[derive(Debug)]
pub struct MultiBlockDerivationDriver {
    /// The current L2 safe head.
    pub l2_safe_head: L2BlockInfo,
    /// The header of the L2 safe head.
    pub l2_safe_head_header: Sealed<Header>,
    /// The inner pipeline.
    // pub pipeline: OraclePipeline<O>,
    /// The block number of the final L2 block being claimed.
    pub l2_claim_block: u64,
}

impl MultiBlockDerivationDriver {
    /// Consumes self and returns the owned [Header] of the current L2 safe head.
    pub fn clone_l2_safe_head_header(&self) -> Sealed<Header> {
        self.l2_safe_head_header.clone()
    }

    /// Creates a new [MultiBlockDerivationDriver] with the given configuration, blob provider, and
    /// chain providers.
    ///
    /// ## Takes
    /// - `cfg`: The rollup configuration.
    /// - `blob_provider`: The blob provider.
    /// - `chain_provider`: The L1 chain provider.
    /// - `l2_chain_provider`: The L2 chain provider.
    ///
    /// ## Returns
    /// - A new [MultiBlockDerivationDriver] instance.
    pub async fn new(
        // mut l2_chain_provider: MultiblockOracleL2ChainProvider<O>,
    ) -> Result<Self> {
        unimplemented!()
    }

    pub fn update_safe_head(
        &mut self,
        new_safe_head: L2BlockInfo,
        new_safe_head_header: Sealed<Header>,
    ) {
        self.l2_safe_head = new_safe_head;
        self.l2_safe_head_header = new_safe_head_header;
    }

    /// Produces the disputed [OpAttributesWithParent] payload, directly from the pipeline.
    pub async fn produce_payload(&mut self) -> Result<OpAttributesWithParent> {
        unimplemented!()
    }
}
