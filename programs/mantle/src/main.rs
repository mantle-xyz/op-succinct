//! A program to verify a Optimism L2 block STF in the zkVM.
//!
//! This binary contains the client program for executing the Optimism rollup state transition
//! across a range of blocks, which can be used to generate an on chain validity proof. Depending on
//! the compilation pipeline, it will compile to be run either in native mode or in zkVM mode. In
//! native mode, the data for verifying the batch validity is fetched from RPC, while in zkVM mode,
//! the data is supplied by the host binary to the verifiable program.

#![cfg_attr(target_os = "zkvm", no_main)]

extern crate alloc;

use alloc::sync::Arc;

use alloy_consensus::{BlockBody, Header, Sealable, Sealed};
// use cfg_if::cfg_if;
use kona_executor::StatelessL2BlockExecutor;
use op_alloy_genesis::rollup::RollupConfig;
use op_succinct_client_utils::{
    mantle_provider::OracleL2ChainProvider, precompiles::zkvm_handle_register,
    InMemoryOracle,
};
use op_succinct_client_utils::types::{MantleInputs, prepare_payload, MantleOutputs};
sp1_zkvm::entrypoint!(main);

fn main() {
    #[cfg(feature = "tracing-subscriber")]
    {
        use anyhow::anyhow;
        use tracing::Level;

        let subscriber = tracing_subscriber::fmt()
            .with_max_level(Level::INFO)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| anyhow!(e))
            .unwrap();
    }

    op_succinct_client_utils::block_on(async move {
        ////////////////////////////////////////////////////////////////
        //                          PROLOGUE                          //
        ////////////////////////////////////////////////////////////////

        println!("cycle-tracker-start: boot-load");
        let mantle_inputs = sp1_zkvm::io::read::<MantleInputs>();
        let prev_block_header = mantle_inputs.prev_block_header;
        let txs = mantle_inputs.txs;
        let attributes = prepare_payload(prev_block_header.clone(), txs);
        // sp1_zkvm::io::commit(&attributes);
        sp1_zkvm::io::commit::<MantleOutputs>(&MantleOutputs {
            l2BlockNumber: prev_block_header.number + 1,
        });
        println!("cycle-tracker-end: boot-load");

        println!("cycle-tracker-start: oracle-load");
        let in_memory_oracle_bytes: Vec<u8> = sp1_zkvm::io::read_vec();
        let oracle = Arc::new(InMemoryOracle::from_raw_bytes(in_memory_oracle_bytes));
        println!("cycle-tracker-end: oracle-load");

        println!("cycle-tracker-report-start: oracle-verify");
        // oracle.verify().expect("key value verification failed");
        println!("cycle-tracker-report-end: oracle-verify");

        let mantle_provider = OracleL2ChainProvider::new(oracle.clone());
        let config = mock_rollup_config();
        ////////////////////////////////////////////////////////////////
        //                   DERIVATION & EXECUTION                   //
        ////////////////////////////////////////////////////////////////

        println!("cycle-tracker-start: execution-instantiation");
        let mut executor = StatelessL2BlockExecutor::builder(
            &config,
            mantle_provider.clone(),
            mantle_provider.clone(),
        )
            .with_parent_header(prev_block_header.seal_slow())
            .with_handle_register(zkvm_handle_register)
            .build();
        println!("cycle-tracker-end: execution-instantiation");

        println!("cycle-tracker-report-start: block-execution");
        let new_block_header = executor.execute_payload(attributes.clone()).unwrap();
        println!("new block header: {:?}", new_block_header);
        println!("cycle-tracker-report-end: block-execution");

        // println!("cycle-tracker-start: output-root");
        // let output_root = executor.compute_output_root().unwrap();
        // println!("cycle-tracker-end: output-root");

        // println!("Validated derivation and STF. Output Root: {}", output_root);
    });
}

fn mock_rollup_config() -> RollupConfig {
    RollupConfig {
        l2_chain_id: 5000,
        regolith_time: Some(0),
        shanghai_time: Some(0),
        ..Default::default()
    }
}
