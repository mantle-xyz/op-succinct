# Proposer

The `op-succinct` service monitors the state of the L2 chain, requests proofs from the [Succinct Prover Network](https://docs.succinct.xyz/docs/sp1/prover-network/intro) and submits them to the L1.

## RPC Requirements

Confirm that your RPC's have all of the required endpoints as specified in the [prerequisites](../advanced/node-setup.md#required-accessible-endpoints) section.

## Hardware Requirements

We recommend the following hardware configuration for the default `op-succinct` validity service (1 concurrent proof request & 1 concurrent witness generation thread):

Using the docker compose file:

- Full `op-succinct` service: 2 vCPUs, 4GB RAM.
- Mock `op-succinct` service: 2 vCPUs, 8GB RAM. Increased memory because the machine is executing the proofs locally.

Depending on the number of concurrent requests you want to run, you may need to increase the number of vCPUs and memory allocated to the `op-succinct` container.

## Environment Setup

Make sure to include *all* of the required environment variables in the `.env` file.

Before starting the proposer, ensure you have deployed the relevant contracts and have the address of the proxy contract ready. Follow the steps in the [Environment Variables](./contracts/environment.md) section.

### Required Environment Variables

| Parameter | Description |
|-----------|-------------|
| `L1_RPC` | L1 Archive Node. |
| `L2_RPC` | L2 Execution Node (`op-geth`). |
| `L2_NODE_RPC` | L2 Rollup Node (`op-node`). |
| `NETWORK_PRIVATE_KEY` | Private key for the Succinct Prover Network. See the [Succinct Prover Network Quickstart](https://docs.succinct.xyz/docs/sp1/prover-network/quickstart) for setup instructions. |
| `L2OO_ADDRESS` | Address of the `OPSuccinctL2OutputOracle` contract. |
| L1 signer | Exactly one signer backend for the account that posts output roots to L1 — AWS KMS, GCP KMS, Web3Signer or `PRIVATE_KEY`. See [L1 Transaction Signer](#l1-transaction-signer). |

### Optional Environment Variables

| Parameter | Description |
|-----------|-------------|
| `L1_BEACON_RPC` | L1 Consensus (Beacon) Node. Could be required for integrations that access consensus-layer data. |
| `NETWORK_RPC_URL` | Default: `https://rpc.production.succinct.xyz`. RPC URL for the Succinct Prover Network. |
| `DATABASE_URL` | Default: `postgres://op-succinct@postgres:5432/op-succinct`. The address of a Postgres database for storing the intermediate proposer state. |
| `L1_CONFIG_DIR` | Default: `<project-root>/configs/L1`. The directory containing the L1 chain configuration files. |
| `L2_CONFIG_DIR` | Default: `<project-root>/configs/L2`. Directory containing L2 chain configuration files. On first run, the rollup config is fetched from the node RPC and cached here. On subsequent runs, the cached file is used. Delete the cached file and restart to force a refresh (e.g., after a hardfork activates). |
| `DGF_ADDRESS` | Address of the `DisputeGameFactory` contract. Note: If set, the proposer will create a dispute game with the DisputeGameFactory, rather than the `OPSuccinctL2OutputOracle`. Compatible with `OptimismPortal2`. |
| `RANGE_PROOF_STRATEGY` | Default: `reserved`. Set to `hosted` to use hosted proof strategy. |
| `AGG_PROOF_STRATEGY` | Default: `reserved`. Set to `hosted` to use hosted proof strategy. |
| `AGG_PROOF_MODE` | Default: `plonk`. Set to `groth16` to use Groth16 proof type. **Note:** Changing the proof mode requires updating the verifier gateway contract address in your L2OutputOracle contract deployment. See [SP1 Contract Addresses](https://docs.succinct.xyz/docs/sp1/verification/contract-addresses) for verifier addresses. |
| `SUBMISSION_INTERVAL` | Default: `1800`. The number of L2 blocks that must be proven before a proof is submitted to the L1. Note: The interval used by the validity service is always >= to the `submissionInterval` configured on the L2OO contract. To allow for the validity service to configure this parameter entirely, set the `submissionInterval` in the contract to `1`. |
| `RANGE_PROOF_INTERVAL` | Default: `1800`. The number of blocks to include in each range proof. For chains with high throughput, you need to decrease this value. |
| `RANGE_PROOF_EVM_GAS_LIMIT` | Default: `0`. The total amount of ethereum gas allowed to be in each range proof. If 0, uses the `RANGE_PROOF_INTERVAL` instead to do a fixed number of blocks interval. NOTE: if both `RANGE_PROOF_INTERVAL` and `RANGE_PROOF_EVM_GAS_LIMIT` are set, the number of blocks to include in each range proof is determined either when the cumulative gas reaches `RANGE_PROOF_EVM_GAS_LIMIT` or the number of blocks reaches `RANGE_PROOF_INTERVAL`, whichever occurs first. |
| `MAX_CONCURRENT_PROOF_REQUESTS` | Default: `1`. The maximum number of concurrent proof requests (in mock and real mode). |
| `MAX_CONCURRENT_WITNESS_GEN` | Default: `1`. The maximum number of concurrent witness generation requests. |
| `OP_SUCCINCT_MOCK` | Default: `false`. Set to `true` to run in mock proof mode. The `OPSuccinctL2OutputOracle` contract must be configured to use an `SP1MockVerifier`. |
| `METRICS_PORT` | Default: `8080`. The port to run the metrics server on. |
| `LOOP_INTERVAL` | Default: `60`. The interval (in seconds) between each iteration of the OP Succinct service. |
| `PROGRESS_WATCHDOG_SECS` | Default: derived (`LOOP_INTERVAL` + 2 × worst-case L1 send + 600; 2940 with defaults). If the main loop completes no iteration for this long while no L1 transaction is being sent, the proposer releases its chain lock and exits so the supervisor restarts it. `0` disables the watchdog. See [Single proposer per chain](#single-proposer-per-chain). |
| `SAFE_DB_FALLBACK` | Default: `false`. Whether to fallback to timestamp-based L1 head estimation even though SafeDB is not activated for op-node.  When `false`, proposer will panic if SafeDB is not available. It is by default `false` since using the fallback mechanism will result in higher proving cost. |
| `L1_BLOCK_TAG` | Default: `finalized`. Which L1 block to anchor proof generation against. One of `finalized` (Casper FFG finalized, ~13 min), `safe` (FFG justified checkpoint, ~6.4 min), `latest` (chain tip). Non-default values trade cryptoeconomic finality for latency and must only be used by operators who understand the L1-reorg implications for validity-mode chains. See the [L1 block selection](#l1-block-selection) section below. |
| `L1_CONFIRMATIONS` | Default: `0`. Number of additional L1 block confirmations to wait behind the block returned by `L1_BLOCK_TAG` (e.g. `L1_BLOCK_TAG=latest` with `L1_CONFIRMATIONS=4` proves against `latest - 4`). Saturates at 0 if the offset would underflow. |
| `OP_SUCCINCT_CONFIG_NAME` | Default: `"opsuccinct_genesis"`. The name of the configuration the proposer will interact with on chain. |
| `OTLP_ENABLED` | Default: `false`. Whether to export logs to [OTLP](https://opentelemetry.io/docs/specs/otel/protocol/). |
| `LOGGER_NAME` | Default: `op-succinct`. This will be the `service.name` exported in the OTLP logs. |
| `OTLP_ENDPOINT` | Default: `http://localhost:4317`. The endpoint to forward OTLP logs to. |
| `USE_KMS_REQUESTER` | Default: ``.  Whether to expect NETWORK_PRIVATE_KEY to be an AWS KMS key ARN instead of a plaintext private key. |
| `MAX_PRICE_PER_PGU` | Default: `300,000,000`. The maximum price per pgu for proving. |
| `PROVING_TIMEOUT` | Default: `14400` (4 hours). The timeout to use for proving (in seconds). |
| `NETWORK_CALLS_TIMEOUT` | Default: `15` (15 seconds). The timeout for network prover calls (in seconds). |
| `RANGE_CYCLE_LIMIT` | Default: `1,000,000,000,000`. The cycle limit to use for range proofs. |
| `RANGE_GAS_LIMIT` | Default: `1,000,000,000,000`. The gas limit to use for range proofs. |
| `AGG_CYCLE_LIMIT` | Default: `1,000,000,000,000`. The cycle limit to use for aggregation proofs. |
| `AGG_GAS_LIMIT` | Default: `1,000,000,000,000`. The gas limit to use for aggregation proofs. |
| `WHITELIST` | Default: ``. The list of prover addresses that are allowed to bid on proof requests. |
| `PRIVATE_STDIN` | Default: `false`. Gated Succinct Prover Network feature. Keeps program inputs private from bidders; [contact us](https://partner.succinct.xyz/) to enable it for your requester account. |
| `MIN_AUCTION_PERIOD` | Default: `1`. The minimum auction period (in seconds). |
| `AUCTION_TIMEOUT` | Default: `60` (1 minute). How long to wait before canceling a proof request that hasn't been assigned (in seconds). |
| `TX_CONFIRMATION_TIMEOUT` | Default: `60`. Maximum time (in seconds) to wait for an L1 transaction to reach the required number of confirmations. Raise on congested L1s to avoid timeout-triggered retries. |

### L1 Transaction Signer

The account that posts output roots to L1 is configured by exactly **one** of the backends below.

| Backend | Environment variables |
|---------|-----------------------|
| AWS KMS | `L1_SIGNER_AWS_KMS_KEY_ID` — key id, key ARN, alias name or alias ARN. When it is an ARN, KMS calls go to the key's region even if the process runs elsewhere (EKS injects `AWS_REGION` into every IRSA pod); credentials keep using the process's own region. Otherwise set `AWS_REGION`. Credentials come from the default AWS provider chain (IRSA on EKS); the source the SDK actually used is logged at startup (`AWS credentials source`, access key id masked), with a warning when IRSA / Pod Identity is configured but the credentials came from something earlier in that chain, such as static keys in the environment or a shared config profile. A blank `AWS_REGION` / `AWS_DEFAULT_REGION` is treated as unset (with a warning). To reach KMS through a VPC endpoint without private DNS set `AWS_ENDPOINT_URL_KMS`; `AWS_ENDPOINT_URL` and `AWS_ENDPOINT_URL_STS` are refused at startup, because the SDK would send the IRSA web-identity token to that address. The key must be an asymmetric `ECC_SECG_P256K1` / `SIGN_VERIFY` key, and the role needs `kms:GetPublicKey` and `kms:Sign` on it. |
| GCP KMS | `HSM_API_NAME` (`projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>[/cryptoKeyVersions/<V>]`) + `HSM_CREDENTIALS` (hex-encoded service-account JSON). Alternatively `GOOGLE_PROJECT_ID` + `GOOGLE_LOCATION` + `GOOGLE_KEYRING` + `HSM_KEY_NAME` (+ optional `HSM_KEY_VERSION`, default `1`) with ambient Google credentials. |
| Web3Signer | `SIGNER_URL` + `SIGNER_ADDRESS`. |
| Local key | `PRIVATE_KEY`. |

Selection rules:

- Configuring more than one of AWS KMS, GCP KMS and Web3Signer is a startup error. A backend counts as configured when one of its signer-specific variables is **present, even if empty**: `L1_SIGNER_AWS_KMS_KEY_ID`, `HSM_API_NAME`, `GOOGLE_KEYRING`, or `SIGNER_URL`. Generic variables such as `GOOGLE_PROJECT_ID`, `HSM_KEY_NAME` or a leftover `SIGNER_ADDRESS` do not select anything on their own.
- A partially configured backend (for example `HSM_API_NAME` without `HSM_CREDENTIALS`, or `L1_SIGNER_AWS_KMS_KEY_ID=""`) is a startup error; the proposer does not fall back to another backend — in particular not to `PRIVATE_KEY`.
- `PRIVATE_KEY` is used only when no other backend is configured. Set alongside one, it is ignored with a warning.
- Values are trimmed, so a trailing newline from a mounted secret is harmless.
- `CRYPTO_KEY_ID` is **not** a signer variable. In Mantle deployments it names the AWS KMS key that decrypts secrets at startup.
- Plain `AWS_KMS_KEY_ID` is **not** a signer variable either. It is Foundry's `--aws` wallet variable, used by this repo's deploy scripts for the deployer key.

At startup a KMS signer signs one fixed probe digest and checks that the signature recovers to its address, so a missing `kms:Sign` (or GCP signing) permission fails immediately rather than at the first L1 transaction. The selected backend and address are logged as `L1 transaction signer ready`. Building a KMS signer (credentials and public key, GCP or AWS) is bounded by 60s, so an unreachable KMS fails startup with an error instead of hanging. AWS KMS calls time out after 10s per attempt and 30s in total; each L1 transaction attempt (fill, sign, broadcast) and every L1 read made while sending is bounded, so a stalled signer or RPC fails the attempt instead of blocking every later transaction. A transaction is only reported as sent once it has 3 confirmations — including when it is found mined after a failed or replaced attempt. A send that fails before any transaction reaches a node — gas estimation reverts, signing fails, or the node refuses it for insufficient funds — stops at once with `L1 transaction not sent: … nothing was broadcast` rather than being retried at higher fees.

`NETWORK_PRIVATE_KEY` / `USE_KMS_REQUESTER` are separate: they sign Succinct Prover Network requests, not L1 transactions.

### Single proposer per chain

Only one proposer may send L1 transactions for a chain at a time. Proposers sharing a database coordinate through a lease in the `chain_locks` table:

- At startup the proposer takes the lease atomically, retrying every 10 seconds while another proposer holds it. A holder that is still running keeps it; one that has stopped is replaced within about two minutes.
- A background heartbeat renews the 120-second lease every 30 seconds. Immediately before every broadcast — including fee-bump replacements of a stuck transaction — the proposer checks it still holds the lease; if not, nothing more is broadcast.
- If the lease is lost — taken by another proposer, or not renewed in time — the proposer exits and the supervisor restarts it.
- If the main loop stops making progress for `PROGRESS_WATCHDOG_SECS` while no transaction is being sent, the proposer gives up the lease and exits.

Operational notes:

- **Never run two proposer versions side by side.** When upgrading or rolling back, stop the running proposer before starting the other version. Older versions do not honour the lease owner.
- Monitor `succinct_loop_last_progress_timestamp` (alert when it falls well behind) and `succinct_chain_lock_held`. A renewing lease does not mean the main loop is healthy.
- `clear-stuck-txs` uses the same account but takes no lease. Stop the proposer before running it.

## Build the Proposer Service

Build the OP Succinct validity service.

```bash
docker compose build
```

## Run the Proposer

Run the OP Succinct validity service.

```bash
docker compose up
```

To see the logs of the OP Succinct services, run:

```bash
docker compose logs -f
```

After several minutes, the validity service will start to generate range proofs. Once enough range proofs have been generated, an aggregation proof will be created. Once the aggregation proof is complete, it will be submitted to the L1.

To stop the OP Succinct validity service, run:

```bash
docker compose stop
```

## L1 block selection

The proposer decides which L1 block to anchor each proof against using the `L1_BLOCK_TAG` and `L1_CONFIRMATIONS` environment variables. The default (`finalized`, `0`) preserves historical behavior.

| `L1_BLOCK_TAG` | Lag from tip | Security | Reorg handling |
|---|---|---|---|
| `finalized` (default) | ~12.8 min (2 epochs) | Cryptoeconomic (Casper FFG) | Not possible to reorg under the 1/3+ slashing assumption. |
| `safe` | ~6.4 min (1 epoch) | Cryptoeconomic (2/3 validator attestations) | Justified checkpoints have never been reorged on Ethereum mainnet; reverting requires 1/3+ adversarial stake. |
| `latest` | 0s (+ `L1_CONFIRMATIONS`) | Probabilistic, depth-based | Reorgs at small depths are rare post-merge but not cryptoeconomically bounded. |

`L1_CONFIRMATIONS` subtracts additional blocks from the selected tag (e.g. `L1_BLOCK_TAG=latest` with `L1_CONFIRMATIONS=4` resolves to `latest.number - 4`).

### SafeDB requirement for non-default selections (Ethereum / EigenDA)

For Ethereum and EigenDA backends, any non-default selection (tag != `finalized` or `confirmations != 0`) resolves the max provable L2 block via `optimism_safeHeadAtL1Block(resolved_l1_number)`. This RPC requires SafeDB to be activated on the op-node. The proposer hard-fails at startup if SafeDB is unavailable under a non-default selection on these backends. `SAFE_DB_FALLBACK` only applies to the default selection; it does not provide a fallback for the non-default L1 -> L2 resolution path.

### Celestia backend: non-default selection is rejected at startup

Celestia's proving path is driven by Blobstream commitments and the op-celestia-indexer, not by an L1 block tag. `CelestiaOPSuccinctHost::calculate_safe_l1_head` and `CelestiaOPSuccinctHost::get_max_provable_l2_block_number` do not read `L1_BLOCK_TAG` / `L1_CONFIRMATIONS`. To avoid silently accepting a knob that would not actually change those decisions, the production proposer binaries and the covered operator-facing utility scripts under `scripts/` hard-fail at startup when Celestia is configured together with a non-default selection.

On Celestia, only the default selection (`finalized`, `0`) is allowed for those entrypoints. Some operator-facing scripts whose proof path does not consult the L1 selection (notably `scripts/utils/bin/preflight.rs`), as well as test harnesses and internal tools that construct a fetcher/host directly, do not invoke the shared enforcement helper and are out of scope for this policy. They will silently ignore `L1_BLOCK_TAG` / `L1_CONFIRMATIONS` rather than rejecting them.

### Operational notes

- The existing `succinct_l2_finalized_block` gauge keeps its literal meaning — the L2 block returned by `eth_getBlockByNumber("finalized")` — regardless of `L1_BLOCK_TAG`. A separate `succinct_l2_max_provable_block` gauge reports the L2 block the host is actually willing to anchor a proof against under the current backend + L1 selection (matches `succinct_l2_finalized_block` under default Ethereum/EigenDA; reflects the L2 safe head at the configured L1 anchor under non-default Ethereum/EigenDA; reflects the Blobstream-resolved max provable L2 block under Celestia).
- Invalid values for `L1_BLOCK_TAG` or `L1_CONFIRMATIONS` cause the proposer and covered utility scripts (which parse via `from_env()?`) to exit cleanly at startup with an error naming the offending env var and value. Non-covered scripts that build a fetcher via the default constructors (e.g. `agg.rs`, `block_data.rs`, `config.rs`, `preflight.rs`) parse via `from_env_or_default()` and will instead panic with the same env var name and value in the message. Double-check env values before running scripts.
