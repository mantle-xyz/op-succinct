use std::{str::FromStr, sync::Arc};

use alloy_consensus::TxEnvelope;
use alloy_eips::Decodable2718;
use alloy_network::{Ethereum, EthereumWallet, TransactionBuilder};
use alloy_primitives::{Address, Bytes, TxHash, TxKind};
use alloy_provider::{Provider, ProviderBuilder, Web3Signer};
use alloy_rpc_types_eth::{TransactionReceipt, TransactionRequest};
use alloy_signer::Signer as AlloySigner;
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
        // Widen before adding: `1 + max_bumps` in u32 overflows at u32::MAX, which a test caught.
        u64::from(self.max_bumps).saturating_add(1).saturating_mul(per_attempt_timeout_secs)
    }
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
}

impl Signer {
    pub fn address(&self) -> Address {
        match self {
            Signer::Web3Signer(_, address) => *address,
            Signer::LocalSigner(signer) => signer.address(),
            Signer::CloudHsmSigner(signer) => signer.address(),
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

    pub async fn from_env() -> Result<Self> {
        // [MANTLE compat] Existing Mantle deployments configure GCP KMS via two env vars:
        //   HSM_API_NAME      — full GCP resource path
        //
        // projects/<P>/locations/<L>/keyRings/<KR>/cryptoKeys/<K>[/cryptoKeyVersions/<V>]
        //   HSM_CREDENTIALS   — hex-encoded JSON service account key
        // Production posture forbids writing service-account JSON to disk, so this path
        // pipes the decoded JSON straight into gcloud-sdk's TokenSourceType::Json — the
        // credential never touches the filesystem.
        if let Ok(hsm_api_name) = std::env::var("HSM_API_NAME") {
            let creds_json_hex = std::env::var("HSM_CREDENTIALS")
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

            return Ok(Signer::CloudHsmSigner(signer));
        }

        if let (Ok(project_id), Ok(location), Ok(keyring_name)) = (
            std::env::var("GOOGLE_PROJECT_ID"),
            std::env::var("GOOGLE_LOCATION"),
            std::env::var("GOOGLE_KEYRING"),
        ) {
            let key_name = std::env::var("HSM_KEY_NAME").expect("HSM_KEY_NAME");
            let key_version =
                std::env::var("HSM_KEY_VERSION").unwrap_or("1".to_string()).parse()?;

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
        } else if let (Ok(signer_url_str), Ok(signer_address_str)) =
            (std::env::var("SIGNER_URL"), std::env::var("SIGNER_ADDRESS"))
        {
            let signer_url = Url::parse(&signer_url_str).context("Failed to parse SIGNER_URL")?;
            let signer_address =
                Address::from_str(&signer_address_str).context("Failed to parse SIGNER_ADDRESS")?;
            Ok(Signer::new_web3_signer(signer_url, signer_address))
        } else if let Ok(private_key_str) = std::env::var("PRIVATE_KEY") {
            Signer::new_local_signer(&private_key_str)
        } else {
            anyhow::bail!(
                "None of the required signer configurations are set in environment:\n\
                - For Cloud HSM (Mantle compat): HSM_API_NAME + HSM_CREDENTIALS (hex JSON)\n\
                - For Cloud HSM (upstream): GOOGLE_PROJECT_ID, GOOGLE_LOCATION, GOOGLE_KEYRING\n\
                - For Web3Signer: SIGNER_URL and SIGNER_ADDRESS\n\
                - For Local: PRIVATE_KEY"
            )
        }
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
        let policy = GasPolicy::from_env();
        let provider = ProviderBuilder::new().network::<Ethereum>().connect_http(l1_rpc.clone());

        // Pin the nonce so every escalation replaces the same transaction rather than queueing a
        // new one behind it. `SignerLock` serialises sends, so nothing else claims it meanwhile.
        let mut request = transaction_request;
        if request.nonce.is_none() {
            let nonce = provider
                .get_transaction_count(self.address())
                .pending()
                .await
                .context("Failed to read pending nonce")?;
            request.set_nonce(nonce);
        }

        let (mut max_fee, mut priority_fee) = if has_caller_fees(&request) {
            // Respect explicit pricing (clear-stuck-txs relies on this) but still escalate it if
            // the transaction stalls.
            (
                request.max_fee_per_gas.unwrap_or_default(),
                request.max_priority_fee_per_gas.unwrap_or_default(),
            )
        } else {
            // `eth_gasPrice`, not the block's `baseFeePerGas`: on an EIP-1559 chain nodes
            // return roughly base fee plus a tip estimate, so feeding it to the multiplier
            // slightly *over*-estimates. That is the safe direction, and it avoids a second
            // round trip to read the latest block header. Renamed accordingly so nobody reads
            // this as a true base fee.
            let gas_price = provider.get_gas_price().await.context("Failed to read gas price")?;
            let priority = provider
                .get_max_priority_fee_per_gas()
                .await
                .unwrap_or(PRIORITY_FEE_FLOOR_WEI)
                .max(PRIORITY_FEE_FLOOR_WEI);
            (compute_max_fee(gas_price, priority, &policy), priority)
        };

        let mut sent_hashes = Vec::new();
        let mut attempt = 0;
        loop {
            let mut candidate = request.clone();
            candidate.set_max_fee_per_gas(max_fee);
            candidate.set_max_priority_fee_per_gas(priority_fee);

            let mut this_hash = None;
            match self
                .dispatch_transaction(l1_rpc.clone(), candidate, timeout_secs, &mut this_hash)
                .await
            {
                Ok(receipt) => return Ok(receipt),
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
                    // been mined, in which case the node rejects the replacement. That is success.
                    for hash in &sent_hashes {
                        if let Ok(Some(receipt)) = provider.get_transaction_receipt(*hash).await {
                            return Ok(receipt);
                        }
                    }

                    if attempt >= policy.max_bumps {
                        return Err(e.context(format!(
                            "transaction did not confirm after {} attempts (final maxFeePerGas \
                             {} gwei); it may still be pending",
                            attempt + 1,
                            max_fee / WEI_PER_GWEI
                        )));
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
    /// above is shared by all three signer kinds; the per-signer bodies are unchanged.
    async fn dispatch_transaction(
        &self,
        l1_rpc: Url,
        mut transaction_request: TransactionRequest,
        timeout_secs: u64,
        sent_hash: &mut Option<TxHash>,
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

                let mut tx = filled_tx.as_builder().unwrap().clone();
                tx.normalize_data();

                let raw: Bytes =
                    signer.provider().client().request("eth_signTransaction", (tx,)).await?;

                let tx_envelope = TxEnvelope::decode_2718(&mut raw.as_ref()).unwrap();

                let pending = provider
                    .send_tx_envelope(tx_envelope)
                    .await
                    .context("Failed to send transaction")?;
                // Record before waiting: on timeout the loop needs this to tell "still pending"
                // from "confirmed while we were escalating".
                *sent_hash = Some(*pending.tx_hash());
                let receipt = pending
                    .with_required_confirmations(NUM_CONFIRMATIONS)
                    .with_timeout(Some(Duration::from_secs(timeout_secs)))
                    .get_receipt()
                    .await?;

                Ok(receipt)
            }
            Signer::LocalSigner(private_key) => {
                let provider = ProviderBuilder::new()
                    .network::<Ethereum>()
                    .wallet(EthereumWallet::new(private_key.clone()))
                    .connect_http(l1_rpc);

                // Ensure the request has a `from` address so the wallet filler can sign it.
                transaction_request.set_from(private_key.address());
                if transaction_request.to.is_none() {
                    // NOTE(fakedev9999): Anvil's wallet filler insists on a `to` field even for
                    // deployments. Mark the request as contract creation so it can be signed.
                    transaction_request.to = Some(TxKind::Create);
                }

                let pending = provider
                    .send_transaction(transaction_request)
                    .await
                    .context("Failed to send transaction")?;
                *sent_hash = Some(*pending.tx_hash());
                let receipt = pending
                    .with_required_confirmations(NUM_CONFIRMATIONS)
                    .with_timeout(Some(Duration::from_secs(timeout_secs)))
                    .get_receipt()
                    .await?;

                Ok(receipt)
            }
            Signer::CloudHsmSigner(signer) => {
                // Set the from address to HSM address
                transaction_request.set_from(signer.address());
                if transaction_request.to.is_none() {
                    // NOTE(fakedev9999): Anvil's wallet filler insists on a `to` field even for
                    // deployments. Mark the request as contract creation so it can be signed.
                    transaction_request.to = Some(TxKind::Create);
                }

                let wallet = EthereumWallet::new(signer.clone());
                let provider = ProviderBuilder::new()
                    .network::<Ethereum>()
                    .wallet(wallet)
                    .connect_http(l1_rpc);

                let pending = provider
                    .send_transaction(transaction_request)
                    .await
                    .context("Failed to send KMS-signed transaction")?;
                *sent_hash = Some(*pending.tx_hash());
                let receipt = pending
                    .with_required_confirmations(NUM_CONFIRMATIONS)
                    .with_timeout(Some(Duration::from_secs(timeout_secs)))
                    .get_receipt()
                    .await?;

                Ok(receipt)
            }
        }
    }
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
        assert_eq!(p.worst_case_send_secs(60), 4 * 60, "1 initial attempt + 3 escalations");

        // No escalation still costs one full timeout.
        let no_bumps = GasPolicy { max_bumps: 0, ..policy() };
        assert_eq!(no_bumps.worst_case_send_secs(60), 60);

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
