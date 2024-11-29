pub mod block_range;
pub mod fetcher;
pub mod helpers;
pub mod rollup_config;
pub mod stats;
pub mod witnessgen;

use alloy::sol;
use alloy_consensus::Header;
use alloy_primitives::{B256, Address, Bytes};
use kona_host::{
    kv::{DiskKeyValueStore, MemoryKeyValueStore},
    HostCli,
};
use op_alloy_genesis::RollupConfig;
use op_succinct_client_utils::{
    boot::BootInfoStruct, types::AggregationInputs, BootInfoWithBytesConfig, InMemoryOracle,
    types::MantleInputs,
};
use sp1_sdk::{HashableKey, SP1Proof, SP1Stdin};
use std::{fs::File, io::Read};
use alloy::{
    eips::BlockNumberOrTag,
    providers::{Provider, ProviderBuilder as MantleProviderBuilder},
    rpc::types::Header as RpcHeader,
};
use alloy::network::primitives::{BlockTransactionsKind, BlockTransactions};
use alloy_eips::eip2718::Encodable2718;
use alloy_rlp::Encodable;
use alloy_rpc_types::Block;
use op_alloy_network::Optimism;


use anyhow::{anyhow, Result};
// use op_alloy_network::Optimism;
use rkyv::{
    ser::{
        serializers::{AlignedSerializer, CompositeSerializer, HeapScratch, SharedSerializeMap},
        Serializer,
    },
    AlignedVec,
};

sol! {
    #[allow(missing_docs)]
    #[sol(rpc)]
    contract L2OutputOracle {
        bytes32 public aggregationVkey;
        bytes32 public rangeVkeyCommitment;
        bytes32 public rollupConfigHash;

        function updateAggregationVKey(bytes32 _aggregationVKey) external onlyOwner;

        function updateRangeVkeyCommitment(bytes32 _rangeVkeyCommitment) external onlyOwner;
    }
}

pub enum ProgramType {
    Single,
    Multi,
}

sol! {
    struct L2Output {
        uint64 zero;
        bytes32 l2_state_root;
        bytes32 l2_storage_hash;
        bytes32 l2_claim_hash;
    }
}
pub async fn get_mantle_proof_stdin(block_number: u64) -> Result<SP1Stdin> {
    let mut stdin = SP1Stdin::new();
    let url = "";
    let client = MantleProviderBuilder::new()
        .network::<Optimism>()
        .on_http(url.parse().unwrap());
    let prev_block = client
        .get_block_by_number(BlockNumberOrTag::from(block_number - 1), BlockTransactionsKind::Hashes)
        .await
        .unwrap()
        .ok_or(anyhow::anyhow!("Block not found"))
        .unwrap();

    let prev_block_header = convert_header(prev_block.header);

    let Block { transactions, .. } = client
        .get_block_by_number(BlockNumberOrTag::from(block_number), BlockTransactionsKind::Hashes)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch block: {e}"))?
        .ok_or(anyhow::anyhow!("Block not found."))?;

    let txs = match transactions {
        BlockTransactions::Hashes(transactions) => {
            let mut encoded_transactions = Vec::with_capacity(transactions.len());
            for tx_hash in transactions {
                let tx = client
                    .client()
                    .request::<&[B256; 1], Bytes>("debug_getRawTransaction", &[tx_hash])
                    .await
                    .map_err(|e| anyhow::anyhow!("Error fetching transaction: {e}"))?;
                encoded_transactions.push(tx);
            }
            encoded_transactions
        }
        _ => anyhow::bail!("Only BlockTransactions::Hashes are supported."),
    };

    let prev_block_header_string = serde_json::to_string(&prev_block_header).unwrap();
    // println!("prev_block_header: {:?}", prev_block_header);
    stdin.write(&prev_block_header_string);
    stdin.write_vec(serde_cbor::to_vec(&txs).unwrap());

    // ------ stdin attributes done -------

    let input = std::fs::read(format!("cache-{}.bin", block_number).as_str()).unwrap();
    stdin.write_slice(&input);
    // ------ stdin input done -------

    Ok(stdin)
}

/// Get the stdin to generate a proof for the given L2 claim.
pub fn get_proof_stdin(host_cli: &HostCli) -> Result<SP1Stdin> {
    let mut stdin = SP1Stdin::new();

    // Read the rollup config.
    let mut rollup_config_file = File::open(host_cli.rollup_config_path.as_ref().unwrap())?;
    let mut rollup_config_bytes = Vec::new();
    rollup_config_file.read_to_end(&mut rollup_config_bytes)?;

    let ser_config = std::fs::read_to_string(host_cli.rollup_config_path.as_ref().unwrap())?;
    let rollup_config: RollupConfig = serde_json::from_str(&ser_config)?;

    let boot_info = BootInfoWithBytesConfig {
        l1_head: host_cli.l1_head,
        l2_output_root: host_cli.agreed_l2_output_root,
        l2_claim: host_cli.claimed_l2_output_root,
        l2_claim_block: host_cli.claimed_l2_block_number,
        chain_id: rollup_config.l2_chain_id,
        rollup_config_bytes,
    };
    stdin.write(&boot_info);

    // Get the disk KV store.
    let disk_kv_store = DiskKeyValueStore::new(host_cli.data_dir.clone().unwrap());

    // Convert the disk KV store to a memory KV store.
    let mem_kv_store: MemoryKeyValueStore = disk_kv_store.try_into().map_err(|_| {
        anyhow::anyhow!("Failed to convert DiskKeyValueStore to MemoryKeyValueStore")
    })?;

    let mut serializer = CompositeSerializer::new(
        AlignedSerializer::new(AlignedVec::new()),
        // Note: This value corresponds to the size of the heap needed to serialize the KV store.
        // Increase this value if we start running into serialization issues.
        HeapScratch::<67108864>::new(),
        SharedSerializeMap::new(),
    );
    // Serialize the underlying KV store.
    serializer.serialize_value(&InMemoryOracle::from_b256_hashmap(mem_kv_store.store))?;

    let buffer = serializer.into_serializer().into_inner();
    let kv_store_bytes = buffer.into_vec();
    stdin.write_slice(&kv_store_bytes);

    Ok(stdin)
}

/// Get the stdin for the aggregation proof.
pub fn get_agg_proof_stdin(
    proofs: Vec<SP1Proof>,
    boot_infos: Vec<BootInfoStruct>,
    headers: Vec<Header>,
    multi_block_vkey: &sp1_sdk::SP1VerifyingKey,
    latest_checkpoint_head: B256,
) -> Result<SP1Stdin> {
    let mut stdin = SP1Stdin::new();
    for proof in proofs {
        let SP1Proof::Compressed(compressed_proof) = proof else {
            panic!();
        };
        stdin.write_proof(*compressed_proof, multi_block_vkey.vk.clone());
    }

    // Write the aggregation inputs to the stdin.
    stdin.write(&AggregationInputs {
        boot_infos,
        latest_l1_checkpoint_head: latest_checkpoint_head,
        multi_block_vkey: multi_block_vkey.hash_u32(),
    });
    // The headers have issues serializing with bincode, so use serde_json instead.
    let headers_bytes = serde_cbor::to_vec(&headers).unwrap();
    stdin.write_vec(headers_bytes);

    Ok(stdin)
}


fn convert_header(header: RpcHeader) -> Header {
    Header {
        parent_hash: header.parent_hash,
        ommers_hash: header.ommers_hash,
        beneficiary: header.beneficiary,
        state_root: header.state_root,
        transactions_root: header.transactions_root,
        receipts_root: header.receipts_root,
        logs_bloom: header.logs_bloom,
        difficulty: header.difficulty,
        number: header.number,
        gas_limit: header.gas_limit,
        gas_used: header.gas_used,
        timestamp: header.timestamp,
        extra_data: header.extra_data.clone(),
        mix_hash: header.mix_hash,
        nonce: header.nonce,
        base_fee_per_gas: header.base_fee_per_gas,
        withdrawals_root: header.withdrawals_root,
        blob_gas_used: header.blob_gas_used,
        excess_blob_gas: header.excess_blob_gas,
        parent_beacon_block_root: header.parent_beacon_block_root,
        requests_hash: header.requests_hash,
    }
}
