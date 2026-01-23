use crate::witness::BlobData;
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use alloy_consensus::Blob;
use alloy_eips::eip4844::{kzg_to_versioned_hash, IndexedBlobHash};
use alloy_primitives::B256;
use async_trait::async_trait;
use kona_derive::{BlobProvider, BlobProviderError};
use kona_protocol::BlockInfo;
use kzg_rs::get_kzg_settings;
use spin::Mutex;

/// Blob store that shares state across clones.
/// This is crucial for MantleEthereumDataSource which clones the blob provider
/// for both mantle_blob_source and blob_source.
#[derive(Clone, Debug)]
pub struct BlobStore {
    // Shared state across all clones
    versioned_blobs: Arc<Mutex<Vec<(B256, Blob)>>>,
}

impl Default for BlobStore {
    fn default() -> Self {
        Self {
            versioned_blobs: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl From<BlobData> for BlobStore {
    fn from(value: BlobData) -> Self {
        let blobs: Vec<_> =
            value.blobs.iter().map(|b| kzg_rs::Blob::from_slice(&b.0).unwrap()).collect();
        let versioned_blobs = value
            .commitments
            .iter()
            .map(|c| kzg_to_versioned_hash(c.as_slice()))
            .zip(blobs.iter().map(|b| Blob::from(b.0)))
            .rev()
            .collect();

        match kzg_rs::KzgProof::verify_blob_kzg_proof_batch(
            blobs,
            value.commitments,
            value.proofs,
            &get_kzg_settings(),
        ) {
            Ok(true) => {} // Verification passed
            Ok(false) => panic!("KZG proof verification failed: invalid proofs"),
            Err(e) => panic!("KZG proof verification error: {}", e),
        }

        Self { 
            versioned_blobs: Arc::new(Mutex::new(versioned_blobs))
        }
    }
}

#[async_trait]
impl BlobProvider for BlobStore {
    type Error = BlobProviderError;

    async fn get_and_validate_blobs(
        &mut self,
        _: &BlockInfo,
        blob_hashes: &[IndexedBlobHash],
    ) -> Result<Vec<Box<Blob>>, Self::Error> {
        let mut blobs = self.versioned_blobs.lock();
        let mut result = Vec::with_capacity(blob_hashes.len());
        
        for (idx, requested_hash) in blob_hashes.iter().enumerate() {
            // Pop from the end (LIFO order due to .rev() in From impl)
            // All clones share the same state, so pops are visible across clones
            let Some((blob_hash, blob)) = blobs.pop() else {
                return Err(BlobProviderError::Backend(format!(
                    "Insufficient blobs: requested {} but only {} available",
                    blob_hashes.len(),
                    idx
                )));
            };
            
            // Strict hash matching - any mismatch indicates a serious ordering problem
            if requested_hash.hash != blob_hash {
                return Err(BlobProviderError::Backend(format!(
                    "Blob hash mismatch at index {}: expected {:?}, got {:?}",
                    idx, requested_hash.hash, blob_hash
                )));
            }
            
            result.push(Box::new(blob));
        }
        
        Ok(result)
    }
}
