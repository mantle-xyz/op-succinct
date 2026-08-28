# Mantle op-succinct Patches

This file is the authoritative registry of every Mantle modification stacked on top of
the upstream Succinct Labs `op-succinct` baseline. It is the primary reference when
synchronizing future upstream changes.

**Whenever Mantle changes are added, modified, or removed, update this file.**

## 1. Current baseline

| Item | Value |
|---|---|
| Upstream tracking point | succinctlabs/op-succinct tag `v3.12.0` @ `94ce6393` |
| Mantle branch | `main` (this repo, `origin` = `mantle-xyz/op-succinct`). Sync branches are cut from `main` and merged back by PR; both the `mantle/op-succinct-v3.8.1` and `mantle/proposer-hardening` branches were deleted once merged (PR #43 / #46). |
| Rust toolchain | nightly-2026-05-15 (rustc 1.97-nightly; see `rust-toolchain.toml`) |
| Dependency source: kona / op-alloy / alloy-op-evm | `mantle-xyz/mantle-v2` rust subtree @ tag `v1.6.1-rc0` (commit `37df2960`). Pinned by **tag**, not rev — 25 entries in `Cargo.toml` use `tag = "v1.6.1-rc0"`; bump them together. |
| Dependency source: revm family | `mantle-xyz/revm` @ tag `v107-mantle-arsia.1` (commit `1ed03aac`) via `[patch.crates-io]`, 16 entries. Resolves to revm 38.0.0 / revm-handler 18.1.0 / **op-revm 19.0.0** (upstream op-succinct is on op-revm 20.0.0). Moves in **lockstep** with the kona tag above. |
| Dependency source: alloy-evm | **upstream `alloy-rs/evm` v0.34.0 from crates.io — NOT patched.** The former `mantle-xyz/evm @ mantle-v0.34.0` fork only added a dead-code `token_ratio` trait method; mantle-v2/rust dropped it at `d2e4ebea` (commit `75d90fc71`), so the `[patch.crates-io]` redirect was removed here to stay in lockstep. |
| Dependency source: alloy core/network | crates.io `2.0.4` — deliberately **behind** upstream v3.12.0's `2.0.5`. See §3.11. |
| Dependency source: alloy-primitives | crates.io `1.5.x` (resolves 1.5.7) — deliberately behind upstream's 1.6.0, which belongs to the kona version lockstep. This is what fixes the `sha3` patch tag; see §3.11. |
| SP1 | `=6.4.0` + sp1-cluster tag `v2.7.2` |
| Contracts baseline | `mantle-xyz/op-succinct` tag `v1.1.7-2` (a.k.a. "v117"); ported into `contracts/` |

### Migration status

| Phase | Scope | Status |
|---|---|---|
| Phase 2 | bump workspace deps to mantle-v2/rust + alloy 2.x + revm 38 + drop EigenDA/Celestia + adapt source to new APIs | ✅ |
| Phase 3 | port v1.1.7-2 contracts onto v3.8.1 baseline + drop Fault Proof feature wholesale + fix v117 internal inconsistencies | ✅ |
| Phase 4 follow-up | redirect `alloy-evm` to `mantle-xyz/evm @ mantle-v0.34.0` fork (in this repo's `[patch.crates-io]`) | ✅ |
| Phase 5 audit | systematic audit of all 68 `mantle-xyz/op-succinct origin/main` Mantle commits vs v3.8.1 baseline | ✅ |
| Phase 5 ports | SP1 error propagation (1 line) + GCP HSM Mantle env-var compat (60 lines) | ✅ |
| Phase 5 follow-up | Rust ABI realignment to v117 contracts (`utils/host/src/contract.rs` + proposer) — caught when re-auditing for the PR-to-main merge | ✅ |
| Phase 5 follow-up | op-node pre-Interop compat — relax `rpc_types::SyncStatus` post-Interop fields to `Option<>` so the host can deserialize prod op-node responses (equivalent of `5efd6ead`) | ✅ |
| Upstream sync v3.8.1 → v3.12.0 | 22 upstream commits: takes #923 (checkpoint anchored to `safe`, upstreamed — see §3.10), #951/#952 (`invalidated_at` + range canonicality reconciliation), #924 (KZG `Ok(false)` fix), SP1 6.1.0 → 6.4.0. Drops the new `altda` DA backend. See §3.11. | ✅ |
| Upstream sync to v4.x | upstream is at v4.3.1; v3.12.0 → v4.x is its own phase | ⏸️ |

### 1.1 Supported L2 block range — Arsia and later only

> ⚠️ **op-succinct (cost-estimator, range proofs, validity proposer) only supports L2 blocks at or after the Mantle Arsia activation. Anything before Arsia will fail derivation.**

| Network | Arsia activation timestamp | First supported L2 block |
|---|---|---|
| Mantle mainnet (chain id 5000) | `1776841200` (2026-04-22 UTC) | **94355444** |

Why this hard floor:

- Before Arsia, Mantle ran in BVM mode with a Mantle-private batch encoding. EIP-4844 blobs were still posted to the same `batch_inbox = 0xffeeddccbbaa0000000000000000000000000000`, but the **blob payload layout is not the OP-Stack v0 encoding** (`BLOB_ENCODING_VERSION = 0` in `kona-derive/src/sources/blob_data.rs`).
- Arsia is the cut-over: from the activation block onward, the rollup config switches every OP-Stack hardfork (canyon / delta / ecotone / fjord / granite / holocene / isthmus / jovian / arsia) on at the same timestamp. Only from there does the chain produce OP-Stack-compliant frames, channels, and batches.
- `kona-derive` is OP-Stack only — it has no BVM legacy path and adding one would mean forking `BlobData::decode` / `FrameQueue` / `ChannelBank` / `BatchQueue` in parallel to the OP-Stack path. Out of scope for this repo.
- Symptom when ignored: `ERROR Failed to parse frames from data.` followed by `Failed to prefetch hint: ... header not found`, then a host panic. The frame-parse error is logged but swallowed in `frame_queue.rs:127`, so the first user-visible failure is the prefetch one.

Pick `--start` / `--end` ≥ 94355444 (mainnet) for any cost-estimator or proof run. For safety, skip the activation block itself and start at 94355445 — the activation block carries the Arsia upgrade-tx bundle (L1 Block / GPO / Operator Fee Vault redeploy + enable call) and has different shape than a steady-state block.

## 2. Architecture decisions

### 2.1 mantle-v2/rust supplies the Mantle protocol layer

Every kona-genesis / kona-protocol / kona-derive / kona-executor / kona-host / op-alloy /
alloy-op-evm dependency in `Cargo.toml` is sourced from `mantle-xyz/mantle-v2` at the
pinned `tag = "v1.6.1-rc0"` (25 entries; commit `37df2960`). The mantle-v2/rust
side owns:

- Mantle hardforks (ARSIA / JOVIAN / SKADI / LIMB) and their bundles
- BVM_ETH deposit-tx fields end-to-end
- Mantle-aware `RollupConfig` (predicates, BaseFee config, system-config kinds)
- Patched op-alloy / alloy-op-evm with Mantle deposit + receipt semantics

This keeps `op-succinct` itself a thin layer over upstream Succinct Labs source — only
SP1 program glue, validity proposer/requester, contracts, and CLI tooling live here.

### 2.2 revm + alloy-evm redirected via `[patch.crates-io]`

`Cargo.toml` `[patch.crates-io]` redirects:

```
# revm family → mantle-xyz/revm @ mantle-elysium
revm, revm-bytecode, revm-context, revm-context-interface, revm-database,
revm-database-interface, revm-handler, revm-inspector, revm-interpreter,
revm-precompile, revm-primitives, revm-state, op-revm
```

`alloy-evm` is **intentionally not patched** — it resolves from crates.io = upstream
`alloy-rs/evm` v0.34.0. The old `mantle-xyz/evm @ mantle-v0.34.0` fork existed only to add a
dead-code `token_ratio` method; mantle-v2/rust dropped the fork at `d2e4ebea`, so we dropped
the patch here too. `alloy-op-evm` and the `op-alloy*` crates are still patched to the
`mantle-xyz/mantle-v2` git source at the tag above (they live inside that subtree, not
crates.io).

Workspace-level `[patch.crates-io]` only applies when this repo is the workspace root.
mantle-v2/rust has its own `[patch.crates-io]` with the same entries — those don't
propagate transitively, so the duplication here is intentional and must stay in sync
with mantle-v2/rust/Cargo.toml after every dep refresh.

### 2.3 v117 contracts baseline; Fault Proof removed

`contracts/` is a verbatim port of `mantle-xyz/op-succinct` tag `v1.1.7-2` (Mantle's
last hand-tuned contract release before they restructured around the upstream Succinct
Labs flow). The rule used during Phase 3 was: **if v3.8.1's `contracts/` had a file that
also exists in v117 and the two differ, v117 wins**; otherwise keep v3.8.1's file.

Fault Proof (`contracts/src/fp/`, `contracts/script/fp/`, `fault-proof/` Rust crate,
related bindings) was deleted wholesale during Phase 3. Mantle's runtime is
Validity-Oracle-only; FP was upstream-Succinct-Labs scaffolding. Keeping it would have
caused cross-dependency drift on the Rust side (`fault-proof/` crate, `bindings/build.rs`
ABI references, `OPSuccinctDisputeGame*` test files) without any production use case.

### 2.4 Mantle-xyz/optimism layout in `contracts/lib/optimism`

`.gitmodules` was overwritten in Phase 3 to make `contracts/lib/optimism` point at
the Mantle optimism fork (see `.gitmodules` for the current URL and gitlink) rather than
`ethereum-optimism/optimism`.
v117 mixes `@optimism/src/<X>` and `@optimism/contracts/<X>` imports; mantle-v2 fork
only has a `contracts/` dir. `contracts/foundry.toml` therefore maps both prefixes to
`contracts/`. **Don't replace the gitlink with upstream optimism without re-checking
that every v117 import still resolves.**

### 2.5 v3.8.1 absorbed nearly all `origin/main` Mantle work

Phase 5 audit found that Succinct Labs upstream, between the old Mantle fork's
baseline (≈ v3.4.1 era) and v3.8.1, independently absorbed:

- `bytecode_address` vs `target_address` precompile-lookup fix (Mantle `2e4cc297`)
- `historicBlockHashes` checkpoint validation (Mantle `3acfb122`)
- `proving_timeout` config + per-request `timeout` wiring (Mantle `13badd18` / `39c38e9e` / `17ee3949`) — upstream named it `proving_timeout`, default 14400s; Mantle used `prove_timeout`, default 3600s
- `server_task.abort()` for witness-server task crash handling (Mantle `8ec31a34` / `e2d824cd`)
- `optimism_rollupConfig` runtime fetch replacing the old `configs/<chain_id>/rollup.json` files (upstream `c7bfaf22 remove configs`)
- New kona API surface: `get_and_validate_blobs`, `attrs.take_inner()`, `outcome` field, `Arc<L1ChainConfig>`
- GCP KMS signing via official `alloy-signer-gcp` v2.0.4 (replaces Mantle's bespoke `utils/signer-gcp/` 233-line crate)

The bridge between mantle-v2/rust deps + v3.8.1 baseline therefore covers ~95% of
Mantle's protocol/business-logic deltas. Only a handful of small ports remained — see §3.

> ⚠️ The original Phase 5 audit *missed* one thing: Phase 3 swapped the on-chain
> contracts to v117 but left the Rust `sol!` bindings on the **pre-v117 ABI**
> shape (6-param `proposeL2Output`, `opSuccinctConfigs` mapping). Caught when
> re-auditing before opening the PR-to-main. Ported via the `7aae8cf1` +
> `57d1cbaa` + `be5354a1` triplet — see §3.4b.

## 3. Mantle changes registry

Every change carries a `[MANTLE]` source comment. Discover all sites with:

```bash
grep -rn "\[MANTLE\]" . --include="*.rs" --include="*.toml" --include="*.sol" \
  | grep -v "target/" | grep -v "contracts/lib/"
```

### 3.1 Cargo workspace dependency wiring (Phase 2 / 4)

| File | Change |
|---|---|
| `Cargo.toml` | All `kona-*`, `op-alloy*`, `alloy-op-evm*` deps switched from crates.io / the official kona repo to `mantle-xyz/mantle-v2` git at the pinned tag (currently `v1.6.1-rc0`; see §1). |
| `Cargo.toml` `[patch.crates-io]` | All 13 revm-family crates redirected to `mantle-xyz/revm` at the pinned tag (currently `v107-mantle-arsia.1`; see §1). |
| `Cargo.toml` `[patch.crates-io]` | ~~`alloy-evm` redirected to `mantle-xyz/evm @ mantle-v0.34.0`.~~ **Dropped at the `d2e4ebea` bump** — `alloy-evm` now resolves from crates.io (upstream `alloy-rs/evm` v0.34.0). See §3.1a. |
| `Cargo.toml` | EigenDA and Celestia DA-backend crates dropped (`utils/eigenda/*`, `programs/range/*/celestia`, `programs/range/*/eigenda`, etc.). Validity-Oracle-only path. |

When bumping the mantle-v2 rev, refresh **every** `rev = "..."` in this file (a
`replace_all` of the old → new SHA is the canonical move). There are 25 such pins.

### 3.1a mantle-v2 rev bumps `29e41dad` → `d2e4ebea` → `13b367fc` (Sepolia blob-schedule divergence fix)

**Symptom.** On QA/Sepolia Mantle chains the proposer/cost-estimator failed with a
repeating `Failed to prefetch hint: ... header ... not found` (a non-canonical L1 hash),
and the host-derived L2 blocks did **not** match canonical — `transactions_root`,
`state_root`, and `receipts_root` all differed on a single-system-tx block. The
`header not found` was only a downstream symptom: the host requested L1/L2 data keyed by
hashes computed from its own **wrong** derived state.

**Root cause (the real one).** The L1 (Sepolia) had entered the **BPO2** blob-parameter
fork (EIP-7892: target 14, max 21, blob-base-fee update fraction **11684671**). op-node
computes the L1 `blobBaseFee` for the L1-attributes (Arsia) system tx with a config-aware
`block.BlobBaseFee(l1ChainConfig)` that honours BPO2 → `63365475`. But kona's
`crates/protocol/registry/src/l1/mod.rs` `default_blob_schedule()` only listed **Cancun +
Prague** (Osaka/BPO1/BPO2 were commented out), while `sepolia()` set the `osaka_time`/`bpo*_time`
fields. So for a BPO2-era block kona knew BPO2 was active but had no BPO2 params → fell back
to **Prague** (update fraction 5007716) → `blobBaseFee ≈ 1.6e18` instead of `63365475`. That
wrong value went into the system tx → `transactions_root` differed → every derived block
diverged. The L1 chain config (incl. `blobSchedule`) is served to the guest as the
`L1_CONFIG_KEY` preimage (read from `configs/L1/<l1_chain_id>.json`), so this is a pure input,
not compiled into the ELF.

**Fix (`13b367fc`).** In kona `default_blob_schedule()`, uncomment `osaka`/`bpo1`/`bpo2` so the
schedule carries their `BlobParams` (alloy `BlobParams::bpo2()` = 14/21/11684671, matching
op-geth `mantle-elysium` `params/config.go`). Mantle **mainnet** stays pinned to Prague because
`mainnet()` leaves `osaka_time`/`bpo*_time = None` — the extra schedule entries are inert when
the fork never activates (mirrors op-node's `MantleArsiaL1ChainConfigByChainID` mainnet =
Cancun+Prague only). As Sepolia advances to **BPO3/BPO4**, those entries must be added too, kept
in sync with op-geth's `BlobScheduleConfig`.

**Also note — cached L1 config.** op-succinct's `fetch_and_save_l1_config` uses
`configs/L1/<l1_chain_id>.json` if it already exists (cache), only regenerating from the kona
registry when absent. A stale cached file (Cancun+Prague only) will keep feeding the guest the
wrong schedule even after the kona fix — delete it so it regenerates, or edit the file's
`blobSchedule` directly as a hot-fix (no rebuild needed since it's a preimage input).

**Intermediate `d2e4ebea` bump.** Done first (from `29e41dad`) to pull `05b2cca3e`
(alloy-op-evm Mantle spec routing) + `05079251c` (op-alloy TxDeposit codec) and to drop the
`alloy-evm` fork patch (§2.2). These are kept but did **not** fix the blob divergence on their
own — the blob-schedule change at `13b367fc` is what resolves it.

**⚠️ vkey / ELF.** The Rust-dep bumps change guest-program execution → SP1 range/agg vkeys
change → `just build-elfs` on x64 + commit the `elf/*` + update on-chain vkeys. (The
`blobSchedule` content itself is a runtime preimage and does not affect the vkey.)

### 3.2 Contracts — v117 baseline (Phase 3)

| File | Change |
|---|---|
| `contracts/**/*.sol` | Verbatim port from `mantle-xyz/op-succinct@v1.1.7-2`. v117 wins on every file that disagrees with upstream v3.8.1. |
| `contracts/foundry.toml` | Remappings updated to point both `@optimism/src/=` and `@optimism/contracts/=` at `lib/optimism/packages/contracts-bedrock/contracts/`; plus narrow `src/<dir>/=` remappings for v117 import shapes. |
| `.gitmodules` | `contracts/lib/optimism` → `mantlenetworkio/mantle-v2.git`; three new submodules (`solady-v0.0.281`, `solmate`, `mantle-cdk`) added. |
| `contracts/test/validity/{Upgrade,OPSuccinctL2OutputOracle}.t.sol` | Two mechanical fixes for v117 internal test inconsistencies (missing struct fields; old 2-arg `checkpointBlockHash` call). |
| `contracts/src/fp/`, `contracts/script/fp/`, `contracts/test/fp/`, related utilities | **Deleted** (Fault Proof removed wholesale). |
| `contracts/test/helpers/Utils.sol` | Dropped two **unused** v117 imports (`@safe-contracts/contracts/Safe.sol` + `@safe-contracts/contracts/common/Enum.sol`). The types `Safe` / `Enum` are never referenced in the 94-line file, and the repo has no `safe-contracts` submodule or remapping — so `forge bind` on a fresh checkout was failing to parse this file even though `bindings/build.rs` passes `--skip 'test/**'` (forge still parses the tree during bind). |
| `contracts/lib/mantle-cdk` submodule + `.gitmodules` block | **Removed**. v117's `.gitmodules` pulled `mantle-xyz/mantle-cdk` (a **private** Mantle repo) but **nothing in `contracts/` ever imported it** — grep is 0 hits across `src/`, `script/`, `test/`, `foundry.toml`, `remappings.txt`. Carrying it meant every fresh server needed GitHub credentials with access to the private repo just to `git submodule update --init --recursive`. Phase 5 dropped it; if a future Mantle protocol upgrade does start needing CDK contracts, re-add it then. |

### 3.3 Fault Proof removed (Phase 3)

| Item | Action |
|---|---|
| `fault-proof/` Rust crate | Removed; dropped from workspace members + `[workspace.dependencies]`. |
| `scripts/prove/Cargo.toml` `op-succinct-fp` dep | Removed. |
| `bindings/build.rs` `required_contracts` list | Trimmed: `AccessManager`, `OPSuccinctFaultDisputeGame`, `MockOptimismPortal2`, `MockPermissionedDisputeGame`, `IFaultDisputeGame` removed. Kept: `DisputeGameFactory`, `SuperchainConfig`, `AnchorStateRegistry`, `SP1MockVerifier`, `ERC1967Proxy`, `IDisputeGame`, `IDisputeGameFactory` (still used by validity / general infra). |
| `validity/src/proposer.rs` and friends | FP integration paths trimmed; validity cluster proving flow retained. |

### 3.4 SP1 error propagation (Phase 5 — `f2bb0624` port)

| File | Change |
|---|---|
| `scripts/prove/bin/agg.rs` | Line ~190 (network-prover prove call): `.expect("proving failed")` → `.context("proving failed")?`. The matching `cpu_prover.setup` + `cpu_prover.execute` + `spawn_blocking` paths were already converted upstream in v3.8.1; only this one call site remained. |

### 3.4b Rust ABI realignment to v117 (Phase 5 follow-up — Mantle `7aae8cf1` + `57d1cbaa` + `be5354a1`-equivalent)

Phase 3 ported the v1.1.7-2 contracts into `contracts/` but the hand-written
`sol!` bindings in `utils/host/src/contract.rs` were left on the **pre-v117 ABI**
that v3.8.1's upstream Rust code expected (6-param `proposeL2Output` with
`_configName` + `_proverAddress`, plus an `opSuccinctConfigs` mapping for vkey
storage). The two would have compiled but every aggregation proof submission
would have reverted on-chain with a function-selector mismatch. Caught during
the pre-PR audit when comparing the Rust callers to `contracts/src/validity/OPSuccinctL2OutputOracle.sol`.

| File | Change |
|---|---|
| `utils/host/src/contract.rs` | Rewrote the `OPSuccinctL2OutputOracle` `sol!` block: dropped the `OpSuccinctConfig` struct + `opSuccinctConfigs(bytes32)` mapping in favour of three direct `bytes32 public` fields (`aggregationVkey`, `rangeVkeyCommitment`, `rollupConfigHash`); `proposeL2Output` changed from 6 params to 4 (no `_configName`, no `_proverAddress`); removed the `dgfProposeL2Output` declaration entirely — v117 ships no such function and Phase 3 removed the dispute-game implementations it would have targeted. Also fixed the casing of `updateAggregationVkey` (was `updateAggregationVKey`) and dropped the now-unused `impl opSuccinctConfigsReturn` helper. |
| `validity/src/proposer.rs` (propose path) | DGF branch (`dgf_address != Address::ZERO`) replaced with a fail-fast error: v117 contracts ship no dispute-game implementation for game type 6, so leaving the branch reachable would route real deployments into a guaranteed revert. The else-branch's `proposeL2Output` call now passes the 4-param tuple. |
| `validity/src/proposer.rs` (`validate_contract_config`) | Replaced the single `opSuccinctConfigs(config_name_hash)` mapping read with three direct field reads (`aggregationVkey()` / `rangeVkeyCommitment()` / `rollupConfigHash()`), each `.call().await?.0`. |

The `op_succinct_config_name_hash` field on `RequesterConfig` and its env-var
`OP_SUCCINCT_CONFIG_NAME` are left in place but are no longer threaded into any
contract call. They're effectively dead config on this build — a separate
cleanup pass can remove them, but doing so now would expand the blast radius
beyond what's needed to fix the ABI mismatch.

### 3.4c op-node pre-Interop compat (Phase 5 follow-up — Mantle `5efd6ead`-equivalent)

Mantle production op-node predates OP-Stack Interop and does **not** emit
`cross_unsafe_l2` / `local_safe_l2` in `optimism_outputAtBlock` responses. kona-rpc's
`SyncStatus` declares those two fields as non-optional `L2BlockInfo`, so serde
rejects the prod response at the first RPC call — the host can never advance.

origin/main's `5efd6ead` solved this by adding a parallel `utils/host/src/compat.rs`
with mirror types and swapping the call sites. This repo already maintains a local
schema copy (`utils/host/src/rpc_types.rs`, originally introduced to avoid pulling
rollup-boost / its alloy version conflict), so we **fold the relaxation into the
existing module** instead of adding a second one.

| File | Change |
|---|---|
| `utils/host/src/rpc_types.rs` | Replaced the `kona_protocol::SyncStatus` re-export with a locally-declared `SyncStatus` struct that's identical except for the two post-Interop fields: `cross_unsafe_l2: Option<L2BlockInfo>` and `local_safe_l2: Option<L2BlockInfo>`, both with `#[serde(default, skip_serializing_if = "Option::is_none")]`. `OutputResponse::sync_status` now refers to the local relaxed `SyncStatus`. No call sites needed updating — `OutputResponse::sync_status` is declared for completeness of the response shape but never read after deserialization. |

**Rollback (when Mantle ops bumps prod op-node past Interop):** swap the local
`SyncStatus` back to `kona_protocol::SyncStatus` (single import change + delete the
local struct definition). Update §7 of this file when doing so.

### 3.5 GCP HSM env-var compat (Phase 5 — Mantle `b31a31e8`/`e37cfd5a`-equivalent)

v3.8.1 upstream uses `alloy-signer-gcp` v2.0.4 with a 4-env path
(`GOOGLE_PROJECT_ID` + `GOOGLE_LOCATION` + `GOOGLE_KEYRING` + `HSM_KEY_NAME` /
`HSM_KEY_VERSION`) and relies on Application Default Credentials for auth — typically
`GOOGLE_APPLICATION_CREDENTIALS` pointing to a JSON file. Mantle production posture
forbids writing service-account JSON to disk.

| File | Change |
|---|---|
| `utils/signer/src/lib.rs` | New `Signer::from_env()` branch driven by `HSM_API_NAME` (full GCP key resource path) + `HSM_CREDENTIALS` (hex-encoded JSON service-account key). The decoded JSON is piped straight into `gcloud_sdk::TokenSourceType::Json(creds_json)` via `GoogleApi::from_function_with_token_source` — the credential never touches the filesystem. Added `parse_gcp_key_resource_path` helper to split the full path into `(project, location, keyring, key, version)`. |

The upstream `GOOGLE_PROJECT_ID` 4-env path is retained as a fallback so deployments
that can use ADC / GCE metadata service / Workload Identity Federation aren't forced
into the hex-JSON convention.

**Env var precedence in `Signer::from_env()`:**
1. `HSM_API_NAME` set → Mantle compat branch (memory-only creds)
2. `GOOGLE_PROJECT_ID` + `GOOGLE_LOCATION` + `GOOGLE_KEYRING` set → upstream branch (ADC)
3. `SIGNER_URL` + `SIGNER_ADDRESS` set → Web3Signer
4. `PRIVATE_KEY` set → local plaintext signer
5. None set → error

### 3.6 Rollup configs maintained outside this repo

Upstream `c7bfaf22 remove configs` deleted the `configs/<chain_id>/rollup.json` tree in
favor of runtime fetch via op-node's `optimism_rollupConfig` RPC. Mantle has historically
kept those JSON files in version control for offline test fixtures and air-gapped builds.

The current solution: maintain them at `~/Projects/mantle-rollup-configs/` (out-of-tree),
sourced from `mantle-xyz/op-succinct origin/main` HEAD `664a1bd4`. See that directory's
`README.md` for the chain-id list and refresh procedure.

### 3.7 SP1 ELF build (`justfile` + `--ignore-rust-version`)

`just build-elfs` runs `cargo-prove prove build --docker` for the range and
aggregation SP1 programs. Two Mantle-specific tweaks:

| File | Change |
|---|---|
| `justfile` (`build-range-elfs`) | Drop the `programs/range/celestia` and `programs/range/eigenda` invocations — Phase 2 deleted those crates (Validity-Oracle-only runtime). Only the `ethereum` block remains. |
| `justfile` (`verify-git-pins`, gating `build-elfs`) | Checks every tag-pinned git dependency in `Cargo.lock` still resolves to the commit recorded there. `Cargo.lock` pins a 40-char SHA and the `tag=` is only a lookup hint, so cargo reuses a cached commit **with no network access** — a force-pushed mutable tag (anything `-rc`, or a re-cut release) therefore builds the OLD code silently while the manifests claim the tag. Unrecoverable for ELFs, since the guest embeds the cargo-git checkout path (URL hash + short commit) and the resulting vkey maps to code nobody can identify later. |
| `justfile` (`build-range-elfs` + `build-agg-elf`) | Pass `--ignore-rust-version` to `cargo-prove`. SP1's docker image has historically shipped an older rustc than our mantle-v2 deps declare via `rust-version = "1.94"`. That build compiles those crates fine — the floor is the dep authors' MSRV declaration, not a hard requirement — so we tell cargo to skip the check. **Not yet re-tested against the SP1 v6.4.0 image; if its bundled rustc is ≥ 1.94, drop the flag from both recipes.** |

#### 3.7a ELF builds: host architecture and the cargo cache

**Apple Silicon works.** `ghcr.io/succinctlabs/sp1` publishes an amd64-only image (checked for
both v6.1.0 and v6.4.0 — identical platform lists), so on arm64 hosts the build runs through
Docker's emulation layer. That costs time, not correctness: the guest target is
`riscv64im-succinct-zkvm-elf` (cross-compiled — the committed ELFs are 64-bit RISC-V; note
`cargo-prove` still carries a legacy `CFLAGS_riscv32im_...` env var, which is not the build
target), and the in-container environment is identical regardless of host — which is precisely
what `--docker` is for ("reproducible builds"). ELFs have in fact been produced this
way on an arm64 machine. `.github/workflows/elf.yml` rebuilds on x64 and requires
`git status --porcelain elf/` to be empty, so CI remains the final arbiter if some host ever
does diverge.

**Docker caching is on by default** — the named volumes `sp1-cargo-git` /
`sp1-cargo-registry`. `cargo-prove --no-docker-cache` disables them (cache then lives in the
container's ephemeral layer and is discarded on exit). This only changes download time, never
the artifacts: cargo resolves each git dependency by the commit SHA in `Cargo.lock` whether it
comes from the volume or the network. If the named volumes do not exist, Docker creates them
empty and the build simply downloads everything — an empty cache has never been an error, and
the volumes accumulate incrementally across runs (they are writable and persistent, so they also
grow without bound; deleting them occasionally is healthy).

**The private-repo workaround is no longer needed.** `cargo-prove prove build --docker` mounts
its cargo caches as the named volumes `sp1-cargo-git` / `sp1-cargo-registry`, forwards no
credentials (no ssh-agent, no .gitconfig, no token) and the image has no `git` CLI. While the
revm patch pointed at the private `mantle-xyz/revm-ghsa-5vfr-x84h-hmvf` fork, the container hit
`failed to authenticate`, and the workaround was to pre-seed the volume from a locally
authenticated clone:

```bash
docker run --rm -v sp1-cargo-git:/v -v "$HOME/.cargo/git":/host:ro alpine sh -c 'cp -a /host/db /v/'
```

All git sources are now anonymously reachable (`mantle-xyz/mantle-v2`, `mantle-xyz/revm`, the
`sp1-patches/*` forks, `succinctlabs/sp1-cluster`), and the private fork is absent from both
`Cargo.toml` and `Cargo.lock` — the lockfile matters as much, since that is what the container
resolves. So `just build-elfs` works with no volume seeding.

Seeding the **registry** volume remains a legitimate speed-up for a cold build; seeding the
**git** volume is now purely optional. Prefer starting clean when the ELFs are going on chain:

```bash
docker volume rm sp1-cargo-git sp1-cargo-registry   # or: --no-docker-cache
```

A warm git cache is exactly what makes a moved tag invisible (cargo never reaches the network for
a commit it already has), which is why `verify-git-pins` gates `build-elfs`.

### 3.8 Toolchain pins — `rust-toolchain.toml` + `mise.toml`

| File | Pin | Why |
|---|---|---|
| `rust-toolchain.toml` | `nightly-2026-02-15` (rustc 1.95-nightly) | Upstream v3.8.1 pinned `nightly-2025-09-15` (rustc 1.92-nightly). After Phase 2 swapped deps to mantle-v2, those crates' `rust-version = "1.94"` declaration started rejecting 1.92-nightly. Bumped to 1.95-nightly which keeps the `rustc-dev` component build scripts need. |
| `mise.toml` | `forge = cast = anvil = "1.4.3"`, `svm-rs = "0.5.19"` | Upstream v3.8.1's `bindings/build.rs` calls `forge bind` to generate `bindings/src/codegen/` (gitignored). Without forge on PATH, build.rs prints a warning and skips generation, then `lib.rs:7 mod codegen;` fails to find the module. `forge bind` from 1.2.x generates alloy-0.x-flavoured Rust (3-arg `RawCallBuilder`, the old `Transport` trait) which won't compile against this workspace's alloy 2.0.4 deps — pin **1.4.x** instead (mantle-v2/mise.toml's 1.2.3 stays because mantle-v2 has no `bindings/` crate). `rust` is intentionally NOT pinned here — `rust-toolchain.toml` already drives it and a mise rust pin would silently override (we hit exactly this with mantle-v2's mise.toml earlier). |

### 3.9 Validity proposer — relay-rejection handling and transport-fault classification

Two independent problems in the aggregation submission path, neither of which upstream addresses.
Upstream's `relay_aggregation_proof` is **byte-for-byte identical from v3.8.1 through v4.6.1**
(compared via the GitHub contents API at those tags — the local `upstream` remote's tag set predates
v3.8.1, so `git show v4.6.1:...` cannot reproduce this; run `git remote update upstream` first): a
revert returns `Err`, the main loop logs it and sleeps 10s, and the next pass resubmits the same
proof unchanged. Because `Complete` counts toward `fetch_active_agg_proofs_count`, a genuinely
invalid proof there is a permanent stall — observed on QA3, where an aggregation sat `Complete` for
two days while 486 range proofs piled up ~90k blocks ahead of a frozen contract head.

**a. Relay rejections are classified from typed revert data, and rebuilt unless proven harmless.**

| File | Change |
|---|---|
| `utils/host/src/contract.rs` | Added the oracle's `error L1BlockHashNotCheckpointed()` / `error L1BlockHashNotAvailable()` declarations, plus a separate `sol! { interface SP1Verifier { error InvalidProof(); error InvalidExitCode(); } }`. Without these the generated `OPSuccinctL2OutputOracleErrors` enum is empty and no revert can be decoded. `InvalidExitCode()` is not in the vendored `sp1-contracts` copy (the deployed verifier is newer); it is declared from its selector `0x1fcf9177`, observed on QA3. |
| `validity/src/relay_rejection.rs` (new) | `classify_revert_data`: a pure function over `&Bytes` returning `RelayRejection` (`ProofRejected` / `CheckpointUnusable` / `UnsatisfiableGuard` / `RebuildableGuard` / `ContractPanic` / `UnknownRevert` / `NoVerdict`). `require(cond, "…")` is decoded via `alloy_sol_types::Revert` back to the original string; every other surface by 4-byte selector. Also `revert_data_of_rpc_error`, which reads the `data` field with `try_data_as` rather than alloy's `as_revert_data()` — the latter first checks `message.contains("revert")`, so a client answering `"VM execution error."` (Nethermind) would hide a real verdict. |
| `validity/src/proposer.rs` (`relay_aggregation_proof`) | Returns `RelayOutcome::{Relayed, Rejected}` instead of `Result<B256>`. A rejection is an outcome, not an error: `Err` is now reserved for a transaction that was never delivered (nonce, funds, dead RPC), the only case where retrying unchanged is right. Pre-flight rejections are read straight off the send error — the common path, since alloy's gas filler runs `eth_estimateGas` first and a deterministic revert never reaches a block. A mined-and-reverted transaction is replayed as `eth_call` at its own block (a receipt carries no revert data), bounded by `NETWORK_CALLS_TIMEOUT`. The two decisions are pure functions — `send_outcome(Result<TransactionReceipt>)` and `replay_verdict(Result<Result<Bytes, RpcError>, Elapsed>)` — because inside the `await` they were unreachable from any test; a mutation pass confirmed both were then free to silently degrade (e.g. dropping the `receipt.status()` check, which would mark a reverted proposal as relayed). |
| `validity/src/proposer.rs` (`handle_relay_rejection`) | One `warn!` per rejection class carrying the decoded reason, the selector in hex, and an explicit operator ACTION. Only `UnsatisfiableGuard` keeps the request `Complete`; everything else — including unrecognised selectors and unrecoverable reasons — transitions to `Failed` so the next loop builds a replacement. Wasting one proof is strictly preferable to freezing the head. All paths return `Ok(())`, so the loop keeps `LOOP_INTERVAL` and `update_chain_lock` still runs. |
| `validity/src/proposer.rs` (`run_loop_iteration`) | Three changes a sync must not undo. **Order**: `submit_agg_proofs` runs BEFORE `create_aggregation_proofs`. This is required, not a preference: `handle_relay_rejection` moves the rejected row from `Complete` to `Failed` within this same pass, and `fetch_active_agg_proofs_count` counts `Complete` while `fetch_failed_agg_request_with_checkpointed_block_hash` reads `Failed` — so running submit first is what lets one pass both reject and rebuild. Create-then-submit costs a full `LOOP_INTERVAL` per rejection. **Per-pass relay floor**: that order opens a window in the *success* case, since `fetch_active_agg_proofs_count` excludes `Relayed` (deliberately — see `db/client.rs`) and an L1 read endpoint lagging behind our own transaction would report a head that does not yet include the relay, so the pass would build a duplicate aggregation over a range already proposed and waste it on the inevitable revert. `submit_agg_proofs` therefore returns the `end_block` it relayed, and `create_aggregation_proofs` compares the head it reads against it (`create_should_run`), skipping the pass when the head is behind. An earlier version simply deferred create for one `LOOP_INTERVAL` after any successful relay; that only narrowed the window to a lag shorter than one interval — beyond it the row is already `Relayed`, `fetch_completed_agg_proof_after_block` no longer returns it, and the next pass creates the duplicate anyway — while costing throughput on every aggregation. Comparing against the floor closes it at any lag and costs nothing when the endpoint is current. The floor is deliberately per-pass: an L1 reorg can drop a confirmed proposal, and a remembered floor would keep the proposer building from a start block the contract never reached. **Error policy**: both of these steps — the only two that broadcast an L1 transaction — log and continue instead of propagating, because an L1 revert that waits on the chain (`blockhash()`'s 256-block window for `checkpointBlockHash`, `TX_CONFIRMATION_TIMEOUT` under congestion for `proposeL2Output`) says nothing about whether the rest of the pass can progress, while aborting skips `request_queued_proofs` (range proofs stop being produced) and `update_chain_lock` (whose lease is exactly `LOOP_INTERVAL`). Every other step still propagates. |
| `validity/src/prom.rs` | `succinct_agg_proof_blocked_by_contract_guard` (0/1, set on every `submit_agg_proofs` pass so it clears itself) and `succinct_agg_proof_rebuilt_after_rejection_count`. |

`UNSATISFIABLE_GUARDS` lists only the four `require`s that no proof can satisfy; anything else —
including a `require` added upstream — falls through to `RebuildableGuard`, which wastes a proof
rather than stalling. `every_guard_on_the_validity_path_is_classified_exactly_once` reads
`contracts/src/validity/OPSuccinctL2OutputOracle.sol` with `include_str!` and asserts SET EQUALITY
between the guards reachable from `proposeL2Output` (its body plus the `whenNotOptimistic` modifier it
applies) and `UNSATISFIABLE_GUARDS ∪ {the rebuildable one}`. Set equality rather than substring or
count checks, each of which a broken list satisfied: substrings accept a message the contract has
since extended, a whole-file search is satisfied by the optimistic-mode overload's copy after the
validity one is reworded, and a count cannot see a member removed from the list.

**No schema change.** No migration, no new `RequestStatus`. A rejected aggregation still goes to
`Failed`.

**No rebuild cap, deliberately.** An earlier attempt bounded rebuilds with `MAX_AGG_REGENERATIONS`
plus a time window over `COUNT(*)` of `Failed` rows. Every cap of that shape recreates an absorbing
state: `Failed` rows are never deleted and the count only resets when an aggregation lands, which is
precisely what the cap prevents — so an operator who fixed the root cause still had to edit the
database by hand, and deploying onto an already-stuck proposer tripped the cap on its first pass. The
window meant to release it then had to outlast the rebuild cycle, which nothing guarantees. Bounding
the cost is therefore left to observation (`succinct_agg_proof_rebuilt_after_rejection_count` plus a
per-class `warn!` with an operator ACTION) rather than to a mechanism that can wedge the chain.
Note the rebuild cycle is one aggregation witnessgen+prove, not `PROVING_TIMEOUT`: a rejection frees
the `fetch_active_agg_proofs_count` slot immediately, so with submit running before create the
replacement is built in the SAME pass.

**b. Transport faults no longer bisect a healthy range** (`e315e944`, `3ab9c5a1`).

| File | Change |
|---|---|
| `validity/src/proposer.rs` | `is_transient_transport_error`: a gRPC `UNAVAILABLE` means the prover backend was unreachable, not that the proof failed, so the range is retried unchanged instead of being bisected. Classified by **typed** `tonic::Status` code via `downcast_ref` (the sp1-sdk's own `retry.rs` does the same), with the old string match kept only as a fallback for errors that are not a downcastable `Status`. |
| `validity/Cargo.toml` | Pinned `tonic = "0.12"` (default-features off) to match the sp1-sdk's tonic — `Cargo.lock` carries four tonic versions, and a mismatch would make the downcast silently return `None`, i.e. everything bisects. |

New dependencies on `validity`: `alloy-transport` (downcasting to `RpcError`), `alloy-rpc-types-eth`
(the replay request/receipt types), and `alloy-json-rpc` as a dev-dependency (building an
`ErrorPayload` to pin that classification never consults the client's message wording).

**Known test gap: the wiring, not the decisions.** Each decision on this path is a pure function
with a table test behind it — `classify_revert_data`, `rejection_action`, `guard_gauge_value`,
`send_outcome`, `replay_verdict`, `checkpoint_plan`, `select_checkpoint_block_number`,
`create_should_run` — and repeated mutation passes have killed every mutation inside them, including
the ones against the contract-source scans and the SQL predicates.

What survives is the handful of lines connecting those functions to `Proposer`, which cannot be
reached without mocking a fetcher, a signer, a contract and a database. Specifically: whether
`handle_relay_rejection` applies `rejection_action`'s verdict at all; whether `submit_agg_proofs`
calls it; how the four `SendOutcome` variants map onto `RelayOutcome`; whether #923's floor reaches
`select_checkpoint_block_number`; and whether `submit_agg_proofs` reports the `end_block` it relayed
rather than `None`. Deleting any one still leaves the suite green.

The boundary is worth stating precisely, because it was drawn wrong once: the per-pass relay floor
was first written as a bare `if` in `run_loop_iteration` and filed under this gap. It did not belong
here — inverting that `if` stops the proposer permanently and silently, and covering it needed a
pure function, not a mock. Anything expressible as a function of its inputs belongs above this
paragraph. **If this path is reworked, re-check the call sites listed here by hand.**

### 3.10 succinctlabs/op-succinct#923 (agg checkpoint anchored to `safe`) — **now upstreamed**

**Status: resolved by the v3.12.0 sync. No `[UPSTREAM #923]` markers remain in the tree** (that
grep returning zero hits is the invariant; the v3.12.0 merge took upstream's copy and downgraded
what we kept to `[MANTLE]`). This section is retained because two pieces of the backport
*survived the sync as Mantle-owned code*, and because the hazards below recur on any sync that
lands a PR we had already backported by hand.

What happened to each piece:

| Piece | Outcome in the v3.12.0 sync |
|---|---|
| `select_checkpoint_block_number` | **Dropped ours, took upstream's** — body and signature were verbatim identical, only our doc comment differed. |
| `checkpoint_plan` / `CheckpointPlan` / `RecheckpointReason::BelowBatchMaxL1Head` | **Kept ours**, marker downgraded to `[MANTLE]`. Behaviour is upstream's, shape is ours. |
| `mod checkpoint_plan_tests` (6 tests) | **Kept ours**, downgraded to `[MANTLE]`. |
| `mod checkpoint_selection_tests` (3 tests) | **Dropped ours**, took upstream's three equivalents in `mod tests`. |
| `get_max_l1_head_block_number_for_range` | **Took upstream's**, including the `invalidated_at IS NULL` clause that #951 added — that column now exists (see §3.11). |
| `test_get_max_l1_head_block_number_for_range*` | **Kept ours** (no upstream equivalent), repointed at upstream's function, downgraded to `[MANTLE]`. |

**The hazard worth remembering: `checkpoint_plan_tests` is easy to delete by mistake.** It carried
the `[UPSTREAM #923]` marker but has no upstream counterpart — upstream implements the reuse gate
as an inline `if/else`, which nothing can test, so upstream's coverage of it is zero. A
grep-and-drop pass over the marker would have silently taken our coverage to zero too. Returning
the `anchor` from a pure function rather than reading it inline is what makes the behaviour
testable at all: a mutation pass found that changing it back to `BlockId::latest()` — the entire
bug #923 fixes — left the whole suite green.

Upstream PR: `succinctlabs/op-succinct#923` by Farhad-Shabani, merged 2026-06-05, released in
upstream **v3.10.0**. Its own description records the failure as having **"Hit 3× on Mantle"** — it
was written for our chain, we backported it by hand, and the v3.12.0 sync then absorbed it.

The proposer checkpointed `BlockId::latest()`. The checkpoint head is pinned **by hash** while the
aggregation guest's header range is fetched **by number**, so a tip reorg between the two orphans
the checkpoint, the guest's `assert_eq!` (`programs/aggregation/src/main.rs`) rejects the input, and
one aggregation proof is wasted.

| File | Change |
|---|---|
| `validity/src/proposer.rs` (`checkpoint_plan`) | **Behaviour is upstream's; the shape is ours — kept through the sync.** #923 gates checkpoint *reuse* on the batch's max `l1Head` — a cached checkpoint below it is discarded, because a matching on-chain hash only proves the block was not reorged out between writing the row and the checkpoint transaction executing, not that the guest can reach every range proof's `l1Head` from it. Upstream writes that as an inline `if/else`; we keep it as a pure function returning `CheckpointPlan::{Reuse, Fresh{anchor, reason}}`. |
| `validity/src/db/client.rs` | `get_max_l1_head_block_number_for_range` is now upstream's, verbatim. Note it is a **runtime `sqlx::query_scalar`, not the compile-time `sqlx::query!` macro** that #923 originally shipped: the macro form needs a `.sqlx` cache entry, and `validity/.sqlx/` has none for this query, so `SQLX_OFFLINE` would fail immediately. Do not "restore" the macro form on a later sync. Its WHERE clause must stay in sync with `get_consecutive_complete_range_proofs` so the MAX covers exactly the range proofs the aggregation consumes. |

### 3.10a No-bisect failure policy and the observability firewall

Two rules the proposer must keep, both found by reviewing the v3.12.0 sync.

**1. Failures that say nothing about the range must not bisect it.**
`no_bisect_reason()` in `validity/src/proposer.rs` is the single policy gate; every failure path
consults it instead of classifying inline. Three classes qualify, and they share one property —
**no proof was ever produced**, so nothing was learned about the range:

| Class | Predicate | Typical cause |
|---|---|---|
| Admission shed | `is_admission_shed_error` | self-hosted prover pool momentarily full |
| Transient transport | `is_transient_transport_error` (gRPC `UNAVAILABLE`) | gateway down, connection reset |
| Unsatisfiable precondition | `is_unsatisfiable_precondition_error` (gRPC `FAILED_PRECONDITION`) | **no program registered for our vk_hash** — i.e. the deployed ELF was never registered with the cluster, the predictable failure right after a vkey change |

These reset the row to `Unrequested` and retry the SAME range. Bisecting them is not merely
useless, it is harmful: each split doubles the request volume aimed at a backend that rejects all
of it, and the fragmentation is **not undone** once the condition clears, so the range is proved
as many small pieces forever after.

Note the alternative is not "give up". A `Failed` range is not in the active set
`add_new_ranges` reads, so it is re-created as a gap next pass regardless — bisecting only
changes the *shape* of the retry, for the worse.

Both paths that can reach `handle_failed_request` from a classifiable error consult the gate:
the task-failure path in `handle_ongoing_tasks`, and the cluster-poll path via
`reset_cluster_request_for_retry`. The remaining two call sites are not classifiable — a task
panic carries no `tonic::Status`, and `handle_terminal_proof_failure_before_request_details`
dispatches on SP1 *fulfillment* status rather than a transport error. (For `Unfulfillable`
specifically, the fix belongs on the cluster side: a backlogged prover must not report a request
unprovable.)

**2. Observability must never be able to stop the proposer.**
`log_proposer_metrics` only reads and sets gauges, but it used to propagate with `?` as step 2 of
the loop. It calls `highest_contiguous_end`, which returns `Err` for a completed range that
overlaps the contiguous chain or has `end <= start`. Such a row is not self-healing, so a single
one failed the iteration at step 2 forever — skipping the scheduling gate, delivery, **and**
`update_chain_lock`, with `run` pinned to its 10s error path. It is now logged and skipped. The
same bad row is still surfaced by `create_aggregation_proofs`, which is where it actually blocks
progress.

The upstream helper this replaced (`highest_proven_contiguous_block`) returned `Option` and never
errored, silently truncating at the overlap — so this strictness is new in v3.12.0. No code path
creates such rows (`find_gaps` keeps new ranges disjoint, and bisection cannot produce an empty
range since it requires `end - start > 1`), so a hit means historical bad data or manual
intervention. Pre-upgrade check:

```sql
WITH visible AS (
  SELECT id, start_block, end_block FROM requests
  WHERE status = 4 AND req_type = 0 AND invalidated_at IS NULL
    AND range_vkey_commitment = ? AND rollup_config_hash = ?
    AND l1_chain_id = ? AND l2_chain_id = ?
    AND start_block >= ?          -- contract latestBlockNumber
)
SELECT a.id, b.id FROM visible a JOIN visible b          -- overlap (over-reports:
  ON a.id < b.id                                          -- the code only errors on the
 AND a.start_block < b.end_block AND b.start_block < a.end_block;   -- contiguous chain)
SELECT id, start_block, end_block FROM visible WHERE end_block <= start_block;  -- empty/reversed
```

### 3.11 Upstream sync v3.8.1 → v3.12.0

22 upstream commits. What we took, what we held, and the three things that are easy to get wrong.

**Held deliberately (the `[MANTLE]` decisions in this sync):**

| Item | Upstream v3.12.0 | Ours | Why |
|---|---|---|---|
| alloy core/network family | `2.0.5` | **`2.0.4`** | These are caret requirements, so `2.0.5` lets cargo resolve the family to the newest `2.x` — `2.4.1` at the time of this sync. `alloy-genesis >= 2.4.1` adds a `bogota_time` field to `ChainConfig` that mantle-v2's vendored `kona-registry` v1.6.0 never initializes, and the workspace stops compiling. Upstream escapes this only because its own lockfile pins the family lower. **The alloy family moves when mantle-v2's kona moves, not before.** |
| `gcloud-sdk` | `0.29` + explicit `tls-webpki-roots` | **`0.27`** | Upstream moved it only to match `alloy-signer-gcp 2.0.5`. Tied to the row above — bump both together or neither. |
| `kona-*` / `op-alloy-*` / `alloy-op-evm` | `ethereum-optimism/optimism` tag `kona-client/v1.6.0` | `mantle-xyz/mantle-v2` tag `v1.6.0` | Same kona version, different source. This is the largest conflict block of any sync and the resolution is always mechanical "keep ours". |
| revm family | crates.io `38.0.0` / `34.0.0` | `mantle-xyz/revm` @ tag `v107-mantle-arsia.1` | Carries the Mantle protocol changes. Everything under `utils/client/src/precompiles/` is adapted to *this* revm — including `OpSpecId::ARSIA`/`OSAKA` where upstream has `KARST` — so those files are "keep ours" wholesale. |
| `alloy-primitives` | resolves `1.6.0` | resolves **`1.5.7`** | 1.6.0 raises its `sha3` requirement from `0.10.8` to `0.11`, and it sits in the same version lockstep as the kona family. Holding it is what keeps the `sha3` patch tag valid — see the trap below. |
| `metrics-exporter-prometheus` | `0.16.2` | **`0.18`** | Two versions in one binary race over the single global `metrics` recorder and freeze `/metrics`. One-line conflict, silent failure mode — see the note in `Cargo.toml`. |
| `alloy-chains` / `c-kzg` | `0.2.30` / `2.0.0` | `0.2.33` / `2.1.5` | We are ahead; no reason to regress. |
| `AggregationOutputs` | 7 fields (adds `proverAddress`) | **6 fields** | A 7th field makes the program commitment 224B while the v117 contract decodes 192B → on-chain `InvalidProof (0x09bde339)`. Upstream did not touch `utils/client/src/types.rs` in this range so it merges clean, but **`scripts/prove/bin/agg.rs` is touched by both sides** — check it every sync. |
| `altda` DA backend | added (crates, ELF, Dockerfile, CI job, e2e) | **dropped** | Same shape as the EigenDA/Celestia removal in Phase 2: this fork is Validity-Oracle-only (§3.1). Dropped `utils/altda/**`, `programs/range/altda`, `elf/altda-range-elf-embedded`, `validity/Dockerfile.altda`, the `altda` features, the CI job, and the `test-e2e-sysgo-altda` recipe. |
| run loop order | `create` → `submit` | **`submit` → `create`** | Load-bearing, see §3.9 and the `[MANTLE]` comment in `run_loop_iteration`. Reverting costs a full `LOOP_INTERVAL` per relay rejection. |

**Bumped alongside this sync (our own changes, not upstream's):**

- **kona family `v1.6.0` → `v1.6.1-rc0`** (`mantle-xyz/mantle-v2`, 25 entries).
- **revm family moved off the private GHSA fork.** Was
  `mantle-xyz/revm-ghsa-5vfr-x84h-hmvf` @ rev `99707e9f` — a stopgap fork opened to carry the
  GHSA-5vfr-x84h-hmvf fix. Now `mantle-xyz/revm` @ tag `v107-mantle-arsia.1` (16 entries).
  Besides being the proper home for the fix, the old fork was **private**, which broke
  `just build-elfs`: the SP1 docker build has no credentials for it.

These two move **in lockstep** — `alloy-op-evm` comes from mantle-v2 while the revm crates come
from mantle-xyz/revm, and a split leaves two `alloy_evm`/`op-revm` versions in the graph, which
surfaces as `OpSpecId` / `OpHaltReason` / `OpEvm` type mismatches against the kona executor.
Verify after any bump:

```bash
grep -c '^name = "alloy-evm"' Cargo.lock      # want 1
grep -c '^name = "alloy-op-evm"' Cargo.lock   # want 1
grep -c '^\[\[patch.unused\]\]' Cargo.lock    # want 0
```

Note this alone changes both vkeys even with identical source bytes: the guest ELF embeds the
cargo-git checkout path, which is derived from the dependency URL and commit.

**Taken from upstream:**

- **#951/#952** — `invalidated_at` column, `RequestStatus::Invalidated = 8`, and
  `reconcile_completed_range_canonicality()`, which recovers from range proofs whose recorded L1
  head was orphaned. Its gate now wraps our whole scheduling/delivery block: while the legacy
  `l1_head_block_hash` backfill is running it returns `false`, and acting on unverified range
  proofs is exactly what it exists to prevent, so `submit_agg_proofs` is gated with the rest.
  **See the deployment note in §3.11.1.**
- **#924** — a real security fix. `kzg-rs` returns `Ok(false)` for a well-formed but *invalid*
  KZG proof; our `verify_kzg_proof` used `.map_err()`, which only handles `Err`, so an invalid
  proof was silently accepted. Now only `Ok(true)` passes. (GHSA-pq4w-5vv8-gxhr.)
- **SP1 6.1.0 → 6.4.0** and sp1-cluster `v2.1.5` → `v2.7.2`. **Changes both vkeys.**
  Note the `sha3` patch tag is *not* part of this bump — see trap 4 below.
- `highest_contiguous_end()`, which replaces our `highest_proven_contiguous_block()`. Upstream's
  version is stricter (it errors on overlapping and empty/reversed ranges instead of silently
  taking the max) and both call sites moved to it, leaving ours dead — so ours and its
  `contiguous_block_tests` module were removed rather than kept as dead code.
- `get_max_provable_l2_block_number()` (renamed from `get_finalized_l2_block_number`) and the new
  `utils/host/src/l1_selection.rs` (`L1_BLOCK_TAG` / `L1_CONFIRMATIONS`).
- The `forge build` step added to `bindings/build.rs` (#947/#948), which is orthogonal to our
  `--skip test/**`.
- `rust-toolchain.toml` → `nightly-2026-05-15`; it is a superset of our 1.94 floor.

**Three traps, all of which bit or nearly bit during this sync:**

1. **The migration numbers collide and git does not say so.** Upstream's
   `05_add_request_invalidation.sql` and our `05_add_requests_indexes.sql` are different
   filenames, so git merges both without a conflict, but sqlx indexes by version number and the
   two cannot coexist. **Upstream's was renamed to `06_`; ours stays at `05`** — deployed
   databases already recorded `05` = our index migration's checksum, and changing it makes every
   running instance fail its migration check at startup. `RequestStatus::Invalidated = 8` is
   appended at the end of the enum, so no existing `status` value shifts.
2. **`Cargo.lock` must not be resolved from scratch.** Base it on ours and let cargo update
   incrementally; a free resolution walks the alloy family up to `2.4.1` and breaks the build
   (see the alloy row above).
3. **`checkpoint_plan_tests` has no upstream counterpart** — see §3.10.
4. **The `sha3` patch tag is coupled to `alloy-primitives`, not to SP1.** Its tag name ends in
   `-sp1-6.0.0`, which makes it look like it should move with the SP1 bump. It should not.
   `sha3-keccak` routes alloy's `keccak256()` through the sha3 crate, and the patched fork is
   what makes that an SP1 precompile syscall instead of software keccak — so the tag has to
   match whatever sha3 version `alloy-primitives` requires:

   | alloy-primitives | requires | correct patch tag |
   |---|---|---|
   | 1.5.x (**ours**) | `sha3 "0.10.8"` | `patch-sha3-0.10.8-sp1-6.0.0` |
   | 1.6.0 (upstream v3.12.0) | `sha3 "0.11"` | `patch-sha3-0.11.0-sp1-6.0.0` |

   sha3 is a 0.x crate, so 0.11 is **not** a semver-compatible substitute for a 0.10.x
   requirement. Taking upstream's 0.11.0 tag while holding alloy-primitives at 1.5.x makes
   cargo skip the patch entirely (`warning: patch ... was not used in the crate graph`) and
   silently fall back to unaccelerated keccak — it still compiles and is still correct, just
   more expensive to prove. **Upstream is not wrong here; the two settings simply have to
   agree.** Guard: `grep -c '^\[\[patch.unused\]\]' Cargo.lock` must be `0`. And cargo will
   not re-resolve sha3 on its own if the locked version still satisfies the requirement —
   force it with `cargo update -p sha3 --precise <version>`.

#### 3.11.1 Deployment notes for this sync

1. **On-chain vkeys must be updated.** SP1 6.4.0 changes both the range and aggregation vkeys;
   update the v117 oracle's `aggregationVkey` and `rangeVkeyCommitment` (the setter is
   `updateAggregationVkey` — note the casing). The concrete on-chain update procedure is a known
   gap in this document.
2. **The self-hosted proving cluster must be upgraded in lockstep** to a version compatible with
   sp1-cluster client `v2.7.2`. This is the only cross-system dependency in this sync.
3. **Expect a proposer stall on first start.** Every existing `Complete` range proof predates
   `l1_head_block_hash`, so `reconcile_completed_range_canonicality` backfills them at
   `RANGE_METADATA_HYDRATION_LIMIT` (100) rows per loop and returns `false` until it finishes —
   which means the entire scheduling and delivery block is skipped for that whole period.
   Estimate the backfill time from the row count before deploying, and pick a low-traffic window.
4. **Rollback hazard:** once any row is written with `status = 8` (`Invalidated`), reverting to
   pre-v3.12.0 code will panic in `RequestStatus::From<i16>`, which has no arm for 8. Before
   rolling back, move those rows to another status.
5. **Register the new ELFs with the proving cluster before starting the proposer.** Both vkeys
   change in this sync, and an unregistered program makes the cluster reject every request with
   `FAILED_PRECONDITION: program not registered for vk_hash <...>`. That is now classified
   no-bisect (§3.10a), so the proposer retries whole ranges and recovers by itself once the
   programs are registered — but it produces nothing until then.
6. `validity/Cargo.toml`'s `tonic` pin exists so `is_transient_transport_error` can downcast to
   the same `tonic::Status` type the pinned sp1-sdk uses. If SP1 6.4.0 pulls a different tonic,
   re-verify that downcast — a silent mismatch sends transient transport faults back into range
   bisection (§3.9).

## 4. Sync workflow

When a new upstream Succinct Labs release lands (e.g. v3.9.0, v4.0.0):

### 4.1 Pre-sync dry-run

```bash
git remote update upstream
git checkout -b sync-dryrun-v<X.Y.Z> origin/main
git merge --no-commit --no-ff v<X.Y.Z>
git diff --name-only --diff-filter=U   # list conflicting files
grep -l "\[MANTLE\]" $(git diff --name-only --diff-filter=U)  # files with our markers
git merge --abort
```

### 4.2 Sync run

```bash
git checkout -b mantle/op-succinct-v<X.Y.Z> origin/main
git merge v<X.Y.Z>
# resolve conflicts — `[MANTLE]` comments mark every site we touched
# `[UPSTREAM #nnn]` marks a BACKPORT: if the sync target already contains that PR,
# DELETE our copy and keep upstream's rather than merging the two. Check §3 for the
# per-site verdict first — not every marked test is ours to drop (see §3.10).
```

Two things git will NOT flag for you:

- **`validity/migrations/`** — a new upstream migration can reuse a version number we already
  took. Different filenames, so the merge is clean, but sqlx indexes by number and deployed
  databases have already recorded a checksum for ours. Renumber *upstream's* file; never ours.
- **`Cargo.lock`** — take ours (`git checkout --ours Cargo.lock`) and let cargo update it
  incrementally. Resolving it from scratch lets caret requirements walk whole dependency
  families to their newest majors; §3.11 has the case where that broke the build.

Resolve order:

1. Hot-spot files in §6 first (proposer / fetcher / contract.rs / signer/lib.rs).
2. Hunks inside `[MANTLE]` comment blocks: keep ours unless upstream's intent is clearly
   a superset (e.g. upstream now also covers our bug fix — drop ours then).
3. `Cargo.toml` + `[patch.crates-io]`: deps refresh is a separate decision; usually
   keep our patch entries and bump the mantle-v2 rev to the latest matching the new
   upstream kona/alloy/revm major versions.
4. `contracts/`: only resolve if upstream changed a file that v117 also changed; v117
   wins. Otherwise prefer upstream.

### 4.3 Verification

```bash
TOOLCHAIN=$(grep channel rust-toolchain.toml | cut -d'"' -f2)

# 1. Workspace cargo build (release; matches what CI checks)
RUSTUP_TOOLCHAIN=$TOOLCHAIN cargo build --workspace

# 2. Contracts compile (Phase 3 verified this against v117 + mantle-v2 lib/optimism)
cd contracts && forge build

# 3. [MANTLE] marker audit
grep -rn "\[MANTLE\]" . --include="*.rs" --include="*.toml" --include="*.sol" \
  | grep -v "target/" | grep -v "contracts/lib/" | wc -l
# Expect the count to match this file's §3 entry count (or higher if you added new ports).
```

### 4.4 Land the sync

```bash
git push -u origin mantle/op-succinct-v<X.Y.Z>
# Open a PR against origin/main, get review, then update §1 baseline in this
# file and merge. (Do not name a release branch the same as its tag — that
# creates ambiguous refs.)
```

## 5. Cold-build checklist (fresh machine)

Setup checklist for a brand-new dev box / CI runner / production server. Steps 5.1
are one-time per machine; 5.2 is per-clone; 5.3 is the build itself; 5.4 collects
the symptoms we've actually hit on fresh Linux servers + how to clear each one.

### 5.1 One-time machine setup

```bash
# mise — version manager that drives forge / cast / anvil / svm-rs from mise.toml
curl https://mise.run | sh
echo 'eval "$(~/.local/bin/mise activate bash)"' >> ~/.bashrc    # or zsh / fish
source ~/.bashrc                                                 # or re-login

# rustup — auto-installs the nightly pinned by rust-toolchain.toml on first cargo
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
```

### 5.2 Per-clone setup

```bash
git clone --recursive https://github.com/mantle-xyz/op-succinct.git
cd op-succinct
git checkout mantle/op-succinct-v3.8.1

# If you forgot --recursive on clone:
git submodule update --init --recursive --depth 1

mise trust
mise install        # forge 1.2.3 / cast 1.2.3 / anvil 1.2.3 / svm-rs 0.5.19
```

### 5.3 Build

```bash
cargo build --workspace
```

First build pulls every crate and may take 15-30 min depending on network and CPU.
Subsequent incrementals are seconds.

### 5.4 Symptoms we've hit on fresh Linux servers + fixes

| Symptom | Root cause | Fix |
|---|---|---|
| `rustc 1.92.0-nightly is not supported by ... requires rustc 1.94` (mantle-v2 deps) | The pinned nightly (`nightly-2026-02-15`) was never installed locally and rustup auto-install didn't run | `rustup toolchain install nightly-2026-02-15 -c llvm-tools,rustc-dev,rustfmt,clippy` |
| Build succeeds locally but fails on fresh machine with the same Cargo.toml | Some `rustup override` was set on your local box at some point and silently bypassed `rust-toolchain.toml` | `rustup override list` to inspect; `rustup override unset --path <dir>` to fix |
| `error[E0583]: file not found for module 'codegen'` + warning `Forge not found in PATH. Skipping bindings generation.` | mise not installed or `mise install` not run; `forge` missing | install mise per §5.1, then `cd op-succinct && mise install` |
| `Error: failed to resolve file: ".../contracts/lib/sp1-contracts/contracts/src/SP1MockVerifier.sol": No such file or directory` (or similar for `solady`, `openzeppelin-contracts`, etc.) | git submodules never initialised on this clone | `git submodule update --init --recursive --depth 1` |
| `git submodule update --init` prompts for `Username for 'https://github.com'` | One of the submodule URLs points at a private repo and the machine has no GitHub credentials. Phase 5 removed `mantle-xyz/mantle-cdk` (the only private one) — if you see this on a fresh clone of `mantle/op-succinct-v3.8.1` post-Phase-5, you're on an older commit. `git pull` first. | `git pull` to land Phase 5 cleanup, or set up a GitHub PAT in `~/.netrc` / git credential helper if you intentionally re-added a private submodule |
| `Error (6275): Source "@safe-contracts/contracts/common/Enum.sol" not found` from `forge bind` | v117 left dangling imports in `contracts/test/helpers/Utils.sol`. Phase 5 dropped the two unused imports; this only resurfaces if someone edits that file and re-adds them. | Drop `Safe` / `Enum` imports from `Utils.sol` (they're never used); see §3.2 row. |
| `bindings/src/codegen/*.rs` won't compile — errors like `cannot find trait 'Transport' in module 'alloy_contract::private'`, `RawCallBuilder` takes 2 generics not 3, `abi_decode_returns` has 1 parameter not 2 | `forge bind` from forge 1.2.x generates alloy-0.x-flavoured Rust; workspace uses alloy 2.0.4. PATH has a forge older than 1.4. | `mise install forge@1.4.3` (mise.toml already pins 1.4.3 — typically caused by a system Foundry from `~/.foundry/bin` shadowing mise's pin: `which forge` to confirm); then `cargo clean -p op-succinct-bindings && cargo build --workspace` |
| `just build-elfs` fails inside docker with `error: rustc 1.93.0-dev is not supported by the following packages: alloy-op-evm@0.32.0 requires rustc 1.94 …` | The SP1 docker image bundles an older rustc than mantle-v2 deps declare as a policy floor | `justfile` already passes `--ignore-rust-version` to `cargo-prove prove build` (§3.7). If you see this anyway, you're calling `cargo-prove` directly — add the flag, or `git pull` to land the §3.7 justfile fix |
| `just build-elfs` exits with `cd: ../celestia: No such file or directory` | Stale justfile recipe referencing `programs/range/celestia` / `eigenda` paths that Phase 2 deleted | `git pull` to land the §3.7 justfile fix (cleanup committed in the same commit as the `--ignore-rust-version` flag) |
| `mise: command not found` after running the installer | mise binary lives at `~/.local/bin/mise` but PATH doesn't include it yet | `eval "$(~/.local/bin/mise activate bash)"` for the current shell; the `echo … >> ~/.bashrc` line above seeds future shells |
| `mise install` finishes instantly with "all tools are installed" but `forge --version` returns the wrong version | Shell didn't pick up mise's PATH shim, so an older `forge` from `~/.foundry/bin/` or system pkg manager is winning | re-source rc files; check `which forge` vs `mise which forge`; `mise exec -- forge --version` to bypass PATH and confirm mise's copy works |
| `git submodule update` hangs or errors out on github.com | network / proxy / firewall on the server can't reach github.com | configure git http proxy or pull through an internal mirror |

### 5.5 Optional: SP1 program builds

The Rust workspace builds without SP1; you only need SP1 if you intend to regenerate
the on-chain ELFs (`programs/range/*`, `programs/aggregation/*`).

```bash
curl -L https://sp1.succinct.xyz | bash
sp1up --version v6.4.0      # matches the SP1 version pinned in Cargo.toml
cargo prove --version
```

ELF rebuilds run inside Docker (`cargo prove build --docker --tag v6.4.0`), so the
host's nightly doesn't need to match the toolchain inside the Docker image —
the host just needs `cargo` and the SP1 CLI on PATH.

## 6. Conflict hot spots and time bombs

### 6.1 High-churn hot spots

| Location | Why it churns | Post-sync checks |
|---|---|---|
| `Cargo.toml` `[patch.crates-io]` | Every upstream dep bump might add/remove a revm-family crate. | Diff against `mantle-v2/rust/Cargo.toml` `[patch.crates-io]` — keep them in lock-step. |
| `Cargo.toml` mantle-v2 pins | 25 entries pin the same tag; bump them together. | `grep -c 'tag = "v1.6.1-rc0"' Cargo.toml` should equal 25, and `grep -c 'tag = "v107-mantle-arsia.1"'` should equal 16 for the revm family. Both move in lockstep. |
| `utils/signer/src/lib.rs::from_env` | New auth backends or env-var conventions arrive in alloy-signer-gcp. | Verify the `HSM_API_NAME` branch still compiles + the precedence ordering still puts Mantle compat first. |
| `bindings/build.rs` `required_contracts` | New contract ABIs land upstream. | Diff vs upstream's list; if a new FP-related ABI appears, drop it (FP is gone). |
| `validity/src/proposer.rs` | The proposer flow is the most-edited file in this repo. | Look for any spot where upstream replaced our checkpoint-validation logic — `historicBlockHashes` cross-check must stay. |
| `contracts/foundry.toml` remappings | Upstream may rename or split source dirs. | Re-run `forge build`; missing-import errors point at the broken remap. |

### 6.2 Time bombs

| Risk | Trigger | Mitigation |
|---|---|---|
| **alloy-evm major bump** | Upstream raises alloy-evm to v0.35+ | `alloy-evm` is unpatched (crates.io), but `alloy-op-evm` comes from mantle-v2 and the two must agree — a mismatch surfaces as duplicate `alloy_evm` types. Wait for mantle-v2 to move, then bump its tag here (kona + revm tags in lockstep). |
| **op-revm v19 → v20+ drift** | mantle-elysium does not track upstream op-revm. New OpSpecId variants surface. | `cargo build` will flag non-exhaustive matches. The KARST treatment in mantle-v2/rust kona genesis sync is the reference pattern (comment out the unsupported arm with `[MANTLE]` rationale). |
| **mantle-xyz/op-succinct origin/main divergence** | Someone lands new Mantle features directly on `origin/main` instead of this v3.8.1 branch. | Treat this branch as the source of truth going forward; pull-and-port new origin/main commits the way Phase 5 did. Add an entry to §3 for each port. |
| **Contracts protocol change** | Mantle network upgrade lands new on-chain contracts. | New v117-style port from the canonical Mantle contracts release into `contracts/`. The contracts side is decoupled from the Rust workspace; bump independently. |
| **GCP HSM auth model shift** | Mantle adopts Workload Identity Federation; ops stops setting `HSM_CREDENTIALS`. | The upstream 4-env branch and metadata-service fallback are already in place — no code change needed; just stop setting `HSM_API_NAME` and configure the cluster identity instead. |
| **Pre-Arsia historical proofs requested** | Product asks for proofs / cost estimates on L2 < 94355444 (mainnet). | Not solvable in this repo. Pre-Arsia blocks were produced under BVM mode with a non-OP-Stack blob encoding. Would require a parallel BVM-mode derivation pipeline (fork `BlobData::decode` / `FrameQueue` / `ChannelBank` / `BatchQueue` in `kona-derive`) plus a BVM batch-format spec — both upstream of this repo. See §1.1. |

## 7. Maintaining this file

When you add, modify, or remove a Mantle change:

1. Add a `[MANTLE]` comment in the source explaining intent.
2. Register the change under the appropriate subsection of §3.
3. If the change is structural (new env-var convention, new dep redirect, new contract
   carve-out), evaluate whether §6.1 needs a new hot-spot entry.
4. If you *remove* a Mantle workaround after concluding upstream now does it natively,
   log that removal here so the next sync engineer does not reintroduce it from the
   old fork's history.
5. Reference this file in the commit message so future contributors can find their way
   back.

## 8. Related artifacts

| Resource | Purpose |
|---|---|
| `mantle-v2/rust/MANTLE_CHANGES.md` | Sister registry for the mantle-v2 Rust subtree (kona / op-alloy / alloy-op-evm). Read it together with §3.1 of this file when bumping deps. |
| `~/Projects/mantle-rollup-configs/` | Out-of-tree rollup-config JSON store (see §3.6). |
| ~~`mantle-xyz/evm @ mantle-v0.34.0`~~ | **No longer used.** The alloy-evm fork was dropped; alloy-evm resolves from crates.io (see §1 / §2.2). |
| `mantle-xyz/revm @ mantle-elysium` | revm fork pinned via `[patch.crates-io]` (see §2.2). |
| `mantle-xyz/op-succinct @ v1.1.7-2` | v117 contract source (see §2.3 / §3.2). |
