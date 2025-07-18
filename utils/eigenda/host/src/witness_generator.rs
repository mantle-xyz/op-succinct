use anyhow::Result;
use async_trait::async_trait;
use kona_proof::l1::OracleBlobProvider;
use op_succinct_client_utils::witness::DefaultWitnessData;
use op_succinct_eigenda_client_utils::executor::EigendaDAWitnessExecutor;
use op_succinct_host_utils::witness_generation::{
    online_blob_store::OnlineBlobStore, preimage_witness_collector::PreimageWitnessCollector,
    DefaultOracleBase, WitnessGenerator,
};
use rkyv::to_bytes;
use sp1_sdk::SP1Stdin;

type WitnessExecutor = EigendaDAWitnessExecutor<
    PreimageWitnessCollector<DefaultOracleBase>,
    OnlineBlobStore<OracleBlobProvider<DefaultOracleBase>>,
>;

pub struct EigendaDAWitnessGenerator {
    pub executor: WitnessExecutor,
}

#[async_trait]
impl WitnessGenerator for EigendaDAWitnessGenerator {
    type WitnessData = DefaultWitnessData;
    type WitnessExecutor = WitnessExecutor;

    fn get_executor(&self) -> &Self::WitnessExecutor {
        &self.executor
    }

    fn get_sp1_stdin(&self, witness: Self::WitnessData) -> Result<SP1Stdin> {
        let mut stdin = SP1Stdin::new();
        let buffer = to_bytes::<rkyv::rancor::Error>(&witness)?;
        stdin.write_slice(&buffer);
        Ok(stdin)
    }
}
