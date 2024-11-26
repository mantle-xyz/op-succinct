//! Contains the concrete implementation of the [L2ChainProvider] trait for the client program.
use crate::block_on;
use anyhow::{anyhow, Result};
use kona_client::HintType;
use alloc::sync::Arc;
use alloy_consensus::Header;
use alloy_primitives::{Address, Bytes, B256};
use alloy_rlp::Decodable;
use kona_mpt::{TrieHinter, TrieProvider};
use kona_preimage::{CommsClient, PreimageKey, PreimageKeyType};

/// The oracle-backed L2 chain provider for the client program.
#[derive(Debug, Clone)]
pub struct OracleL2ChainProvider<T: CommsClient> {
    /// The preimage oracle client.
    oracle: Arc<T>,
}

impl<T: CommsClient> OracleL2ChainProvider<T> {
    /// Creates a new [OracleL2ChainProvider] with the given boot information and oracle client.
    pub fn new(oracle: Arc<T>) -> Self {
        Self { oracle }
    }
}

impl<T: CommsClient> TrieProvider for OracleL2ChainProvider<T> {
    type Error = anyhow::Error;

    fn trie_node_preimage(&self, key: B256) -> Result<Bytes> {
        // On L2, trie node preimages are stored as keccak preimage types in the oracle. We assume
        // that a hint for these preimages has already been sent, prior to this call.
        block_on(async move {
            Ok(self
                .oracle
                .get(PreimageKey::new(*key, PreimageKeyType::Keccak256))
                .await?
                .into())
        })
    }

    fn bytecode_by_hash(&self, hash: B256) -> Result<Bytes> {
        // Fetch the bytecode preimage from the caching oracle.
        block_on(async move {
            self.oracle
                .write(&HintType::L2Code.encode_with(&[hash.as_ref()]))
                .await?;

            Ok(self
                .oracle
                .get(PreimageKey::new(*hash, PreimageKeyType::Keccak256))
                .await?
                .into())
        })
    }

    fn header_by_hash(&self, hash: B256) -> Result<Header> {
        // Fetch the header from the caching oracle.
        block_on(async move {
            self.oracle
                .write(&HintType::L2BlockHeader.encode_with(&[hash.as_ref()]))
                .await?;

            let header_bytes = self
                .oracle
                .get(PreimageKey::new(*hash, PreimageKeyType::Keccak256))
                .await?;
            Header::decode(&mut header_bytes.as_slice())
                .map_err(|e| anyhow!("Failed to RLP decode Header: {e}"))
        })
    }
}

impl<T: CommsClient> TrieHinter for OracleL2ChainProvider<T> {
    type Error = anyhow::Error;

    fn hint_trie_node(&self, hash: B256) -> Result<()> {
        block_on(async move {
            Ok(self
                .oracle
                .write(&HintType::L2StateNode.encode_with(&[hash.as_slice()]))
                .await?)
        })
    }

    fn hint_account_proof(&self, address: Address, block_number: u64) -> Result<()> {
        block_on(async move {
            Ok(self
                .oracle
                .write(
                    &HintType::L2AccountProof
                        .encode_with(&[block_number.to_be_bytes().as_ref(), address.as_slice()]),
                )
                .await?)
        })
    }

    fn hint_storage_proof(
        &self,
        address: alloy_primitives::Address,
        slot: alloy_primitives::U256,
        block_number: u64,
    ) -> Result<()> {
        block_on(async move {
            Ok(self
                .oracle
                .write(&HintType::L2AccountStorageProof.encode_with(&[
                    block_number.to_be_bytes().as_ref(),
                    address.as_slice(),
                    slot.to_be_bytes::<32>().as_ref(),
                ]))
                .await?)
        })
    }
}

