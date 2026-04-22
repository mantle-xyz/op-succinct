use anyhow::Result;
use async_trait::async_trait;
use kona_proof::l1::OracleBlobProvider;
use op_succinct_client_utils::witness::DefaultWitnessData;
use op_succinct_ethereum_client_utils::executor::ETHDAWitnessExecutor;
use op_succinct_host_utils::witness_generation::{
    online_blob_store::OnlineBlobStore, preimage_witness_collector::PreimageWitnessCollector,
    DefaultOracleBase, WitnessGenerator,
};
use rkyv::to_bytes;
use sp1_sdk::SP1Stdin;

type WitnessExecutor = ETHDAWitnessExecutor<
    PreimageWitnessCollector<DefaultOracleBase>,
    OnlineBlobStore<OracleBlobProvider<DefaultOracleBase>>,
>;

pub struct ETHDAWitnessGenerator {
    pub executor: WitnessExecutor,
}

#[async_trait]
impl WitnessGenerator for ETHDAWitnessGenerator {
    type WitnessData = DefaultWitnessData;
    type WitnessExecutor = WitnessExecutor;

    fn get_executor(&self) -> &Self::WitnessExecutor {
        &self.executor
    }

    fn get_sp1_stdin(&self, witness: Self::WitnessData) -> Result<SP1Stdin> {
        // Debug: 打印收集到的 preimage key 类型分布
        let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for key in witness.preimage_store.preimage_map.keys() {
            *counts.entry(format!("{:?}", key.key_type())).or_insert(0) += 1;
        }
        eprintln!("[Debug] PreimageStore total keys: {}", witness.preimage_store.preimage_map.len());
        eprintln!("[Debug] PreimageStore key types: {:?}", counts);
        eprintln!("[Debug] BlobData blobs count: {}", witness.blob_data.blobs.len());

        let mut stdin = SP1Stdin::new();
        let buffer = to_bytes::<rkyv::rancor::Error>(&witness)?;
        stdin.write_slice(&buffer);
        Ok(stdin)
    }
}
