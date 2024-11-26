use alloy_primitives::{hex, Address, B256};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use log::info;
use op_succinct_client_utils::{
    boot::{hash_rollup_config, BootInfoStruct},
    types::u32_to_u8,
};
use op_succinct_host_utils::{
    fetcher::{CacheMode, OPSuccinctDataFetcher},
    get_agg_proof_stdin, get_proof_stdin,
    get_mantle_proof_stdin,
    witnessgen::WitnessGenExecutor,
    L2OutputOracle, ProgramType,
};
use op_succinct_proposer::{
    AggProofRequest, ContractConfig, ProofResponse, ProofStatus, MantleSingleProofRequest,
    UnclaimDescription, ValidateConfigRequest, ValidateConfigResponse,
};
use sp1_sdk::{
    network::{
        client::NetworkClient,
        proto::network::{ProofMode, ProofStatus as SP1ProofStatus},
    },
    utils, HashableKey, NetworkProverV1, ProverClient, SP1Proof, SP1ProofWithPublicValues,
};
use std::{env, str::FromStr, time::Duration};
use tower_http::limit::RequestBodyLimitLayer;


// pub const MULTI_BLOCK_ELF: &[u8] = include_bytes!("../../../elf/range-elf");
pub const AGG_ELF: &[u8] = include_bytes!("../../../elf/aggregation-elf");

pub const MANTLE_ELF: &[u8] = include_bytes!("../../../elf/mantle-elf");

#[tokio::main]
async fn main() {
    utils::setup_logger();

    dotenv::dotenv().ok();

    env::set_var("SKIP_SIMULATION", "true");

    let prover = ProverClient::new();
    // println!("setting up aggregation elf");
    // let (_, agg_vk) = prover.setup(AGG_ELF);
    // println!("setting up aggregation elf done");
    // let (_, range_vk) = prover.setup(MULTI_BLOCK_ELF);
    println!("setting up mantle elf");
    let (_, mantle_vk) = prover.setup(MANTLE_ELF);
    println!("setting up mantle elf done");
    // let multi_block_vkey_u8 = u32_to_u8(range_vk.vk.hash_u32());
    // let range_vkey_commitment = B256::from(multi_block_vkey_u8);
    // println!("setting up mantle vkey hash");
    // let mantle_vkey_hash = u32_to_u8(mantle_vk.vk.hash_u32());
    // println!("setting up mantle vkey hash done");
    // println!("setting up mantle vkey commitment");
    // let mantle_vkey_commitment = B256::from(mantle_vkey_hash);
    // println!("setting up mantle vkey commitment done");
    // println!("setting up agg vkey hash");
    // // let agg_vkey_hash = B256::from_str(&agg_vk.bytes32()).unwrap();
    // println!("setting up agg vkey hash done");

    // let fetcher = OPSuccinctDataFetcher::new_with_rollup_config()
    //     .await
    //     .unwrap();
    // Note: The rollup config hash never changes for a given chain, so we can just hash it once at
    // server start-up. The only time a rollup config changes is typically when a new version of the
    // [`RollupConfig`] is released from `op-alloy`.
    // let rollup_config_hash = hash_rollup_config(fetcher.rollup_config.as_ref().unwrap());

    // Initialize global hashes.
    let global_hashes = ContractConfig {
        agg_vkey_hash: B256::default(),
        mantle_vkey_commitment: B256::default(),
        rollup_config_hash: B256::default(),
        mantle_vk,
    };

    let app = Router::new()
        .route("/request_span_proof", post(request_span_proof))
        .route("/request_mantle_local", post(request_mantle_local))
        // .route("/request_agg_proof", post(request_agg_proof))
        .route("/status/:proof_id", get(get_proof_status))
        .route("/ping", get(|| async { "pong" }))
        // .route("/validate_config", post(validate_config))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(102400 * 1024 * 1024))
        .with_state(global_hashes);

    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    println!("PORT: {}", port);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .unwrap();
    println!("Server listening on {}", listener.local_addr().unwrap());
    // info!("Server listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

/// Validate the configuration of the L2 Output Oracle.
async fn validate_config(
    State(state): State<ContractConfig>,
    Json(payload): Json<ValidateConfigRequest>,
) -> Result<(StatusCode, Json<ValidateConfigResponse>), AppError> {
    unimplemented!()
    // info!("Received validate config request: {:?}", payload);
    // let fetcher = OPSuccinctDataFetcher::default();
    //
    // let address = Address::from_str(&payload.address).unwrap();
    // let l2_output_oracle = L2OutputOracle::new(address, fetcher.l1_provider);
    //
    // let agg_vkey = l2_output_oracle.aggregationVkey().call().await?;
    // let range_vkey = l2_output_oracle.rangeVkeyCommitment().call().await?;
    // let rollup_config_hash = l2_output_oracle.rollupConfigHash().call().await?;
    //
    // let agg_vkey_valid = agg_vkey.aggregationVkey == state.agg_vkey_hash;
    // let range_vkey_valid = range_vkey.rangeVkeyCommitment == state.range_vkey_commitment;
    // let rollup_config_hash_valid = rollup_config_hash.rollupConfigHash == state.rollup_config_hash;
    //
    // Ok((
    //     StatusCode::OK,
    //     Json(ValidateConfigResponse {
    //         rollup_config_hash_valid,
    //         agg_vkey_valid,
    //         range_vkey_valid,
    //     }),
    // ))
}

async fn request_mantle_local(
    Json(payload): Json<MantleSingleProofRequest>,
) -> Result<(StatusCode, Json<ProofResponse>), AppError> {
    let block_number = payload.block_number;
    let sp1_stdin = get_mantle_proof_stdin(block_number).await.unwrap();

    let client = ProverClient::new();
    let (_, execution_report) = client.execute(MANTLE_ELF, sp1_stdin).run().unwrap();
    // Print the total number of cycles executed and the full execution report with a breakdown of
    // the RISC-V opcode and syscall counts.
    println!(
        "Executed program with {} cycles",
        execution_report.total_instruction_count() + execution_report.total_syscall_count()
    );
    println!("Full execution report:\n{:?}", execution_report);
    Ok((StatusCode::OK, Json(ProofResponse { proof_id: "".to_string() })))
}

/// Request a proof for a span of blocks.
async fn request_span_proof(
    Json(payload): Json<MantleSingleProofRequest>,
) -> Result<(StatusCode, Json<ProofResponse>), AppError> {
    info!("Received span proof request: {:?}", payload);
    let block_number = payload.block_number;
    let sp1_stdin = get_mantle_proof_stdin(block_number).await.unwrap();
    
    let prover = NetworkProverV1::new();
    let res = prover
        .request_proof(MANTLE_ELF, sp1_stdin, ProofMode::Compressed)
        .await;

    // Check if error, otherwise get proof ID.
    let proof_id = match res {
        Ok(proof_id) => proof_id,
        Err(e) => {
            log::error!("Failed to request proof: {}", e);
            return Err(AppError(anyhow::anyhow!("Failed to request proof: {}", e)));
        }
    };

    Ok((StatusCode::OK, Json(ProofResponse { proof_id })))
}

/// Request an aggregation proof for a set of subproofs.
async fn request_agg_proof(
    State(state): State<ContractConfig>,
    Json(payload): Json<AggProofRequest>,
) -> Result<(StatusCode, Json<ProofResponse>), AppError> {
    info!("Received agg proof request");
    let mut proofs_with_pv: Vec<SP1ProofWithPublicValues> = payload
        .subproofs
        .iter()
        .map(|sp| bincode::deserialize(sp).unwrap())
        .collect();

    let boot_infos: Vec<BootInfoStruct> = proofs_with_pv
        .iter_mut()
        .map(|proof| proof.public_values.read())
        .collect();

    let proofs: Vec<SP1Proof> = proofs_with_pv
        .iter_mut()
        .map(|proof| proof.proof.clone())
        .collect();

    let l1_head_bytes = hex::decode(
        payload
            .head
            .strip_prefix("0x")
            .expect("Invalid L1 head, no 0x prefix."),
    )?;
    let l1_head: [u8; 32] = l1_head_bytes.try_into().unwrap();

    let fetcher = OPSuccinctDataFetcher::new_with_rollup_config()
        .await
        .unwrap();
    let headers = fetcher
        .get_header_preimages(&boot_infos, l1_head.into())
        .await?;

    let prover = NetworkProverV1::new();

    let stdin =
        get_agg_proof_stdin(proofs, boot_infos, headers, &state.mantle_vk, l1_head.into()).unwrap();

    // Set simulation to true on aggregation proofs as they're relatively small.
    env::set_var("SKIP_SIMULATION", "false");
    let proof_id = prover
        .request_proof(AGG_ELF, stdin, ProofMode::Groth16)
        .await?;
    env::set_var("SKIP_SIMULATION", "true");

    Ok((StatusCode::OK, Json(ProofResponse { proof_id })))
}

/// Get the status of a proof.
async fn get_proof_status(
    Path(proof_id): Path<String>,
) -> Result<(StatusCode, Json<ProofStatus>), AppError> {
    info!("Received proof status request: {:?}", proof_id);
    let private_key = env::var("SP1_PRIVATE_KEY")?;

    let client = NetworkClient::new(&private_key);

    // Time out this request if it takes too long.
    let timeout = Duration::from_secs(10);
    let (status, maybe_proof) =
        match tokio::time::timeout(timeout, client.get_proof_status(&proof_id)).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                return Ok((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ProofStatus {
                        status: SP1ProofStatus::ProofUnspecifiedStatus.into(),
                        proof: vec![],
                        unclaim_description: None,
                    }),
                ));
            }
            Err(_) => {
                return Ok((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ProofStatus {
                        status: SP1ProofStatus::ProofUnspecifiedStatus.into(),
                        proof: vec![],
                        unclaim_description: None,
                    }),
                ));
            }
        };

    let unclaim_description = status.unclaim_description.unwrap_or_default();

    let unclaim_description_enum: UnclaimDescription = unclaim_description.into();

    let status: SP1ProofStatus = SP1ProofStatus::try_from(status.status)?;
    if status == SP1ProofStatus::ProofFulfilled {
        let proof: SP1ProofWithPublicValues = maybe_proof.unwrap();

        match proof.proof {
            SP1Proof::Compressed(_) => {
                // If it's a compressed proof, we need to serialize the entire struct with bincode.
                // Note: We're re-serializing the entire struct with bincode here, but this is fine
                // because we're on localhost and the size of the struct is small.
                let proof_bytes = bincode::serialize(&proof).unwrap();
                return Ok((
                    StatusCode::OK,
                    Json(ProofStatus {
                        status: status.into(),
                        proof: proof_bytes,
                        unclaim_description: None,
                    }),
                ));
            }
            SP1Proof::Groth16(_) => {
                // If it's a groth16 proof, we need to get the proof bytes that we put on-chain.
                let proof_bytes = proof.bytes();
                return Ok((
                    StatusCode::OK,
                    Json(ProofStatus {
                        status: status.into(),
                        proof: proof_bytes,
                        unclaim_description: None,
                    }),
                ));
            }
            SP1Proof::Plonk(_) => {
                // If it's a plonk proof, we need to get the proof bytes that we put on-chain.
                let proof_bytes = proof.bytes();
                return Ok((
                    StatusCode::OK,
                    Json(ProofStatus {
                        status: status.into(),
                        proof: proof_bytes,
                        unclaim_description: None,
                    }),
                ));
            }
            _ => (),
        }
    } else if status == SP1ProofStatus::ProofUnclaimed {
        return Ok((
            StatusCode::OK,
            Json(ProofStatus {
                status: status.into(),
                proof: vec![],
                unclaim_description: Some(unclaim_description_enum),
            }),
        ));
    }
    Ok((
        StatusCode::OK,
        Json(ProofStatus {
            status: status.into(),
            proof: vec![],
            unclaim_description: None,
        }),
    ))
}

pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", self.0)).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

