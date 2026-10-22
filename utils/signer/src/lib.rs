use std::{str::FromStr, sync::Arc};

use alloy_consensus::TxEnvelope;
use alloy_eips::Decodable2718;
use alloy_network::{Ethereum, EthereumWallet, TransactionBuilder};
use alloy_primitives::{keccak256, Address, Bytes, TxHash, TxKind};
use alloy_provider::{Provider, ProviderBuilder, Web3Signer};
use alloy_rpc_types_eth::{TransactionReceipt, TransactionRequest};
use alloy_signer::Signer as AlloySigner;
use alloy_signer_aws::AwsSigner;
use alloy_signer_gcp::{
    // [MANTLE] Use the gcloud_sdk version re-exported by alloy_signer_gcp 2.x so the
    // GoogleApiClient type used to construct GcpSigner matches what GcpSigner::new expects.
    // The workspace's gcloud-sdk = "0.27" pulls a different version that produces incompatible
    // types.
    gcloud_sdk::{
        google::cloud::kms::v1::key_management_service_client::KeyManagementServiceClient,
        GoogleApi, TokenSourceType, GCP_DEFAULT_SCOPES,
    },
    GcpKeyRingRef,
    GcpSigner,
    KeySpecifier,
};
use alloy_signer_local::PrivateKeySigner;
use alloy_transport_http::reqwest::Url;
use anyhow::{Context, Result};
use tokio::{sync::Mutex, time::Duration};

pub const NUM_CONFIRMATIONS: u64 = 3;
pub const TIMEOUT_SECONDS: u64 = 60;

const WEI_PER_GWEI: u128 = 1_000_000_000;

/// [MANTLE] Lower bound on `maxFeePerGas`, in gwei.
///
/// Sized against a real incident: with base fee at ~0.15 gwei, alloy's default estimate produced
/// 0.32 gwei, and every transaction became permanently unmineable once base fee reached 1.25 gwei.
/// The transactions this signer sends are small (a checkpoint is ~46k gas), so the absolute cost
/// of a floor this high is negligible — far cheaper than a stalled proposer.
pub const DEFAULT_FEE_FLOOR_GWEI: u128 = 3;

/// [MANTLE] Multiplier applied to base fee, replacing alloy's default of 2.
///
/// base fee rises at most 12.5% per block, so 2x only covers ~6 blocks of sustained increase.
/// 4x covers ~12. The thin default is precisely what let a rising base fee overtake in-flight
/// transactions.
pub const DEFAULT_BASE_FEE_MULTIPLIER: u128 = 4;

/// [MANTLE] How many times a transaction is re-sent at a higher price before giving up.
pub const DEFAULT_MAX_BUMPS: u32 = 3;

/// Floor on `maxPriorityFeePerGas`. A zero tip gives block builders no reason to include the
/// transaction, and makes the +10% replacement threshold awkward to clear.
const PRIORITY_FEE_FLOOR_WEI: u128 = 10_000_000; // 0.01 gwei

/// [MANTLE] Per-attempt and total time limits for AWS KMS calls (`GetPublicKey`, `Sign`).
/// A KMS sign normally takes tens of milliseconds; the SDK retries within the operation budget.
const AWS_KMS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(10);
const AWS_KMS_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);

/// [MANTLE] Upper bound on filling, signing and broadcasting one transaction attempt (the wait
/// for confirmations is bounded separately by the caller's timeout). Covers every signer kind —
/// a remote signer or an L1 RPC that stops answering mid-call would otherwise hold the
/// `SignerLock` indefinitely. On expiry the attempt fails into the normal same-nonce escalation.
const SEND_TIMEOUT: Duration = Duration::from_secs(120);

/// [MANTLE] Bound on each group of L1 reads made while `SignerLock` is held (pre-send nonce/fee
/// reads, the post-failure receipt lookup, the final nonce read). The default HTTP client has no
/// request timeout.
const L1_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// [MANTLE] How often the post-failure path re-checks a mined receipt's confirmation depth.
const CONFIRMATION_POLL_INTERVAL: Duration = Duration::from_secs(4);

/// [MANTLE] Bound on the startup signing self-check.
const SELF_CHECK_TIMEOUT: Duration = Duration::from_secs(60);

/// [MANTLE] A permission check the caller supplies, run immediately before every broadcast of a
/// send (see [`Signer::send_transaction_request_guarded`]). The proposer passes its chain lock
/// check. The signer itself knows nothing about locks.
pub type BroadcastGuard<'a> = &'a (dyn Fn() -> Result<()> + Send + Sync);

/// [MANTLE] A send stopped because its [`BroadcastGuard`] refused a broadcast. Nothing was
/// broadcast for the refused attempt, and the send does not escalate further.
#[derive(Debug)]
pub struct BroadcastRefused(pub String);

impl std::fmt::Display for BroadcastRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "broadcast refused: {}", self.0)
    }
}

impl std::error::Error for BroadcastRefused {}

fn check_broadcast_guard(guard: BroadcastGuard<'_>) -> Result<()> {
    guard().map_err(|e| anyhow::Error::new(BroadcastRefused(format!("{e:#}"))))
}

/// Reads and parses an env var, warning loudly if it is set but unparseable.
///
/// A silent fallback to the default is the wrong behaviour for an operational knob: a typo in a
/// value someone set during an incident would take effect as "no change", with nothing in the
/// logs to explain why.
fn parse_env<T: std::str::FromStr>(name: &str) -> Option<T> {
    match std::env::var(name) {
        Err(_) => None,
        Ok(raw) => match raw.trim().parse::<T>() {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!(
                    env_var = name,
                    value = %raw,
                    "Ignoring unparseable value; falling back to the default"
                );
                None
            }
        },
    }
}

/// [MANTLE] Fee strategy for L1 transactions.
///
/// Exists because this signer previously delegated entirely to alloy's automatic estimation and
/// never revisited a transaction once sent: a transaction priced below a later base fee sat in the
/// mempool forever, and nonce ordering meant it blocked every subsequent transaction from the same
/// account. Recovery required manual intervention.
#[derive(Clone, Copy, Debug)]
pub struct GasPolicy {
    /// Lower bound on `maxFeePerGas`, in wei.
    pub fee_floor_wei: u128,
    /// Multiplier applied to the current base fee.
    pub base_fee_multiplier: u128,
    /// Maximum number of price escalations before returning an error.
    pub max_bumps: u32,
}

impl Default for GasPolicy {
    fn default() -> Self {
        Self {
            fee_floor_wei: DEFAULT_FEE_FLOOR_GWEI * WEI_PER_GWEI,
            base_fee_multiplier: DEFAULT_BASE_FEE_MULTIPLIER,
            max_bumps: DEFAULT_MAX_BUMPS,
        }
    }
}

impl GasPolicy {
    /// Reads the policy from the environment, falling back to the defaults above.
    ///
    /// Overridable so a live incident can be handled by changing configuration rather than
    /// shipping a build.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            fee_floor_wei: parse_env("L1_GAS_FEE_FLOOR_GWEI")
                .map(|gwei: u128| gwei.saturating_mul(WEI_PER_GWEI))
                .unwrap_or(default.fee_floor_wei),
            base_fee_multiplier: parse_env("L1_GAS_BASE_FEE_MULTIPLIER")
                .filter(|m: &u128| {
                    // A zero multiplier would price every transaction at the floor alone,
                    // silently disabling the headroom this exists to provide.
                    if *m == 0 {
                        tracing::warn!(
                            "L1_GAS_BASE_FEE_MULTIPLIER=0 ignored; using {}",
                            DEFAULT_BASE_FEE_MULTIPLIER
                        );
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(default.base_fee_multiplier),
            max_bumps: parse_env("L1_TX_MAX_BUMPS").unwrap_or(default.max_bumps),
        }
    }
}

impl GasPolicy {
    /// Worst-case wall time one send can occupy, given a per-attempt confirmation timeout.
    ///
    /// Exposed so callers can reason about how long a send may block without knowing that
    /// escalation exists or how many attempts it makes. `Proposer` uses it to check its chain
    /// lock lease against the slowest possible iteration; previously it reached in for
    /// `max_bumps` and did that arithmetic itself, which meant the lease check silently depended
    /// on this module's retry shape.
    pub fn worst_case_send_secs(&self, per_attempt_timeout_secs: u64) -> u64 {
        let rpc = L1_RPC_TIMEOUT.as_secs();
        // Per attempt: the attempt itself, the post-failure receipt lookup, and — if that finds a
        // mined transaction — the wait for its confirmations (bounded by the same timeout).
        let per_attempt = attempt_budget_secs(per_attempt_timeout_secs)
            .saturating_add(rpc)
            .saturating_add(per_attempt_timeout_secs);
        // Widen before adding: `1 + max_bumps` in u32 overflows at u32::MAX, which a test caught.
        u64::from(self.max_bumps)
            .saturating_add(1)
            .saturating_mul(per_attempt)
            // Plus the pre-send nonce/fee reads and the final nonce read.
            .saturating_add(rpc.saturating_mul(2))
    }
}

/// [MANTLE] Wall-time budget of one send attempt: the confirmation wait plus [`SEND_TIMEOUT`] for
/// filling, signing and broadcasting. The send loop enforces exactly this, and
/// [`GasPolicy::worst_case_send_secs`] is built from it, so the proposer's lease check and the
/// real bound cannot drift apart.
fn attempt_budget_secs(confirmation_timeout_secs: u64) -> u64 {
    confirmation_timeout_secs.saturating_add(SEND_TIMEOUT.as_secs())
}

/// `maxFeePerGas` for a new transaction: the current price with headroom, never below the floor.
///
/// `reference_price_wei` is whatever the node reports as the going rate (`eth_gasPrice`), which on
/// an EIP-1559 chain already includes a tip estimate — so the result errs high, deliberately.
fn compute_max_fee(reference_price_wei: u128, priority_fee_wei: u128, policy: &GasPolicy) -> u128 {
    reference_price_wei
        .saturating_mul(policy.base_fee_multiplier)
        .saturating_add(priority_fee_wei)
        .max(policy.fee_floor_wei)
}

/// Fees for replacing a transaction that did not confirm in time.
///
/// EIP-1559 replacement requires **both** fields to rise by at least 10%; +25% clears that with
/// margin. The `+1` keeps the increase strict even at values where integer arithmetic would
/// otherwise round to a no-op — a bump that does not increase is rejected as underpriced, which
/// would leave the original transaction stuck.
fn bump_fees(max_fee_wei: u128, priority_fee_wei: u128) -> (u128, u128) {
    let bump = |v: u128| (v.saturating_mul(5) / 4).max(v.saturating_add(1));
    (bump(max_fee_wei), bump(priority_fee_wei))
}

/// Whether the caller already priced this transaction, in which case it is left untouched.
///
/// Both fields must be set: filling in just one would pair a caller's value with a computed one.
fn has_caller_fees(request: &TransactionRequest) -> bool {
    request.max_fee_per_gas.is_some() && request.max_priority_fee_per_gas.is_some()
}

#[derive(Clone, Debug)]
/// The type of signer to use for signing transactions.
pub enum Signer {
    /// The signer URL and address.
    Web3Signer(Url, Address),
    /// The local signer.
    LocalSigner(PrivateKeySigner),
    /// Cloud HSM signer using Google.
    CloudHsmSigner(GcpSigner),
    /// [MANTLE] AWS KMS signer (`L1_SIGNER_AWS_KMS_KEY_ID`, + `AWS_REGION` unless the key id is an
    /// ARN; credentials from the default AWS chain — IRSA on EKS). Not `CRYPTO_KEY_ID`, which
    /// is Mantle's secret-decryption key; see [`Signer::aws_kms_from_env`].
    AwsKmsSigner(AwsSigner),
}

impl Signer {
    pub fn address(&self) -> Address {
        match self {
            Signer::Web3Signer(_, address) => *address,
            Signer::LocalSigner(signer) => signer.address(),
            Signer::CloudHsmSigner(signer) => signer.address(),
            Signer::AwsKmsSigner(signer) => signer.address(),
        }
    }

    /// Creates a new Web3 signer with the given URL and address.
    pub fn new_web3_signer(url: Url, address: Address) -> Self {
        Signer::Web3Signer(url, address)
    }

    /// Creates a new local signer from a private key string.
    pub fn new_local_signer(private_key_str: &str) -> Result<Self> {
        let private_key =
            PrivateKeySigner::from_str(private_key_str).context("Failed to parse private key")?;
        Ok(Signer::LocalSigner(private_key))
    }

    /// Builds the L1 transaction signer selected by the environment.
    ///
    /// [MANTLE] Exactly one backend is chosen, by [`select_signer_backend`] — see there for the
    /// rules. Configuring two remote signers, or half of one, is an error rather than a silent
    /// fallthrough to the next backend. Remote (KMS) signers sign one probe digest before this
    /// returns, and the chosen backend and address are logged, so a mis-wired signer fails at
    /// startup instead of at the first L1 transaction.
    pub async fn from_env() -> Result<Self> {
        let backend = select_signer_backend(&raw_env_var)?;
        let signer = match backend {
            SignerBackend::GcpKmsResourcePath => Self::gcp_kms_from_resource_path().await?,
            SignerBackend::GcpKmsKeyring => Self::gcp_kms_from_keyring().await?,
            SignerBackend::AwsKms => Self::aws_kms_from_env().await?,
            SignerBackend::Web3Signer => {
                let url = env_var("SIGNER_URL").context("SIGNER_URL")?;
                let address = env_var("SIGNER_ADDRESS").context("SIGNER_ADDRESS")?;
                let signer_url = Url::parse(&url).context("Failed to parse SIGNER_URL")?;
                let signer_address =
                    Address::from_str(&address).context("Failed to parse SIGNER_ADDRESS")?;
                Signer::new_web3_signer(signer_url, signer_address)
            }
            SignerBackend::Local => {
                Signer::new_local_signer(&env_var("PRIVATE_KEY").context("PRIVATE_KEY")?)?
            }
        };

        match &signer {
            Signer::CloudHsmSigner(s) => verify_remote_signer(s, backend).await?,
            Signer::AwsKmsSigner(s) => verify_remote_signer(s, backend).await?,
            Signer::Web3Signer(..) | Signer::LocalSigner(_) => {}
        }

        if backend == SignerBackend::Local {
            tracing::warn!(
                backend = %backend,
                address = %signer.address(),
                "L1 transaction signer ready — using a plaintext PRIVATE_KEY"
            );
        } else {
            tracing::info!(
                backend = %backend,
                address = %signer.address(),
                "L1 transaction signer ready"
            );
        }
        Ok(signer)
    }

    /// GCP KMS via a full key resource path.
    ///
    /// [MANTLE compat] Existing Mantle deployments configure GCP KMS via two env vars:
    ///   HSM_API_NAME      — full GCP resource path
    ///     projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>[/cryptoKeyVersions/<V>]
    ///   HSM_CREDENTIALS   — hex-encoded JSON service account key
    /// Production posture forbids writing service-account JSON to disk, so this path pipes the
    /// decoded JSON straight into gcloud-sdk's TokenSourceType::Json — the credential never
    /// touches the filesystem.
    async fn gcp_kms_from_resource_path() -> Result<Self> {
        let hsm_api_name = env_var("HSM_API_NAME").context("HSM_API_NAME")?;
        let creds_json_hex = env_var("HSM_CREDENTIALS")
            .context("HSM_CREDENTIALS required when HSM_API_NAME is set")?;
        let creds_json_bytes = alloy_primitives::hex::decode(&creds_json_hex)
            .context("HSM_CREDENTIALS is not valid hex")?;
        let creds_json = String::from_utf8(creds_json_bytes)
            .context("HSM_CREDENTIALS hex did not decode to UTF-8 JSON")?;

        let GcpKeyPath { project_id, location, keyring, key_name, key_version } =
            parse_gcp_key_resource_path(&hsm_api_name)?;

        let keyring_ref = GcpKeyRingRef::new(&project_id, &location, &keyring);
        let key_specifier = KeySpecifier::new(keyring_ref, &key_name, key_version);

        let client = GoogleApi::from_function_with_token_source(
            KeyManagementServiceClient::new,
            "https://cloudkms.googleapis.com",
            None,
            GCP_DEFAULT_SCOPES.clone(),
            TokenSourceType::Json(creds_json),
        )
        .await?;
        let signer = GcpSigner::new(client, key_specifier, None).await?;

        Ok(Signer::CloudHsmSigner(signer))
    }

    /// GCP KMS via `GOOGLE_PROJECT_ID` / `GOOGLE_LOCATION` / `GOOGLE_KEYRING` + `HSM_KEY_NAME`
    /// (upstream's shape), with ambient Google credentials.
    async fn gcp_kms_from_keyring() -> Result<Self> {
        let project_id = env_var("GOOGLE_PROJECT_ID").context("GOOGLE_PROJECT_ID")?;
        let location = env_var("GOOGLE_LOCATION").context("GOOGLE_LOCATION")?;
        let keyring_name = env_var("GOOGLE_KEYRING").context("GOOGLE_KEYRING")?;
        let key_name = env_var("HSM_KEY_NAME").context("HSM_KEY_NAME")?;
        let key_version = match env_var("HSM_KEY_VERSION") {
            Some(v) => v.parse().context("HSM_KEY_VERSION is not a u64")?,
            None => 1,
        };

        let keyring = GcpKeyRingRef::new(&project_id, &location, &keyring_name);
        let key_specifier = KeySpecifier::new(keyring, &key_name, key_version);

        let client = GoogleApi::from_function(
            KeyManagementServiceClient::new,
            "https://cloudkms.googleapis.com",
            None,
        )
        .await?;
        let signer = GcpSigner::new(client, key_specifier, None).await?;

        Ok(Signer::CloudHsmSigner(signer))
    }

    /// [MANTLE] AWS KMS via `L1_SIGNER_AWS_KMS_KEY_ID` (key id, key ARN, alias name or alias ARN).
    ///
    /// Credentials come from the default AWS provider chain — IRSA on EKS, i.e. the pod's
    /// service-account role, which must be granted `kms:Sign` and `kms:GetPublicKey` on the key.
    /// The region is the key ARN's when the key id is an ARN (overriding `AWS_REGION`, see
    /// [`resolve_aws_region`]), otherwise `AWS_REGION`, otherwise whatever the default chain
    /// resolves (`AWS_DEFAULT_REGION`, profile).
    ///
    /// Deliberately not `CRYPTO_KEY_ID`: in Mantle deployments that name is the AWS KMS key
    /// `/usr/lib/crypto` uses to *decrypt* secrets at startup (with `CRYPTO_SECRET_NAMES`), and
    /// mainnet sets it today. Reusing it would read a decryption key as a signing key.
    ///
    /// Deliberately not plain `AWS_KMS_KEY_ID` either: that is Foundry's `--aws` wallet variable,
    /// and this repo's deploy scripts (`contracts/lib/func.sh`) use it for the *deployer* key. An
    /// operator who ran a deploy and then `clear-stuck-txs` from the same shell would sign as the
    /// deployer — a different account — with only a warning that `PRIVATE_KEY` was ignored.
    ///
    /// KMS calls are bounded by [`AWS_KMS_ATTEMPT_TIMEOUT`] / [`AWS_KMS_OPERATION_TIMEOUT`]. The
    /// SDK sets none by default, and a hung call here would hold the `SignerLock` and stall every
    /// later L1 send behind it while liveness probes stay green.
    async fn aws_kms_from_env() -> Result<Self> {
        let key_id = env_var("L1_SIGNER_AWS_KMS_KEY_ID").context("L1_SIGNER_AWS_KMS_KEY_ID")?;
        let key_region =
            kms_arn_region(&key_id).map(|region| aws_config::Region::new(region.to_string()));

        // The *home* region — what credential resolution (IRSA → STS) uses — comes from the
        // default chain (AWS_REGION, AWS_DEFAULT_REGION, profile). The key's region is only a
        // fallback for it, so a local run with nothing but an ARN still works. Overriding the
        // whole SdkConfig with the key's region would also send the IRSA STS call there, which
        // fails on a pod whose egress / VPC endpoints only cover its own region.
        let mut home_region = aws_config::meta::region::RegionProviderChain::default_provider();
        if let Some(region) = key_region {
            home_region = home_region.or_else(region);
        }
        let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .timeout_config(
                aws_config::timeout::TimeoutConfig::builder()
                    .operation_attempt_timeout(AWS_KMS_ATTEMPT_TIMEOUT)
                    .operation_timeout(AWS_KMS_OPERATION_TIMEOUT)
                    .build(),
            )
            .region(home_region)
            .load()
            .await;

        // Only the KMS client is pointed at the key's region.
        let region = resolve_aws_region(config.region().map(ToString::to_string), &key_id)
            .context(
                "AWS region is unknown: set AWS_REGION, or use a key ARN for \
                 L1_SIGNER_AWS_KMS_KEY_ID",
            )?;
        let kms_config = aws_sdk_kms::config::Builder::from(&config)
            .region(aws_config::Region::new(region.clone()))
            .build();
        let client = aws_sdk_kms::Client::from_conf(kms_config);
        let signer = AwsSigner::new(client, key_id.clone(), None).await.with_context(|| {
            // Name the region: a bare key id or alias looked up in the wrong one (e.g. a stale
            // AWS_DEFAULT_REGION) surfaces from KMS only as "not found".
            format!(
                "Failed to load AWS KMS key {key_id} in region {region}. The key must be an \
                 asymmetric ECC_SECG_P256K1 SIGN_VERIFY key in that region, and the role needs \
                 kms:GetPublicKey on it."
            )
        })?;

        Ok(Signer::AwsKmsSigner(signer))
    }

    /// Sends a transaction request, signed by the configured `signer`, using the default
    /// confirmation timeout of [`TIMEOUT_SECONDS`].
    pub async fn send_transaction_request(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
    ) -> Result<TransactionReceipt> {
        self.send_transaction_request_with_timeout(l1_rpc, transaction_request, TIMEOUT_SECONDS)
            .await
    }

    /// Sends a transaction request, signed by the configured `signer`, with a caller-supplied
    /// confirmation timeout (in seconds).
    ///
    /// [MANTLE] Prices the transaction under [`GasPolicy`] and, if it does not confirm within the
    /// timeout, re-sends it at the **same nonce** with escalated fees. Previously this sent once
    /// with alloy's automatic estimate and gave up on timeout, leaving the transaction in the
    /// mempool where nonce ordering blocked every later transaction from the same account until
    /// someone intervened by hand.
    ///
    /// A caller that priced the transaction itself keeps its fees; only the nonce is pinned.
    pub async fn send_transaction_request_with_timeout(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
        timeout_secs: u64,
    ) -> Result<TransactionReceipt> {
        self.send_transaction_request_guarded(l1_rpc, transaction_request, timeout_secs, &|| Ok(()))
            .await
    }

    /// [`Self::send_transaction_request_with_timeout`], with `guard` called immediately before
    /// **every** broadcast — the first attempt and each fee-escalation replacement — and once
    /// up front. If it fails, nothing more is broadcast: the send stops at once with a
    /// [`BroadcastRefused`] error instead of escalating. The proposer passes its chain lock
    /// check here, so a process that lost the lock mid-send (lease lapsed, or resumed from a
    /// pause) cannot broadcast a replacement.
    pub async fn send_transaction_request_guarded(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
        timeout_secs: u64,
        guard: BroadcastGuard<'_>,
    ) -> Result<TransactionReceipt> {
        // Up front too: refuses before any L1 read or signing when the guard already fails.
        check_broadcast_guard(guard)?;

        let policy = GasPolicy::from_env();
        let provider = ProviderBuilder::new().network::<Ethereum>().connect_http(l1_rpc.clone());

        // [MANTLE] Every L1 read below runs while `SignerLock` is held, and the default HTTP
        // client has no request timeout, so each one is bounded by `L1_RPC_TIMEOUT`: an RPC that
        // accepts the connection and never answers would otherwise block every later send.
        //
        // Pre-send reads, bounded together. Nothing has been broadcast yet, so failing here is
        // safe.
        let (request, mut max_fee, mut priority_fee) =
            tokio::time::timeout(L1_RPC_TIMEOUT, async {
                // Pin the nonce so every escalation replaces the same transaction rather than
                // queueing a new one behind it. `SignerLock` serialises sends, so nothing else
                // claims it meanwhile.
                let mut request = transaction_request;
                if request.nonce.is_none() {
                    let nonce = provider
                        .get_transaction_count(self.address())
                        .pending()
                        .await
                        .context("Failed to read pending nonce")?;
                    request.set_nonce(nonce);
                }

                let (max_fee, priority_fee) = if has_caller_fees(&request) {
                    // Respect explicit pricing (clear-stuck-txs relies on this) but still escalate
                    // it if the transaction stalls.
                    (
                        request.max_fee_per_gas.unwrap_or_default(),
                        request.max_priority_fee_per_gas.unwrap_or_default(),
                    )
                } else {
                    // `eth_gasPrice`, not the block's `baseFeePerGas`: on an EIP-1559 chain nodes
                    // return roughly base fee plus a tip estimate, so feeding it to the multiplier
                    // slightly *over*-estimates. That is the safe direction, and it avoids a second
                    // round trip to read the latest block header. Renamed accordingly so nobody
                    // reads this as a true base fee.
                    let gas_price =
                        provider.get_gas_price().await.context("Failed to read gas price")?;
                    let priority = provider
                        .get_max_priority_fee_per_gas()
                        .await
                        .unwrap_or(PRIORITY_FEE_FLOOR_WEI)
                        .max(PRIORITY_FEE_FLOOR_WEI);
                    (compute_max_fee(gas_price, priority, &policy), priority)
                };
                anyhow::Ok((request, max_fee, priority_fee))
            })
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                "L1 RPC did not answer the pre-send nonce / fee reads within {}s; nothing was sent",
                L1_RPC_TIMEOUT.as_secs()
            )
            })??;

        let mut sent_hashes = Vec::new();
        let mut attempt = 0;
        loop {
            let mut candidate = request.clone();
            candidate.set_max_fee_per_gas(max_fee);
            candidate.set_max_priority_fee_per_gas(priority_fee);

            let mut this_hash = None;
            // Bound the whole attempt: the confirmation wait is already limited to
            // `timeout_secs`, and `SEND_TIMEOUT` covers filling, signing (KMS / Web3Signer) and
            // broadcasting. A timeout is an ordinary failed attempt; `this_hash` still records
            // anything that was broadcast before it fired.
            let attempt_budget = Duration::from_secs(attempt_budget_secs(timeout_secs));
            let outcome = match tokio::time::timeout(
                attempt_budget,
                self.dispatch_transaction(
                    l1_rpc.clone(),
                    candidate,
                    timeout_secs,
                    &mut this_hash,
                    guard,
                ),
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(_) => Err(anyhow::anyhow!(
                    "L1 transaction attempt did not finish within {}s (signing, broadcast or \
                     confirmation stalled)",
                    attempt_budget.as_secs()
                )),
            };
            match outcome {
                Ok(receipt) => return Ok(receipt),
                // The guard refused this attempt's broadcast: stop here — no escalation, no
                // further broadcast. Earlier attempts' hashes are named for the operator; the
                // caller (whose permission to send just ended) decides what that means.
                Err(e) if e.downcast_ref::<BroadcastRefused>().is_some() => {
                    return Err(e.context(format!(
                        "L1 send stopped before broadcasting attempt {} (nonce {:?}); earlier \
                         attempts already broadcast: {sent_hashes:?}",
                        attempt + 1,
                        request.nonce
                    )));
                }
                Err(e) => {
                    // Record this attempt BEFORE deciding anything. The invariant that keeps a
                    // successful transaction from being reported as failed is: every `Err` return
                    // below is preceded by a receipt lookup over *all* hashes we have broadcast,
                    // including the one from this attempt. Pushing after the checks left the last
                    // attempt's hash unexamined, so a transaction confirming just after its
                    // confirmation timeout was reported as a failure — and the caller would then
                    // send a duplicate checkpoint.
                    if let Some(hash) = this_hash {
                        sent_hashes.push(hash);
                    }

                    // An escalation can lose the race: the transaction it replaces may have just
                    // been mined, in which case the node rejects the replacement. That is success
                    // — but only once it has the same `NUM_CONFIRMATIONS` the normal path waits
                    // for, so a caller never treats a one-block-deep checkpoint as done.
                    if let Some(receipt) =
                        confirmed_receipt_among(&provider, &sent_hashes, timeout_secs).await?
                    {
                        return Ok(receipt);
                    }

                    if attempt >= policy.max_bumps {
                        // Distinguish "still sitting in the mempool" from "the nonce was consumed
                        // by a transaction we cannot identify" — the operator's next move differs.
                        // Decided from the on-chain nonce rather than by matching error text,
                        // which varies between clients. A read that times out counts as "not
                        // consumed": the message then says "still pending", the safer reading.
                        let latest_nonce = tokio::time::timeout(
                            L1_RPC_TIMEOUT,
                            provider.get_transaction_count(self.address()),
                        )
                        .await;
                        let nonce_consumed = match (request.nonce, latest_nonce) {
                            (Some(nonce), Ok(Ok(latest))) => latest > nonce,
                            _ => false,
                        };

                        return Err(e.context(if nonce_consumed {
                            format!(
                                "nonce {:?} was consumed by a transaction other than the {} we \
                                 broadcast (most likely the original, mined before a replacement \
                                 reached the node). Its outcome is not knowable from here — check \
                                 the account's recent transactions.",
                                request.nonce,
                                sent_hashes.len()
                            )
                        } else {
                            format!(
                                "transaction did not confirm after {} attempts (final \
                                 maxFeePerGas {} gwei); it is still pending",
                                attempt + 1,
                                max_fee / WEI_PER_GWEI
                            )
                        }));
                    }

                    let (next_max, next_priority) = bump_fees(max_fee, priority_fee);
                    tracing::warn!(
                        nonce = ?request.nonce,
                        attempt = attempt + 1,
                        max_bumps = policy.max_bumps,
                        from_max_fee_gwei = max_fee / WEI_PER_GWEI,
                        to_max_fee_gwei = next_max / WEI_PER_GWEI,
                        error = ?e,
                        "L1 transaction did not confirm; replacing it at the same nonce with \
                         higher fees"
                    );
                    (max_fee, priority_fee) = (next_max, next_priority);
                    attempt += 1;
                }
            }
        }
    }

    /// Signs and broadcasts one attempt, waiting up to `timeout_secs` for confirmation.
    ///
    /// [MANTLE] Extracted from `send_transaction_request_with_timeout` so the escalation loop
    /// above is shared by every signer kind; the wallet-based kinds share [`send_with_wallet`].
    async fn dispatch_transaction(
        &self,
        l1_rpc: Url,
        mut transaction_request: TransactionRequest,
        timeout_secs: u64,
        sent_hash: &mut Option<TxHash>,
        guard: BroadcastGuard<'_>,
    ) -> Result<TransactionReceipt> {
        match self {
            Signer::Web3Signer(signer_url, signer_address) => {
                // Set the from address to the signer address.
                transaction_request.set_from(*signer_address);

                // Fill the transaction request with all of the relevant gas and nonce information.
                let provider = ProviderBuilder::new().network::<Ethereum>().connect_http(l1_rpc);
                let filled_tx = provider.fill(transaction_request).await?;

                // Sign the transaction request using the Web3Signer.
                let web3_provider =
                    ProviderBuilder::new().network::<Ethereum>().connect_http(signer_url.clone());
                let signer = Web3Signer::new(web3_provider.clone(), *signer_address);

                let mut tx = filled_tx
                    .as_builder()
                    .context("Web3Signer path: filled request is unexpectedly already signed")?
                    .clone();
                tx.normalize_data();

                let raw: Bytes =
                    signer.provider().client().request("eth_signTransaction", (tx,)).await?;

                let tx_envelope = TxEnvelope::decode_2718(&mut raw.as_ref())
                    .context("Web3Signer returned an undecodable eth_signTransaction payload")?;

                // Last moment before the transaction leaves this process.
                check_broadcast_guard(guard)?;
                // Record before broadcasting — see `send_with_wallet` for why the order matters.
                *sent_hash = Some(*tx_envelope.tx_hash());
                let pending = provider
                    .send_tx_envelope(tx_envelope)
                    .await
                    .context("Failed to send transaction")?;
                let receipt = pending
                    .with_required_confirmations(NUM_CONFIRMATIONS)
                    .with_timeout(Some(Duration::from_secs(timeout_secs)))
                    .get_receipt()
                    .await?;

                Ok(receipt)
            }
            Signer::LocalSigner(private_key) => {
                send_with_wallet(
                    l1_rpc,
                    EthereumWallet::new(private_key.clone()),
                    private_key.address(),
                    transaction_request,
                    timeout_secs,
                    sent_hash,
                    "Failed to send transaction",
                    guard,
                )
                .await
            }
            Signer::CloudHsmSigner(signer) => {
                send_with_wallet(
                    l1_rpc,
                    EthereumWallet::new(signer.clone()),
                    signer.address(),
                    transaction_request,
                    timeout_secs,
                    sent_hash,
                    "Failed to send GCP KMS-signed transaction",
                    guard,
                )
                .await
            }
            Signer::AwsKmsSigner(signer) => {
                send_with_wallet(
                    l1_rpc,
                    EthereumWallet::new(signer.clone()),
                    signer.address(),
                    transaction_request,
                    timeout_secs,
                    sent_hash,
                    "Failed to send AWS KMS-signed transaction",
                    guard,
                )
                .await
            }
        }
    }
}

/// Signs `transaction_request` with `wallet` (whose signer is `from`), broadcasts it, and waits
/// up to `timeout_secs` for [`NUM_CONFIRMATIONS`].
///
/// [MANTLE] Shared by every signer that signs locally through an [`EthereumWallet`] — a private
/// key, GCP KMS or AWS KMS. Their bodies were identical apart from the error message.
// The broadcast guard is the eighth; bundling these into a struct would only move the list.
#[allow(clippy::too_many_arguments)]
async fn send_with_wallet(
    l1_rpc: Url,
    wallet: EthereumWallet,
    from: Address,
    mut transaction_request: TransactionRequest,
    timeout_secs: u64,
    sent_hash: &mut Option<TxHash>,
    send_error: &'static str,
    guard: BroadcastGuard<'_>,
) -> Result<TransactionReceipt> {
    // Ensure the request has a `from` address so the wallet filler can sign it.
    transaction_request.set_from(from);
    if transaction_request.to.is_none() {
        // NOTE(fakedev9999): Anvil's wallet filler insists on a `to` field even for
        // deployments. Mark the request as contract creation so it can be signed.
        transaction_request.to = Some(TxKind::Create);
    }

    let provider = ProviderBuilder::new().network::<Ethereum>().wallet(wallet).connect_http(l1_rpc);

    // Sign first and record the hash *before* broadcasting. The caller bounds the whole attempt
    // with a timeout; if that fires while `eth_sendRawTransaction` is in flight, the node may
    // already have the transaction. With the hash recorded, the escalation loop can still find
    // its receipt — without it a transaction that was mined would be reported as failed, and the
    // proposer could send a duplicate checkpoint. (`send_transaction` only yields the hash after
    // the broadcast returns.)
    let envelope =
        provider.fill(transaction_request).await.context(send_error)?.try_into_envelope().map_err(
            |_| anyhow::anyhow!("{send_error}: the wallet filler did not sign the request"),
        )?;
    // Last moment before the transaction leaves this process: filling and (KMS) signing above
    // can take a while, and a guard that held at the start of the send may not hold now.
    check_broadcast_guard(guard)?;
    *sent_hash = Some(*envelope.tx_hash());

    let pending = provider.send_tx_envelope(envelope).await.context(send_error)?;
    let receipt = pending
        .with_required_confirmations(NUM_CONFIRMATIONS)
        .with_timeout(Some(Duration::from_secs(timeout_secs)))
        .get_receipt()
        .await?;

    Ok(receipt)
}

/// [MANTLE] After a failed attempt: finds a mined receipt among `hashes` and, if there is one,
/// waits until it has [`NUM_CONFIRMATIONS`] — the same depth the normal path requires.
///
/// - `Ok(Some(receipt))` — one of our transactions is mined and confirmed.
/// - `Ok(None)` — none is mined, the lookup timed out, or the mined one was reorged out while we
///   waited. The caller keeps escalating at the same nonce.
/// - `Err` — one is mined but did not reach the confirmation depth within `timeout_secs`.
///   Escalating is pointless (its nonce is consumed) and reporting success would skip the
///   confirmation requirement, so this fails, naming the transaction for the operator.
///
/// Previously the first receipt found was returned as success immediately, so a transaction one
/// block deep — and still reorg-able — was reported to the proposer as a completed checkpoint.
async fn confirmed_receipt_among<P: Provider>(
    provider: &P,
    hashes: &[TxHash],
    timeout_secs: u64,
) -> Result<Option<TransactionReceipt>> {
    // Bounded as a whole: this runs under `SignerLock`.
    let mined = tokio::time::timeout(L1_RPC_TIMEOUT, async {
        for hash in hashes {
            if let Ok(Some(receipt)) = provider.get_transaction_receipt(*hash).await {
                return Some(receipt);
            }
        }
        None
    })
    .await
    .unwrap_or_else(|_| {
        tracing::warn!(
            timeout_secs = L1_RPC_TIMEOUT.as_secs(),
            "Receipt lookup timed out; treating our transactions as not yet mined"
        );
        None
    });
    let Some(receipt) = mined else { return Ok(None) };
    let hash = receipt.transaction_hash;

    let wait_for_depth = async {
        loop {
            // Re-read the receipt each round: if it disappears, its block was reorged out.
            match provider.get_transaction_receipt(hash).await {
                Ok(None) => {
                    tracing::warn!(%hash, "Mined L1 transaction was reorged out; resuming escalation");
                    return Ok::<_, anyhow::Error>(None);
                }
                Ok(Some(current)) => {
                    if let (Some(block), Ok(latest)) =
                        (current.block_number, provider.get_block_number().await)
                    {
                        // confirmations = latest - block + 1, as the normal path counts them.
                        if latest.saturating_add(1) >= block.saturating_add(NUM_CONFIRMATIONS) {
                            return Ok(Some(current));
                        }
                    }
                }
                // A failed read is transient here; the overall timeout bounds the loop.
                Err(_) => {}
            }
            tokio::time::sleep(CONFIRMATION_POLL_INTERVAL).await;
        }
    };

    match tokio::time::timeout(Duration::from_secs(timeout_secs), wait_for_depth).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "L1 transaction {hash} is mined (block {:?}) but did not reach {NUM_CONFIRMATIONS} \
             confirmations within {timeout_secs}s; not reporting it as complete",
            receipt.block_number
        )),
    }
}

/// [MANTLE] A signer backend [`Signer::from_env`] can select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignerBackend {
    /// `HSM_API_NAME` + `HSM_CREDENTIALS` — Mantle's GCP KMS deployment shape.
    GcpKmsResourcePath,
    /// `GOOGLE_PROJECT_ID` + `GOOGLE_LOCATION` + `GOOGLE_KEYRING` + `HSM_KEY_NAME` — upstream's.
    GcpKmsKeyring,
    /// `L1_SIGNER_AWS_KMS_KEY_ID`.
    AwsKms,
    /// `SIGNER_URL` + `SIGNER_ADDRESS`.
    Web3Signer,
    /// `PRIVATE_KEY`.
    Local,
}

impl std::fmt::Display for SignerBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SignerBackend::GcpKmsResourcePath => "gcp-kms (HSM_API_NAME)",
            SignerBackend::GcpKmsKeyring => "gcp-kms (GOOGLE_KEYRING)",
            SignerBackend::AwsKms => "aws-kms",
            SignerBackend::Web3Signer => "web3signer",
            SignerBackend::Local => "local-private-key",
        })
    }
}

/// Remote backends: (backend, variables whose presence selects it, variables it then requires).
///
/// Selectors are only variables that mean "sign with this backend" and nothing else. Generic
/// names other components also read — `GOOGLE_PROJECT_ID` / `GOOGLE_LOCATION` (logging,
/// monitoring, ADC) or a leftover `SIGNER_ADDRESS` — must not select a backend, or an unrelated
/// setting turns into a "partially configured signer" startup failure.
const REMOTE_SIGNER_BACKENDS: [(SignerBackend, &[&str], &[&str]); 4] = [
    (SignerBackend::GcpKmsResourcePath, &["HSM_API_NAME"], &["HSM_API_NAME", "HSM_CREDENTIALS"]),
    (
        SignerBackend::GcpKmsKeyring,
        &["GOOGLE_KEYRING"],
        &["GOOGLE_PROJECT_ID", "GOOGLE_LOCATION", "GOOGLE_KEYRING", "HSM_KEY_NAME"],
    ),
    (SignerBackend::AwsKms, &["L1_SIGNER_AWS_KMS_KEY_ID"], &["L1_SIGNER_AWS_KMS_KEY_ID"]),
    (SignerBackend::Web3Signer, &["SIGNER_URL"], &["SIGNER_URL", "SIGNER_ADDRESS"]),
];

/// Reads an env var for use, trimmed; empty or whitespace-only reads as `None`.
///
/// Trimmed because a value mounted from a secret file often carries a trailing newline, which a
/// KMS key id or an address parser would otherwise receive verbatim.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(trimmed_non_empty)
}

fn trimmed_non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Reads an env var exactly as set — `Some("")` for a variable that is present but empty.
/// [`select_signer_backend`] needs that distinction; see there.
///
/// Presence is decided by `var_os`, not `var`: `var` reports a non-UTF-8 value as absent, which
/// would let a mangled selector fall through to `PRIVATE_KEY`. Such a value reads as empty, so it
/// selects its backend and then fails as unset.
fn raw_env_var(name: &str) -> Option<String> {
    std::env::var_os(name).map(|value| value.into_string().unwrap_or_default())
}

/// [MANTLE] Chooses exactly one signer backend from the environment.
///
/// - A remote backend (GCP KMS, AWS KMS, Web3Signer) is *configured* as soon as one of its
///   selecting variables is **present — even if empty** — and must then have all of its required
///   variables set to non-empty values, or this fails. Previously a half-configured remote signer
///   fell through to the next backend — and ultimately to `PRIVATE_KEY`, which mainnet's secret
///   also carries — without a word. "Present but empty" counts as configured for the same reason: a
///   template whose value failed to render (`HSM_API_NAME: ""`) must fail loudly, not quietly start
///   on the plaintext key.
/// - More than one configured remote backend is an error, not a precedence order.
/// - `PRIVATE_KEY` is used only when no remote backend is configured; alongside one it is ignored
///   with a warning.
fn select_signer_backend(env: &impl Fn(&str) -> Option<String>) -> Result<SignerBackend> {
    let present = |name: &str| env(name).is_some();
    let set = |name: &str| env(name).is_some_and(|v| !v.trim().is_empty());

    let mut configured = Vec::new();
    for (backend, selectors, required) in REMOTE_SIGNER_BACKENDS {
        if !selectors.iter().any(|name| present(name)) {
            continue;
        }
        let missing: Vec<&str> = required.iter().copied().filter(|name| !set(name)).collect();
        anyhow::ensure!(
            missing.is_empty(),
            "L1 signer `{backend}` is partially configured: {missing:?} unset or empty. Refusing to \
             fall back to another signer."
        );
        configured.push(backend);
    }

    match configured.as_slice() {
        [] => {
            anyhow::ensure!(
                set("PRIVATE_KEY"),
                "No L1 signer is configured. Set exactly one of:\n\
                 - AWS KMS: L1_SIGNER_AWS_KMS_KEY_ID (+ AWS_REGION unless the key id is an ARN)\n\
                 - GCP KMS (Mantle): HSM_API_NAME + HSM_CREDENTIALS (hex JSON)\n\
                 - GCP KMS (upstream): GOOGLE_PROJECT_ID + GOOGLE_LOCATION + GOOGLE_KEYRING + \
                 HSM_KEY_NAME\n\
                 - Web3Signer: SIGNER_URL + SIGNER_ADDRESS\n\
                 - Local: PRIVATE_KEY"
            );
            Ok(SignerBackend::Local)
        }
        [backend] => {
            if set("PRIVATE_KEY") {
                tracing::warn!(
                    backend = %backend,
                    "PRIVATE_KEY is set but ignored because a remote L1 signer is configured"
                );
            }
            Ok(*backend)
        }
        many => {
            let names: Vec<String> = many.iter().map(ToString::to_string).collect();
            anyhow::bail!(
                "More than one L1 signer is configured ({}). Set exactly one.",
                names.join(", ")
            )
        }
    }
}

/// Returns the region embedded in a KMS key or alias ARN
/// (`arn:<partition>:kms:<region>:<account>:key/<id>` or `...:alias/<name>`).
fn kms_arn_region(key_id: &str) -> Option<&str> {
    let parts: Vec<&str> = key_id.splitn(6, ':').collect();
    match parts.as_slice() {
        ["arn", _, "kms", region, _, _] if !region.is_empty() => Some(region),
        _ => None,
    }
}

/// The region the KMS client must use, given the home region the default chain resolved.
///
/// A key ARN names its region, and a KMS call to any other region fails with a confusing "key
/// not found" — so the ARN wins. It must beat the home region rather than conflict with it: on
/// EKS the IRSA pod-identity webhook injects `AWS_REGION` (the cluster's region) into every pod,
/// so operators cannot leave it unset, and a key in another region (e.g. a multi-region `mrk-`
/// key) is legitimate. A disagreement is logged, not fatal. Credentials keep using the home
/// region either way; see [`Signer::aws_kms_from_env`].
///
/// Without an ARN the home region is used; `None` if there is none.
fn resolve_aws_region(home_region: Option<String>, key_id: &str) -> Option<String> {
    match (home_region, kms_arn_region(key_id)) {
        (home, Some(arn)) => {
            if let Some(home) = home.filter(|home| home != arn) {
                tracing::warn!(
                    home_region = %home,
                    key_region = %arn,
                    "L1_SIGNER_AWS_KMS_KEY_ID is in another region than this process; KMS calls \
                     go to the key's region, credentials keep using the home region"
                );
            }
            Some(arn.to_string())
        }
        (home, None) => home,
    }
}

/// Signs one fixed digest and checks the signature recovers to the signer's address.
///
/// Building a KMS signer only exercises `GetPublicKey`. A role that lacks the *sign* permission,
/// or a key that cannot sign, would otherwise surface only at the first L1 transaction — for the
/// proposer, after a proof has already been paid for.
async fn verify_remote_signer<S>(signer: &S, backend: SignerBackend) -> Result<()>
where
    S: AlloySigner + Sync,
{
    let probe = keccak256(b"op-succinct: L1 signer startup self-check");
    let signature = tokio::time::timeout(SELF_CHECK_TIMEOUT, signer.sign_hash(&probe))
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "L1 signer `{backend}` ({}) did not answer the startup self-check within {}s",
                signer.address(),
                SELF_CHECK_TIMEOUT.as_secs()
            )
        })?
        .with_context(|| {
            format!(
                "L1 signer `{backend}` ({}) could not sign at startup — check that the role may \
                 sign with this key",
                signer.address()
            )
        })?;
    let recovered = signature
        .recover_address_from_prehash(&probe)
        .context("L1 signer startup self-check produced an unrecoverable signature")?;
    anyhow::ensure!(
        recovered == signer.address(),
        "L1 signer `{backend}` self-check recovered {recovered}, expected {}",
        signer.address()
    );
    Ok(())
}

/// Parsed components of a GCP KMS crypto key resource path.
///
/// Accepted shapes (as set in Mantle's HSM_API_NAME env var):
/// - `projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>` — version defaults to `1`
/// - `projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>/cryptoKeyVersions/<V>`
struct GcpKeyPath {
    project_id: String,
    location: String,
    keyring: String,
    key_name: String,
    key_version: u64,
}

fn parse_gcp_key_resource_path(s: &str) -> Result<GcpKeyPath> {
    let parts: Vec<&str> = s.split('/').collect();
    let invalid = || {
        anyhow::anyhow!(
            "HSM_API_NAME `{s}` is not a valid GCP KMS key resource path; expected \
             `projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>[/cryptoKeyVersions/<V>]`"
        )
    };
    if parts.len() != 8 && parts.len() != 10 {
        return Err(invalid());
    }
    if parts[0] != "projects" ||
        parts[2] != "locations" ||
        parts[4] != "keyRings" ||
        parts[6] != "cryptoKeys"
    {
        return Err(invalid());
    }
    let key_version = if parts.len() == 10 {
        if parts[8] != "cryptoKeyVersions" {
            return Err(invalid());
        }
        parts[9].parse().context("HSM_API_NAME cryptoKeyVersions segment is not a u64")?
    } else {
        1
    };
    Ok(GcpKeyPath {
        project_id: parts[1].to_string(),
        location: parts[3].to_string(),
        keyring: parts[5].to_string(),
        key_name: parts[7].to_string(),
        key_version,
    })
}

/// Wrapper around Signer that provides thread-safe transaction sending.
/// Transactions are serialized via a Mutex to prevent nonce conflicts.
#[derive(Clone, Debug)]
pub struct SignerLock {
    inner: Arc<Mutex<Signer>>,
    cached_address: Address,
}

impl SignerLock {
    /// Creates a new SignerLock wrapping the given Signer.
    pub fn new(signer: Signer) -> Self {
        let cached_address = signer.address();
        SignerLock { inner: Arc::new(Mutex::new(signer)), cached_address }
    }

    /// Creates a SignerLock from environment variables.
    pub async fn from_env() -> Result<Self> {
        Ok(SignerLock::new(Signer::from_env().await?))
    }

    /// Returns the address of the signer without acquiring a lock.
    pub fn address(&self) -> Address {
        self.cached_address
    }

    /// Sends a transaction request, signed by the configured signer, using the default
    /// confirmation timeout of [`TIMEOUT_SECONDS`]. Transactions are serialized via a Mutex
    /// to prevent nonce conflicts.
    pub async fn send_transaction_request(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
    ) -> Result<TransactionReceipt> {
        let signer = self.inner.lock().await;
        signer.send_transaction_request(l1_rpc, transaction_request).await
    }

    /// Sends a transaction request with a caller-supplied confirmation timeout (in seconds).
    /// Transactions are serialized via a Mutex to prevent nonce conflicts.
    pub async fn send_transaction_request_with_timeout(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
        timeout_secs: u64,
    ) -> Result<TransactionReceipt> {
        let signer = self.inner.lock().await;
        signer
            .send_transaction_request_with_timeout(l1_rpc, transaction_request, timeout_secs)
            .await
    }

    /// [`Signer::send_transaction_request_guarded`] under the lock: `guard` runs before every
    /// broadcast of this send.
    pub async fn send_transaction_request_guarded(
        &self,
        l1_rpc: Url,
        transaction_request: TransactionRequest,
        timeout_secs: u64,
        guard: BroadcastGuard<'_>,
    ) -> Result<TransactionReceipt> {
        let signer = self.inner.lock().await;
        signer
            .send_transaction_request_guarded(l1_rpc, transaction_request, timeout_secs, guard)
            .await
    }
}

/// [MANTLE] Gas policy and replacement escalation.
#[cfg(test)]
mod gas_policy_tests {
    use super::*;

    const GWEI: u128 = 1_000_000_000;

    fn policy() -> GasPolicy {
        GasPolicy { fee_floor_wei: 3 * GWEI, base_fee_multiplier: 4, max_bumps: 3 }
    }

    /// The incident this fixes: base fee was ~0.15 gwei, alloy's `base_fee * 2` produced
    /// 0.32 gwei, and the transaction became unmineable once base fee reached 1.25 gwei.
    /// The floor is what makes that impossible.
    #[test]
    fn floor_applies_when_the_network_is_cheap() {
        let base_fee = 150_000_000; // 0.15 gwei, as during the incident
        let priority = 10_000_000; // 0.01 gwei
        let max_fee = compute_max_fee(base_fee, priority, &policy());

        assert_eq!(max_fee, 3 * GWEI, "the floor must win when the multiplier result is tiny");
        assert!(
            max_fee > 1_250_000_000,
            "must survive the base fee that broke us (1.25 gwei), got {max_fee}"
        );
    }

    /// Above the floor the multiplier decides, and it must be thicker than alloy's default 2x.
    #[test]
    fn multiplier_applies_when_the_network_is_busy() {
        let base_fee = 30 * GWEI;
        let priority = 2 * GWEI;
        let max_fee = compute_max_fee(base_fee, priority, &policy());

        assert_eq!(max_fee, 30 * GWEI * 4 + 2 * GWEI);
        assert!(max_fee > base_fee * 2 + priority, "must exceed alloy's 2x default");
    }

    /// base_fee rises at most 12.5% per block, so a 4x buffer has to cover a meaningful number
    /// of consecutive full blocks. Twelve blocks is ~2.4 minutes.
    #[test]
    fn buffer_survives_a_sustained_base_fee_climb() {
        let base_fee = 10 * GWEI;
        let max_fee = compute_max_fee(base_fee, GWEI, &policy());

        let mut climbed = base_fee;
        let mut blocks = 0;
        while climbed <= max_fee {
            climbed = climbed * 1125 / 1000; // +12.5%, the per-block maximum
            blocks += 1;
        }
        assert!(blocks >= 12, "4x buffer should cover >=12 blocks of max climb, got {blocks}");
    }

    /// EIP-1559 replacement requires BOTH fee fields to rise by at least 10%. A bump that fails
    /// this is rejected as underpriced, which would leave the original transaction stuck — the
    /// whole failure mode we are removing.
    #[test]
    fn bump_satisfies_the_replacement_threshold() {
        for (max_fee, priority) in
            [(3 * GWEI, GWEI / 100), (GWEI, 0), (1, 0), (100 * GWEI, 5 * GWEI)]
        {
            let (new_max, new_priority) = bump_fees(max_fee, priority);

            assert!(
                new_max * 100 >= max_fee * 110,
                "maxFeePerGas {max_fee} -> {new_max} is under the +10% threshold"
            );
            assert!(
                new_priority * 100 >= priority * 110,
                "priority {priority} -> {new_priority} is under the +10% threshold"
            );
            assert!(new_max > max_fee, "must strictly increase, even from {max_fee}");
        }
    }

    /// Escalation must be monotone across attempts; a plateau would retry at a price the network
    /// has already rejected.
    #[test]
    fn repeated_bumps_strictly_increase() {
        let (mut max_fee, mut priority) = (3 * GWEI, GWEI / 100);
        for _ in 0..policy().max_bumps {
            let (next_max, next_priority) = bump_fees(max_fee, priority);
            assert!(next_max > max_fee);
            assert!(next_priority >= priority);
            (max_fee, priority) = (next_max, next_priority);
        }
        // 3 bumps at +25% must stay within sane bounds — not a gas-price explosion.
        assert!(max_fee < 3 * GWEI * 3, "escalation overshot: {max_fee}");
    }

    /// A caller that set its own fees is respected: clear-stuck-txs deliberately picks fees to
    /// replace specific stuck transactions and must not have them recomputed.
    #[test]
    fn caller_supplied_fees_are_not_overridden() {
        let explicit = TransactionRequest::default()
            .with_max_fee_per_gas(42 * GWEI)
            .with_max_priority_fee_per_gas(7 * GWEI);
        assert!(has_caller_fees(&explicit));

        assert!(!has_caller_fees(&TransactionRequest::default()));
        // Half-specified is not "specified": filling only one field would leave an inconsistent
        // pair, so treat it as ours to compute.
        assert!(!has_caller_fees(&TransactionRequest::default().with_max_fee_per_gas(42 * GWEI)));
    }

    /// The proposer checks its chain lock lease against this, so the arithmetic is part of the
    /// module's contract rather than an internal detail.
    #[test]
    fn worst_case_send_covers_every_attempt() {
        let p = policy(); // max_bumps = 3
        let rpc = L1_RPC_TIMEOUT.as_secs();
        // attempt (60 + send budget) + receipt lookup + confirmation wait (60)
        let per_attempt = 60 + SEND_TIMEOUT.as_secs() + rpc + 60;
        assert_eq!(
            p.worst_case_send_secs(60),
            4 * per_attempt + 2 * rpc,
            "1 initial attempt + 3 escalations, plus the pre-send and final reads"
        );

        // No escalation still costs one full attempt.
        let no_bumps = GasPolicy { max_bumps: 0, ..policy() };
        assert_eq!(no_bumps.worst_case_send_secs(60), per_attempt + 2 * rpc);

        // Must not overflow into a nonsensical small number on absurd input.
        assert_eq!(
            GasPolicy { max_bumps: u32::MAX, ..policy() }.worst_case_send_secs(u64::MAX),
            u64::MAX
        );
    }

    #[test]
    fn policy_defaults_match_the_documented_values() {
        let p = GasPolicy::from_env();
        assert_eq!(p.fee_floor_wei, DEFAULT_FEE_FLOOR_GWEI * GWEI);
        assert_eq!(p.base_fee_multiplier, DEFAULT_BASE_FEE_MULTIPLIER);
        assert_eq!(p.max_bumps, DEFAULT_MAX_BUMPS);
    }
}

/// [MANTLE] Signer backend selection and AWS region resolution.
#[cfg(test)]
mod signer_selection_tests {
    use std::collections::HashMap;

    use super::*;

    /// Mirrors `raw_env_var`: a listed variable is present with exactly the given value.
    fn select(vars: &[(&str, &str)]) -> Result<SignerBackend> {
        let env: HashMap<String, String> =
            vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        select_signer_backend(&|name: &str| env.get(name).cloned())
    }

    const KEY_ARN: &str =
        "arn:aws:kms:ap-southeast-1:123456789012:key/1234abcd-12ab-34cd-56ef-1234567890ab";

    #[test]
    fn each_backend_is_selected_on_its_own() {
        assert_eq!(
            select(&[("HSM_API_NAME", "p"), ("HSM_CREDENTIALS", "c")]).unwrap(),
            SignerBackend::GcpKmsResourcePath
        );
        assert_eq!(
            select(&[
                ("GOOGLE_PROJECT_ID", "p"),
                ("GOOGLE_LOCATION", "l"),
                ("GOOGLE_KEYRING", "k"),
                ("HSM_KEY_NAME", "n"),
            ])
            .unwrap(),
            SignerBackend::GcpKmsKeyring
        );
        assert_eq!(
            select(&[("L1_SIGNER_AWS_KMS_KEY_ID", KEY_ARN)]).unwrap(),
            SignerBackend::AwsKms
        );
        assert_eq!(
            select(&[("SIGNER_URL", "http://x"), ("SIGNER_ADDRESS", "0x00")]).unwrap(),
            SignerBackend::Web3Signer
        );
        assert_eq!(select(&[("PRIVATE_KEY", "0x01")]).unwrap(), SignerBackend::Local);
    }

    #[test]
    fn two_remote_backends_are_rejected_not_ranked() {
        let err = select(&[
            ("HSM_API_NAME", "p"),
            ("HSM_CREDENTIALS", "c"),
            ("L1_SIGNER_AWS_KMS_KEY_ID", KEY_ARN),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("More than one L1 signer"), "{err}");
    }

    #[test]
    fn half_configured_remote_backend_does_not_fall_back_to_private_key() {
        // The trap this guards: a remote signer missing one variable used to fall through
        // silently to PRIVATE_KEY.
        let err = select(&[("HSM_API_NAME", "p"), ("PRIVATE_KEY", "0x01")]).unwrap_err();
        assert!(err.to_string().contains("HSM_CREDENTIALS"), "{err}");

        let err =
            select(&[("GOOGLE_KEYRING", "k"), ("HSM_KEY_NAME", "n"), ("PRIVATE_KEY", "0x01")])
                .unwrap_err();
        assert!(err.to_string().contains("GOOGLE_PROJECT_ID"), "{err}");

        let err = select(&[("SIGNER_URL", "http://x"), ("PRIVATE_KEY", "0x01")]).unwrap_err();
        assert!(err.to_string().contains("SIGNER_ADDRESS"), "{err}");
    }

    #[test]
    fn generic_variables_do_not_select_a_backend() {
        // GOOGLE_PROJECT_ID / GOOGLE_LOCATION are read by logging, monitoring and ADC too, and a
        // leftover SIGNER_ADDRESS is common in shared .env files. None of them alone means "sign
        // with this backend".
        assert_eq!(
            select(&[
                ("GOOGLE_PROJECT_ID", "p"),
                ("GOOGLE_LOCATION", "l"),
                ("SIGNER_ADDRESS", "0x00"),
                ("PRIVATE_KEY", "0x01"),
            ])
            .unwrap(),
            SignerBackend::Local
        );
        assert_eq!(
            select(&[("GOOGLE_PROJECT_ID", "p"), ("L1_SIGNER_AWS_KMS_KEY_ID", KEY_ARN),]).unwrap(),
            SignerBackend::AwsKms
        );
    }

    #[test]
    fn private_key_alongside_a_remote_backend_is_ignored() {
        assert_eq!(
            select(&[("L1_SIGNER_AWS_KMS_KEY_ID", KEY_ARN), ("PRIVATE_KEY", "0x01")]).unwrap(),
            SignerBackend::AwsKms
        );
    }

    #[test]
    fn mantle_crypto_key_id_is_not_a_signer() {
        // mainnet sets CRYPTO_KEY_ID for /usr/lib/crypto secret decryption next to HSM_API_NAME.
        assert_eq!(
            select(&[
                ("HSM_API_NAME", "p"),
                ("HSM_CREDENTIALS", "c"),
                ("CRYPTO_KEY_ID", KEY_ARN),
                ("CRYPTO_SECRET_NAMES", "KMS_ACCOUNT_JSON_0"),
            ])
            .unwrap(),
            SignerBackend::GcpKmsResourcePath
        );
    }

    #[test]
    fn empty_remote_selector_fails_instead_of_falling_back_to_private_key() {
        // A template whose value failed to render must not quietly start on the plaintext key.
        for selector in ["L1_SIGNER_AWS_KMS_KEY_ID", "HSM_API_NAME", "GOOGLE_KEYRING", "SIGNER_URL"]
        {
            let err = select(&[(selector, ""), ("PRIVATE_KEY", "0x01")]).unwrap_err();
            assert!(err.to_string().contains("partially configured"), "{selector}: {err}");
            let err = select(&[(selector, "  "), ("PRIVATE_KEY", "0x01")]).unwrap_err();
            assert!(err.to_string().contains("partially configured"), "{selector}: {err}");
        }
        // An empty PRIVATE_KEY on its own is not a signer either.
        assert!(select(&[("PRIVATE_KEY", "")]).is_err());
    }

    #[test]
    fn foundry_and_stray_gcp_variables_do_not_select_a_backend() {
        // AWS_KMS_KEY_ID is Foundry's `--aws` deployer variable (contracts/lib/func.sh), and a
        // leftover HSM_KEY_NAME is common next to the HSM_API_NAME shape.
        assert_eq!(
            select(&[("AWS_KMS_KEY_ID", KEY_ARN), ("PRIVATE_KEY", "0x01")]).unwrap(),
            SignerBackend::Local
        );
        assert_eq!(
            select(&[("HSM_API_NAME", "p"), ("HSM_CREDENTIALS", "c"), ("HSM_KEY_NAME", "n")])
                .unwrap(),
            SignerBackend::GcpKmsResourcePath
        );
    }

    #[test]
    fn env_values_are_trimmed_for_use() {
        assert_eq!(trimmed_non_empty(" arn:aws:kms:x \n".into()).as_deref(), Some("arn:aws:kms:x"));
        assert_eq!(trimmed_non_empty(" \n".into()), None);
    }

    #[test]
    fn nothing_configured_is_an_error() {
        assert!(select(&[]).is_err());
    }

    #[test]
    fn key_arn_region_wins_over_aws_region() {
        assert_eq!(kms_arn_region(KEY_ARN), Some("ap-southeast-1"));
        assert_eq!(
            kms_arn_region("arn:aws:kms:us-east-1:123456789012:alias/proposer"),
            Some("us-east-1")
        );
        assert_eq!(kms_arn_region("1234abcd-12ab-34cd-56ef-1234567890ab"), None);
        assert_eq!(kms_arn_region("alias/proposer"), None);

        assert_eq!(resolve_aws_region(None, KEY_ARN).as_deref(), Some("ap-southeast-1"));
        assert_eq!(
            resolve_aws_region(Some("ap-southeast-1".into()), KEY_ARN).as_deref(),
            Some("ap-southeast-1")
        );
        // EKS injects AWS_REGION = the cluster's region into every IRSA pod. A key in another
        // region must still work, so the ARN wins instead of conflicting.
        assert_eq!(
            resolve_aws_region(Some("us-east-1".into()), KEY_ARN).as_deref(),
            Some("ap-southeast-1")
        );
        // No ARN: AWS_REGION, else defer to the default provider chain.
        assert_eq!(
            resolve_aws_region(Some("eu-west-1".into()), "alias/proposer").as_deref(),
            Some("eu-west-1")
        );
        assert_eq!(resolve_aws_region(None, "alias/proposer"), None);
    }

    #[tokio::test]
    async fn self_check_accepts_a_signer_that_signs_for_its_own_address() {
        let signer = PrivateKeySigner::random();
        verify_remote_signer(&signer, SignerBackend::Local).await.unwrap();
    }

    /// Live check against a real AWS KMS key. Run with
    /// `L1_SIGNER_AWS_KMS_KEY_ID=<arn> cargo test -p op-succinct-signer-utils aws_kms_live --
    /// --ignored` using credentials that may call kms:GetPublicKey and kms:Sign on the key.
    #[tokio::test]
    #[ignore]
    async fn aws_kms_live_signer_builds_and_self_checks() {
        let signer = Signer::aws_kms_from_env().await.unwrap();
        let Signer::AwsKmsSigner(aws) = &signer else { panic!("expected an AWS KMS signer") };
        verify_remote_signer(aws, SignerBackend::AwsKms).await.unwrap();
        println!("AWS KMS signer address: {}", signer.address());
    }

    /// anvil's first default account.
    const ANVIL_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

    /// The post-failure path must not report a transaction one block deep as done.
    /// Needs `anvil --port 8545` (automine; set `no_proxy=127.0.0.1,localhost` behind a proxy).
    #[tokio::test]
    #[ignore]
    async fn shallow_receipt_is_not_reported_until_confirmed() {
        let url: Url = "http://127.0.0.1:8545".parse().unwrap();
        let key: PrivateKeySigner = ANVIL_KEY.parse().unwrap();
        let sender = ProviderBuilder::new()
            .network::<Ethereum>()
            .wallet(EthereumWallet::new(key))
            .connect_http(url.clone());
        let tx = TransactionRequest::default()
            .to(Address::repeat_byte(0x11))
            .value(alloy_primitives::U256::from(1u64));
        let hash = *sender.send_transaction(tx).await.unwrap().tx_hash();
        let provider = ProviderBuilder::new().network::<Ethereum>().connect_http(url);

        // Automine put it in its own block and nothing else is coming: 1 confirmation.
        let err = confirmed_receipt_among(&provider, &[hash], 6).await.unwrap_err();
        assert!(err.to_string().contains("did not reach"), "{err}");

        // Two more blocks make 3 confirmations.
        provider
            .client()
            .request::<_, ()>("anvil_mine", (alloy_primitives::U256::from(2u64),))
            .await
            .unwrap();
        let receipt = confirmed_receipt_among(&provider, &[hash], 10).await.unwrap();
        assert_eq!(receipt.map(|r| r.transaction_hash), Some(hash));

        // A hash that was never mined is "not found", not an error.
        let missing = TxHash::repeat_byte(0x77);
        assert!(confirmed_receipt_among(&provider, &[missing], 5).await.unwrap().is_none());
    }

    /// An L1 RPC that accepts the connection and never answers must fail the send within
    /// `L1_RPC_TIMEOUT` instead of holding the signer forever. Takes ~30s, hence `#[ignore]`.
    #[tokio::test]
    #[ignore]
    async fn hung_l1_rpc_fails_the_send_instead_of_blocking() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket); // accept, never reply
            }
        });

        let signer = Signer::new_local_signer(ANVIL_KEY).unwrap();
        let started = std::time::Instant::now();
        let err = signer
            .send_transaction_request(
                format!("http://{addr}").parse().unwrap(),
                TransactionRequest::default().to(Address::ZERO),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pre-send"), "{err}");
        assert!(started.elapsed() < L1_RPC_TIMEOUT + Duration::from_secs(10));
    }

    /// A guard that already fails refuses the send before any L1 read or signing.
    #[tokio::test]
    async fn guard_refusing_up_front_sends_nothing() {
        let signer = Signer::new_local_signer(ANVIL_KEY).unwrap();
        let err = signer
            .send_transaction_request_guarded(
                // Unroutable: any network access would fail differently (or hang).
                "http://127.0.0.1:9".parse().unwrap(),
                TransactionRequest::default().to(Address::ZERO),
                60,
                &|| anyhow::bail!("chain lock not held"),
            )
            .await
            .unwrap_err();
        assert!(err.downcast_ref::<BroadcastRefused>().is_some(), "{err:#}");
    }

    /// The guard is checked again at the last moment before broadcasting, after the transaction
    /// is filled and signed: a guard that held at the start but fails by then must leave nothing
    /// on chain. Needs `anvil --port 8545` (`no_proxy=127.0.0.1,localhost` behind a proxy).
    #[tokio::test]
    #[ignore]
    async fn guard_refusing_at_broadcast_leaves_nothing_on_chain() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let url: Url = "http://127.0.0.1:8545".parse().unwrap();
        let signer = Signer::new_local_signer(ANVIL_KEY).unwrap();
        let provider = ProviderBuilder::new().network::<Ethereum>().connect_http(url.clone());
        let nonce_before = provider.get_transaction_count(signer.address()).await.unwrap();

        // Allow the up-front check, refuse every later (pre-broadcast) one.
        let calls = AtomicUsize::new(0);
        let guard = move || {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                Ok(())
            } else {
                anyhow::bail!("lease lost while signing")
            }
        };
        let err = signer
            .send_transaction_request_guarded(
                url,
                TransactionRequest::default()
                    .to(Address::repeat_byte(0x22))
                    .value(alloy_primitives::U256::from(1u64)),
                60,
                &guard,
            )
            .await
            .unwrap_err();
        assert!(err.downcast_ref::<BroadcastRefused>().is_some(), "{err:#}");

        let nonce_after = provider.get_transaction_count(signer.address()).await.unwrap();
        assert_eq!(nonce_after, nonce_before, "nothing may reach the chain after a refusal");
    }
}

#[cfg(test)]
mod tests {
    use alloy_eips::BlockId;
    use alloy_primitives::{address, U256};
    use op_succinct_host_utils::OPSuccinctL2OutputOracle::OPSuccinctL2OutputOracleInstance as OPSuccinctL2OOContract;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_send_transaction_request_web3() {
        let proposer_signer = SignerLock::new(Signer::new_web3_signer(
            "http://localhost:9000".parse().unwrap(),
            "0x9b3F173823E944d183D532ed236Ee3B83Ef15E1d".parse().unwrap(),
        ));

        let provider = ProviderBuilder::new()
            .network::<Ethereum>()
            .connect_http("http://localhost:8545".parse().unwrap());

        let l2oo_contract = OPSuccinctL2OOContract::new(
            address!("0xDafA1019F21AB8B27b319B1085f93673F02A69B7"),
            provider.clone(),
        );

        let latest_header = provider.get_block(BlockId::latest()).await.unwrap().unwrap();

        let transaction_request = l2oo_contract
            .checkpointBlockHash(U256::from(latest_header.header.number))
            .into_transaction_request();

        let receipt = proposer_signer
            .send_transaction_request("http://localhost:8545".parse().unwrap(), transaction_request)
            .await
            .unwrap();

        println!("Signed transaction receipt: {receipt:?}");
    }

    #[tokio::test]
    #[ignore]
    // This test is meant to be ran locally to test various signers implementations,
    // depending of the envvars set.
    async fn test_send_transaction_request() {
        dotenv::dotenv().ok();

        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
        let signer = SignerLock::from_env().await.unwrap();

        println!("Signer: {}", signer.address());

        let transaction_request = TransactionRequest::default()
            .to(Address::from([
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
            ]))
            .value(U256::from(100000u64))
            .from(signer.address());
        let receipt = signer
            .send_transaction_request("http://localhost:8545".parse().unwrap(), transaction_request)
            .await
            .unwrap();
        println!("Signed transaction receipt: {receipt:?}");
    }
}
