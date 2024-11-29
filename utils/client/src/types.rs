use alloy_consensus::Header;
use alloy_primitives::{B256, Bytes, Address, address};
use alloy_sol_types::sol;
// use op_alloy_rpc_types_engine::OpPayloadAttributes;
use serde::{Deserialize, Serialize};
use op_alloy_rpc_types_engine::OpPayloadAttributes;
use alloy_rpc_types_engine::PayloadAttributes;
const SEQUENCER_FEE_VAULT_ADDRESS: Address = address!("4200000000000000000000000000000000000011");

use crate::boot::BootInfoStruct;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregationInputs {
    pub boot_infos: Vec<BootInfoStruct>,
    pub latest_l1_checkpoint_head: B256,
    pub multi_block_vkey: [u32; 8],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MantleInputs {
    pub prev_block_header: Header,
    pub txs: Vec<Bytes>,
    // pub attributes: String,
}

pub fn prepare_payload(header: Header, txs: Vec<Bytes>) -> OpPayloadAttributes {
    OpPayloadAttributes {
        payload_attributes: PayloadAttributes {
            timestamp: header.timestamp,
            prev_randao: header.mix_hash,
            suggested_fee_recipient: SEQUENCER_FEE_VAULT_ADDRESS,
            parent_beacon_block_root: None,
            withdrawals: Some(Vec::default()),
        },
        transactions: Some(txs),
        no_tx_pool: Some(true),
        gas_limit: Some(header.gas_limit),
        base_fee: None,
    }
}


sol! {
    #[derive(Debug, Serialize, Deserialize)]
    struct AggregationOutputs {
        bytes32 l1Head;
        bytes32 l2PreRoot;
        bytes32 l2PostRoot;
        uint64 l2BlockNumber;
        bytes32 rollupConfigHash;
        bytes32 multiBlockVKey;
    }
}

sol!{
    #[derive(Debug, Serialize, Deserialize)]
    struct MantleOutputs {
        uint64 l2BlockNumber;
    }
}

/// Convert a u32 array to a u8 array. Useful for converting the range vkey to a B256.
pub fn u32_to_u8(input: [u32; 8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    for (i, &value) in input.iter().enumerate() {
        let bytes = value.to_be_bytes();
        output[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
    }
    output
}
