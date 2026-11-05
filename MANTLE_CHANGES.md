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
| Dependency source: kona / op-alloy / alloy-op-evm | `mantle-xyz/mantle-v2` rust subtree @ tag `mantle-v1.6.3` (commit `05e2f58d`). Pinned by **tag**, not rev — 25 entries in `Cargo.toml` use `tag = "mantle-v1.6.3"`; bump them together. Note mantle-v2 switched to the `mantle-v` tag prefix at this release; a plain `sort` puts those tags *before* every `v*` tag, so "latest" is not the tail of a sorted list. |
| Dependency source: revm family | `mantle-xyz/revm` @ tag `v107-mantle-arsia.2` (commit `835cb4eb`) via `[patch.crates-io]`, 16 entries. Resolves to revm 38.0.0 / revm-handler 18.1.0 / **op-revm 19.0.0** (upstream op-succinct is on op-revm 20.0.0). Moves in **lockstep** with the kona tag above. |
| Dependency source: alloy-evm | **upstream `alloy-rs/evm` v0.34.0 from crates.io — NOT patched.** The former `mantle-xyz/evm @ mantle-v0.34.0` fork only added a dead-code `token_ratio` trait method; mantle-v2/rust dropped it at `d2e4ebea` (commit `75d90fc71`), so the `[patch.crates-io]` redirect was removed here to stay in lockstep. |
| Dependency source: alloy core/network | crates.io `2.0.4` — deliberately **behind** upstream v3.12.0's `2.0.5`. See §3.11. |
| Dependency source: alloy-primitives | crates.io `1.5.x` (resolves 1.5.7) — deliberately behind upstream's 1.6.0, which belongs to the kona version lockstep. This is what fixes the `sha3` patch tag; see §3.11. |
| SP1 | `=6.10.0` + sp1-cluster tag `v2.9.0` — bumped ahead of the kona v1.8.0 line, see §3.11a |
| Contracts baseline | `mantle-xyz/op-succinct` tag `v1.1.7-2` (a.k.a. "v117"); ported into `contracts/` |

### 1.0 Release versioning

Releases are numbered on **Mantle's own version line**, starting at `mantle-v1.6.0`. This replaces
the older `v<upstream>-<network>-mantle-<hardfork>.<n>` scheme (e.g.
`v3.8.1-mainnet-mantle-arsia.2`), which led with the upstream version.

**Releases are cut from `main`, and both networks run the same release.** Testnet and mainnet are
not separate builds: the L1 registry is chain-aware and dispatches on `chain_id` at runtime, so
the two differ in configuration, not in the artifact. A release is deployed to testnet first and
then to mainnet without re-tagging or rebuilding.

Tags outside that flow (release candidates and the like) are fine; what must hold is that the
release both networks run is one tag, cut from `main`.

**Where the upstream version lives.** Because the tag no longer carries it, three places do, with
deliberately different jobs:

| Place | Holds | Why there |
|---|---|---|
| `Cargo.toml` `[workspace.package] version` | The **upstream** version (`3.12.0`) | It arrives with the upstream merge and updates itself. **Do not** change it to the Mantle version: that would add a conflict to every sync and remove the one field that tracks upstream for free. |
| The table below | Mantle version ↔ upstream baseline | The authoritative mapping; survives tag renames and is greppable. |
| Tag annotation | Full context of one release | Makes a single tag self-explanatory without cross-referencing. |

#### Version map

| Mantle version | Upstream baseline | mantle-v2 (kona) | revm | SP1 | Notes |
|---|---|---|---|---|---|
| `mantle-v1.6.0` | `v3.12.0` @ `94ce6393` | `v1.6.2` | `v107-mantle-arsia.1` | `6.4.0` | First release on the Mantle version line. Sync v3.8.1 → v3.12.0, proposer resilience fixes, L1 gas policy. Both vkeys changed — see §3.11.1. |
| `v3.8.1-{testnet,mainnet}-mantle-arsia.2` | `v3.8.1` @ `1e8e32e0` | `v1.6.0` | `revm-ghsa…` @ `99707e9f` | `6.1.0` | Old naming scheme. Proposer hardening (PR #46). |
| `v3.8.1-{testnet,mainnet}-mantle-arsia.1` | `v3.8.1` @ `1e8e32e0` | `v1.6.0` | `revm-ghsa…` @ `99707e9f` | `6.1.0` | Old naming scheme. v3.8.1 baseline (PR #43). Does **not** contain the #923 checkpoint fix. |

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
pinned `tag = "mantle-v1.6.3"` (25 entries; commit `05e2f58d`). The mantle-v2/rust
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
| `Cargo.toml` | All `kona-*`, `op-alloy*`, `alloy-op-evm*` deps switched from crates.io / the official kona repo to `mantle-xyz/mantle-v2` git at the pinned tag (currently `mantle-v1.6.3`; see §1). |
| `Cargo.toml` `[patch.crates-io]` | All 13 revm-family crates redirected to `mantle-xyz/revm` at the pinned tag (currently `v107-mantle-arsia.2`; see §1). |
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

**Env var precedence in `Signer::from_env()`** — *superseded by §3.5a, kept for history:*
1. `HSM_API_NAME` set → Mantle compat branch (memory-only creds)
2. `GOOGLE_PROJECT_ID` + `GOOGLE_LOCATION` + `GOOGLE_KEYRING` set → upstream branch (ADC)
3. `SIGNER_URL` + `SIGNER_ADDRESS` set → Web3Signer
4. `PRIVATE_KEY` set → local plaintext signer
5. None set → error

### 3.5a AWS KMS L1 signer, and exactly-one signer selection

Added for the move of the proposer's L1 key from GCP KMS to AWS KMS (after the 2026-10-06 HSM
compromise). Host-only — no guest code, **no vkey change**.

| File | Change |
|---|---|
| `utils/signer/src/lib.rs` | `Signer::AwsKmsSigner(alloy_signer_aws::AwsSigner)`, built from `L1_SIGNER_AWS_KMS_KEY_ID`. Two regions are kept apart: credentials (IRSA → STS) use the *home* region from the default chain (`AWS_REGION` — which EKS injects into every IRSA pod — `AWS_DEFAULT_REGION`, profile; the key's region only as a fallback when none is found), while **only the KMS client** uses the key ARN's region, with a warning if it differs from home. A cross-region (e.g. `mrk-`) key is legitimate, and pointing STS at the key's region would break pods whose egress only covers their own region. |
| | `from_env` now goes through `select_signer_backend`: exactly one backend. |
| | KMS signers (GCP and AWS) sign a fixed probe digest at startup and check it recovers to their address; backend + address logged as `L1 transaction signer ready`. |
| | Local / GCP / AWS dispatch share `send_with_wallet` (the per-signer bodies were identical). It now **signs, records the tx hash, then broadcasts** (`fill` → `send_tx_envelope`) instead of `send_transaction`, which only yields the hash after the broadcast returns; the Web3Signer path records its hash before broadcasting too. Required by the attempt timeout below: a timeout firing mid-broadcast must not lose the hash of a transaction the node already accepted, or the escalation loop would report a mined transaction as failed and the proposer could resend a checkpoint. |
| | Timeouts: AWS KMS calls 10s per attempt / 30s per operation (the SDK sets none); each L1 send attempt is bounded by the confirmation timeout + 120s for fill/sign/broadcast (all signer kinds); every L1 read made under the lock — the pre-send nonce/fee reads, the post-failure receipt lookup, the final nonce read — is bounded by 30s (the default HTTP client has no request timeout); the startup self-check by 60s. A hung KMS or RPC call would otherwise hold the `SignerLock` and stall every later send while probes stay green. `GasPolicy::worst_case_send_secs` is computed from the same bounds, so the proposer's chain-lock lease check (`validity/src/proposer.rs`) sees the real worst case — with defaults, 4 × (60 + 120 + 30 + 60) + 2 × 30 = 1140s per send instead of 240s. |
| | **Post-failure success now requires the same `NUM_CONFIRMATIONS` (3) as the normal path** (`confirmed_receipt_among`). Pre-existing on `main`: after an attempt failed, the first receipt found among our hashes was returned as success immediately, so a transaction one block deep — still reorg-able — was reported to the proposer as a completed checkpoint/submission. It now waits (≤ the confirmation timeout) for 3 confirmations, re-reading the receipt each round; a receipt that disappears (reorg) resumes escalation, and a mined transaction that does not reach depth in time is an error, not a success. |
| `Cargo.toml` | `alloy-signer-aws =2.0.4` (moves with the alloy family); `aws-config` / `aws-sdk-kms` `^1` resolve to the copies sp1-sdk already pulls. `Cargo.lock` adds only `alloy-signer-aws 2.0.4`; sp1-sdk's own `alloy-signer-aws 1.8.3` (alloy 1.x) coexists. |

**Selection rules (replace the §3.5 precedence list):**
- AWS KMS, GCP KMS (either shape) and Web3Signer are *remote* backends. One is configured as soon
  as one of its **signer-specific** selecting variables is **present — even if empty** —
  `L1_SIGNER_AWS_KMS_KEY_ID`, `HSM_API_NAME`, `GOOGLE_KEYRING`, `SIGNER_URL` — and must then have
  all its required variables set to non-empty values, otherwise startup fails. Previously a
  half-configured remote signer fell through, ultimately to `PRIVATE_KEY`, which mainnet's secret
  also carries; that silent fallback is what this closes. "Present but empty" counts as configured
  for the same reason: a template whose value failed to render (`HSM_API_NAME: ""`) must fail
  loudly rather than start on the plaintext key.
  Generic names other components read (`GOOGLE_PROJECT_ID`, `GOOGLE_LOCATION`, `HSM_KEY_NAME`, a
  leftover `SIGNER_ADDRESS`) deliberately do not select a backend, or an unrelated setting would
  become a "partially configured signer" startup failure.
- Two configured remote backends → startup error (no precedence order).
- `PRIVATE_KEY` only when no remote backend is configured; alongside one it is ignored with a
  warning.
- Values are trimmed before use (a secret mounted with a trailing newline is harmless).

**Why `L1_SIGNER_AWS_KMS_KEY_ID`.** Two obvious names are taken:
- `CRYPTO_KEY_ID` — other Mantle services name a KMS key this, but there it is the key
  `/usr/lib/crypto` uses to *decrypt* the secrets listed in `CRYPTO_SECRET_NAMES` at startup —
  mantle-succinct's mainnet config sets it today, next to `HSM_API_NAME`. Reading it as a signing
  key would make mainnet fail at startup under the exactly-one rule (and a symmetric decrypt key
  cannot sign anyway). Pinned by `mantle_crypto_key_id_is_not_a_signer`.
- `AWS_KMS_KEY_ID` — Foundry's `--aws` wallet variable, which this repo's deploy scripts
  (`contracts/lib/func.sh`) use for the **deployer** key. An operator who deployed and then ran
  `clear-stuck-txs` from the same shell would sign as the deployer, with only a warning that
  `PRIVATE_KEY` was ignored. Pinned by `foundry_and_stray_gcp_variables_do_not_select_a_backend`.
  (An earlier revision of this branch used this name; a second review caught it.)

**QA follow-ups (2026-10-28).** Seven findings from QA (1–5 on the first revision, 6–7 a second
round), reproduced and fixed:

| # | Finding | Fix |
|---|---|---|
| 1 | `AWS_ENDPOINT_URL` / `AWS_ENDPOINT_URL_STS` also reach the STS client that exchanges the IRSA web-identity token, so the token went to the override address. `AWS_ENDPOINT_URL_KMS` alone does not. | Both are refused at startup (`check_aws_endpoint_overrides`); the SDK has no public way to keep them from the credential provider. `AWS_ENDPOINT_URL_KMS` stays supported — mantle-config#16830 (hoodi-qa3) relies on it for a KMS VPC endpoint with private DNS off — and is logged. KMS requests are SigV4-signed: an endpoint sees the access key id and session token, not the secret key. An STS VPC endpoint must use private DNS. **Not covered:** an `endpoint_url` set in a shared config file (`AWS_CONFIG_FILE` / `~/.aws/config`) reaches STS the same way; the image ships no such file, so keep it that way. **Log caveat:** with `AWS_IGNORE_CONFIGURED_ENDPOINT_URLS=true` the SDK ignores `AWS_ENDPOINT_URL_KMS` although it is still logged — do not set it. |
| 2 | Static keys in the environment silently take precedence over IRSA; nothing in the logs showed it. | The KMS client's credentials go through `CredentialSourceRecorder`, which reads the source tags the SDK itself puts on every set of credentials (`AwsCredentialFeature`) — the identity KMS calls actually use, with no inference from the environment and no extra credential resolution. Logged at startup (`AWS credentials source`, access key id masked), also when loading the key fails (an AccessDenied is easier to read next to the identity that got it); a WARN when IRSA / Pod Identity is configured but the credentials came from another source (static keys, a shared config profile, …); a WARN if a later refresh comes from a different source. Not refused: static keys and profiles are legitimate for local runs. Earlier revisions inferred the source from environment variables and files; each review found another SDK rule they missed (`SECRET_ACCESS_KEY`, `~` expansion, profile fall-through), which is why it is observed now. |
| 3 | `AWS_REGION=""` with a bare key id gave an opaque SDK endpoint error. Underneath: the SDK's environment region provider returns `Some("")` for a blank `AWS_REGION` and ends the region chain there, so `AWS_DEFAULT_REGION`, the profile and IMDS were never consulted. | Blank region variables are unset for the whole chain, as the SDK treats blank credential variables: the environment step is done here (`AWS_REGION`, then `AWS_DEFAULT_REGION`, blanks skipped), the rest of the default chain (profile file, then IMDS) is left to the SDK, and a WARN names the blank variable. A bare key id with no region anywhere gets "AWS region is unknown"; an ARN still sets the KMS region, while STS keeps the pod's own region. |
| 4 | GCP signer construction had no timeout: with the token endpoint or KMS unreachable, startup hung silently (the signer is built before the metrics server, so only the startup probe would notice). | Building any KMS signer is bounded by `REMOTE_SIGNER_INIT_TIMEOUT` (60s) with an error naming the backend. |
| 5 | A send whose attempts all failed *before broadcast* (signing error, gas-estimation revert) was escalated at higher fees and finally reported as "still pending" although nothing had been broadcast. Pre-existing on `main`, for every signer kind. The same held when the node *refused* the broadcast (insufficient funds, nonce too low): the hash is recorded before `eth_sendRawTransaction`, so a refused transaction counted as sent. | A broadcast the node refused for **insufficient funds** forgets its hash (`forget_if_rejected`, an allowlist: `DEFINITIVE_BROADCAST_REJECTIONS`) — not in any pool, and a higher fee cannot help. Everything else keeps it — transport errors and timeouts (outcome unknown), "already known" (the transaction is in the pool), an RPC gateway's own JSON-RPC error after the transaction may already have reached a node, "nonce too low" (a gateway retrying a broadcast that has meanwhile been mined gets exactly that for our own transaction), and "underpriced" (a fee bump *is* the fix; `clear-stuck-txs` hits it when its fee does not clear a stuck transaction's, and must keep escalating as on `main`). Wrongly keeping a hash only makes the final message less precise; wrongly forgetting one could report a mined transaction as failed and make the proposer send it again. If no attempt has a transaction that may be in a mempool, the send stops at once: `L1 transaction not sent: … nothing was broadcast`, cause kept in the chain (the proposer's revert-data classification is unchanged and now sees an estimate revert immediately instead of after three fee bumps (four attempts)). Once something has been broadcast, a replacement that fails before broadcast keeps escalating as before: "still pending" is then true, and stopping would let the next send queue at a new nonce. |
| 6 | GCP credentials (`HSM_CREDENTIALS`) that are not valid JSON were not checked before connecting to Google: with Google unreachable only a transport error surfaced, nothing pointed at the credentials. gcloud-sdk opens its channel *before* parsing them (`with_token_source`: `init_google_services_channel`, then `GoogleAuthTokenGenerator::new`). | `check_gcp_credentials` builds the same `GoogleAuthTokenGenerator` first — the SDK's own parsing, no network (tokens are fetched on first use) — for `HSM_CREDENTIALS` and for the key file named by `GOOGLE_APPLICATION_CREDENTIALS` on the keyring path (read raw, as the SDK does). A hand-written structural check was tried first and dropped in review: the SDK matches credentials by structure (`#[serde(untagged)]`), ignores `type`, and needs `private_key_id` / `token_uri` — a copy of its rules drifts. Errors name the variable and never echo the contents. Not covered: a private key that is a string but not a valid PEM — the SDK reads it only when it first signs a token request. |
| 7 | After this release has run once, rolling back to `mantle-v1.6.1` failed: the older binary exits at startup with "migration 7 was previously applied but is missing", so the documented rollback ("stop the new version, start the old one") did not work. | `validity/migrations-rollback/07_chain_lock_owner.sql` — run once, proposers stopped, before starting the older release; see §3.10d "Rolling back". Tested end to end against the real migrators. The earlier claim here that the migration "may be deployed first … an old binary ignores both columns" was wrong and is removed. |

**Deployment checklist (mantle-config, not this repo):**
- The mantle-succinct IRSA role (`cicd/services/mantle-succinct/aws-<env>.yaml`) has only
  Secrets Manager today. Add `kms:GetPublicKey` + `kms:Sign` on the key ARN; the IRSA role has no
  implicit KMS permission. Missing `kms:Sign` is caught by the startup self-check, which is the
  point of it: `GetPublicKey` alone lets the signer build.
- Set `L1_SIGNER_AWS_KMS_KEY_ID` to the key **ARN** (then the region comes from it; `AWS_REGION` is only
  needed for a bare key id or alias); remove `HSM_API_NAME` / `HSM_CREDENTIALS` and
  `PRIVATE_KEY`. `CRYPTO_*` can stay if other secrets still need decrypting.
- On chain: `addProposer(<new address>)` from the owner Safe, fund it, switch, then
  `removeProposer(<old address>)`.
- The proposer's lease warning now accounts for the send and read timeouts (1140s per send with
  defaults). If `LOOP_INTERVAL` was tuned tight against the old 240s figure, the startup warning may
  start firing; that is the check becoming accurate, not a regression.
- New L2OO deployments only: `fetch_l2oo_config` derives the default proposer address from
  `PRIVATE_KEY`. With a KMS signer, set `PROPOSER=<KMS address>` explicitly.

**Verifying against a real key locally.** `aws_kms_live_signer_builds_and_self_checks` is
`#[ignore]`d. The Rust SDK does **not** read the AWS CLI's MFA cache, so with an MFA-gated
`assume_role` profile it re-runs AssumeRole without MFA and is denied. Export the CLI's cached
credentials into the environment first:

```bash
eval "$(AWS_PROFILE=<profile> aws configure export-credentials --format env)"
unset AWS_PROFILE
L1_SIGNER_AWS_KMS_KEY_ID=<key arn> cargo test -p op-succinct-signer-utils aws_kms_live -- --ignored --nocapture
```

Verified 2026-10-15 against `ECC_SECG_P256K1` key `mrk-3227…5bc8` (ap-southeast-1, region taken
from the ARN): address `0x59415743C6035a338D2661A9b5B325225237D91B`, self-check passed. A role
with `kms:GetPublicKey` but not `kms:Sign` failed at the self-check, as intended.

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
| Unsatisfiable precondition | `is_unsatisfiable_precondition_error` (gRPC `FAILED_PRECONDITION`) | **no program registered for our vk_hash** — something admitting requests by vk_hash in front of the prover refuses the current one, the predictable failure right after a vkey change. Not a missing cluster-side registration: the ELF is uploaded to the artifact store with every request (`ClusterElf::NewElf`) |

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

### 3.10b Unpollable proof requests must not stall the proposer (mainnet incident)

**Symptom.** After an L1 reorg made a PLONK proof panic on mainnet, the proposer held a
`proof_request_id` the network had no record of. Every loop it logged

```
ERROR proposer.run: Error in proposer loop: Timeout after 15s waiting for proof request
details for request 19902 (start_block=99911546, end_block=99913346)
```

and made no other progress. The range was never re-requested; recovery meant deleting the row
by hand.

**Two independent faults.**

1. *The failure aborted the whole iteration.* `handle_proving_requests` contained the
   asymmetry below — the cluster branch contained errors per request, the network branch did
   not:

   ```rust
   if self.proof_requester.cluster {
       if let Err(e) = self.process_cluster_proof_status(request).await { warn!(...) }
   } else {
       self.process_proof_request_status(request).await?;   // <- aborts the iteration
   }
   ```

   Because this is step 5 of the loop, everything after it was skipped: the canonicality gate,
   `add_new_ranges`, proof submission, aggregation, and `update_chain_lock`. `run` then took its
   10s error path and repeated. Same shape as the `log_proposer_metrics` fault in §3.10a — a
   step that propagates a **non-self-healing** error stops the proposer permanently.

2. *Nothing ever gave up on the request.* The network path had no equivalent of the cluster
   path's `consecutive_poll_failures`, so an unanswerable request was polled forever.

**Fix.** The network branch now contains errors per request and tracks consecutive failures in
`Proposer::network_poll_failures`, reusing the cluster path's `MAX_CONSECUTIVE_POLL_FAILURES`
threshold. On reaching it the row is reset to `Unrequested`, so the range is proven again from
scratch — `update_request_to_prove` overwrites `proof_request_id` on resubmission, so the dead id
does not linger. `poll_failure_action()` isolates the escalation as a pure function, tested for
both directions (transient failures must not throw away an in-flight proof; persistent ones must
escalate, monotonically — a non-monotone rule would restore the infinite loop).

**Why `Unrequested` and not `Failed`.** The split gate counts `Failed` rows *for the same block
range* (`fetch_failed_request_count_by_block_range`), so failing the row here would push an
otherwise healthy range towards bisection after a few network outages — exactly what the
no-bisect policy in §3.10a exists to prevent. Nothing is wrong with the range; only the proof
request became unreachable. For the same reason this does not go through
`handle_failed_request`.

**Only network failures count.** `process_proof_request_status` also fails on our own database
(`is_request_invalidated`, `update_prove_duration`, ...) and on local invariants ("Core proofs are
not supported"). Those must not spend a request's abandon budget: discarding an in-flight proof
would not fix a local problem, and a *permanent* local error would loop forever re-requesting a
range that fails identically every time. `network_call_with_timeout` therefore tags its failures
with `NetworkPollError`, and only those are counted; everything else is logged and retried next
loop.

**The reset is guarded on the row still being in `Prove`.** A poll can store a finished proof and
move the row to `Complete`, then fail on a *later* step of the same poll — fetching execution
statistics used to be such a step. Abandoning on that failure with an unconditional write would
discard a proof that exists and was paid for. `reset_prove_request_to_unrequested` therefore
carries `AND status = Prove` and reports whether anything changed; two DB tests pin both
directions. Fetching execution statistics is also no longer fatal — it is observability, gathered
after the proof is already stored.

**Why this is bounded rather than merely "all cases handled".** `process_proof_request_status`
has 16 error exits spanning three origins (prover network, our database, local invariants) and
they fire at different points in the row's lifecycle — some after the row has already moved to
`Complete`. Enumerating them and getting each judgement right is not a property anyone can keep
true as the function changes. So abandoning is gated on three conditions, of which the last is
structural:

1. the error carries `NetworkPollError` (only `network_call_with_timeout` produces it),
2. it has recurred `MAX_CONSECUTIVE_POLL_FAILURES` times, and
3. **the row is still in `Prove` at the moment of the write** (`AND status = Prove`).

Condition 3 means a misjudgement in 1 or 2 cannot corrupt anything: the worst case becomes
"a healthy in-flight proof is thrown away and re-proven", never "a finished proof is discarded"
or "the proposer stops". If you add an error exit to that function — especially one after the
proof is stored — condition 3 is what keeps it safe, so do not replace the guarded update with
an unconditional one.

Under a total network outage the loop still behaves: every request retries on its own cadence,
the proposer keeps running, other requests keep being processed, and the chain lock keeps being
renewed. Before this fix, a single unpollable request stopped all of it.

**Threshold tradeoff.** The cost of being wrong is asymmetric: abandoning too eagerly discards an
in-flight proof and pays to redo it, while abandoning too late stops the proposer indefinitely.
With the defaults (`LOOP_INTERVAL` 60s, `NETWORK_CALLS_TIMEOUT` 15s) the proposer tolerates about
three minutes of continuous polling failure before giving up on a request.

**Not the same bug as #923.** #923 removed the *cause* of that particular panic (checkpoint
anchored to `latest`) and is present in this branch, in `origin/main`, and in the
`v3.8.1-mainnet-mantle-arsia.2` tag. The stall above is what the proposer does *after* any proof
becomes unpollable, whatever the cause — so it is worth carrying independently of the #923
backport status of whatever release line is deployed.

### 3.10c L1 gas policy and stuck-transaction escalation

**Incident.** 20 `checkpointBlockHash` transactions sat in the mempool and the contract head
stopped advancing. The head of the queue carried `maxFeePerGas` 0.3228 gwei against a base fee
that had risen to 1.25 gwei, so it could never be mined, and nonce ordering held the other 19
behind it — 17 of which were priced perfectly adequately. Full analysis lives outside the repo;
the essentials:

- `utils/signer/src/lib.rs` had **no gas configuration**, delegating entirely to alloy's
  automatic estimate of roughly `base_fee * 2 + priority`.
- base fee rises up to 12.5% per block, so a 2x buffer covers only ~6 blocks. **The thinner the
  absolute gas price, the faster that buffer is overtaken** — this failed at *low* gas
  (0.15 → 1.25 gwei), which is the opposite of the intuition that congestion causes stuck
  transactions.
- Once sent, nothing revisited a transaction: on timeout the signer returned `Err` and left it in
  the mempool forever, blocking every later nonce.

**Fix (all inside `send_transaction_request_with_timeout`, so all three signer kinds and every
call site benefit; the proposer needed no change):**

| Element | Behaviour | Default | Env override |
|---|---|---|---|
| Fee floor | `maxFeePerGas` never below this | 3 gwei | `L1_GAS_FEE_FLOOR_GWEI` |
| Base-fee buffer | `base_fee * N + priority` | 4 (was alloy's 2) | `L1_GAS_BASE_FEE_MULTIPLIER` |
| Escalation | re-send at the **same nonce**, +25% on both fee fields | up to 3 times | `L1_TX_MAX_BUMPS` |

The floor is what makes the incident impossible: a checkpoint is ~46k gas, so 3 gwei costs about
0.00014 ETH — negligible against a stalled proposer. The buffer test asserts 4x survives ≥12
consecutive blocks of maximum base-fee climb.

**Three details that are load-bearing:**

1. **The nonce is pinned before the first send.** Escalation must *replace* the transaction, not
   queue a new one behind it. `SignerLock` already serialises sends, so nothing else claims the
   nonce meanwhile.
2. **Escalation can lose the race.** The transaction being replaced may be mined just as the
   replacement goes out, and the node then rejects the replacement. That is success, not failure.
   The invariant is: **every `Err` return is preceded by a receipt lookup over *all* hashes
   broadcast so far, including the current attempt's.** Each hash is recorded before waiting for
   confirmation, and the record happens before the checks — an earlier version pushed it after,
   which left the final attempt's hash unexamined, so a transaction confirming just past its
   timeout was reported as failed and the caller would send a duplicate checkpoint. When adding a
   return path here, keep it behind that lookup.
3. **Caller-supplied fees are respected.** `has_caller_fees` requires *both* fee fields to be set
   before treating the pricing as the caller's — half-specified would pair a caller value with a
   computed one. `clear-stuck-txs` depends on this, since it picks fees deliberately to replace
   specific stuck transactions.

Verified against anvil with block production withheld: the first attempt times out and the signer
re-sends at the same nonce, `maxFeePerGas 15 -> 18 gwei`, which then confirms.

**Interaction with the chain lock — worth knowing before tuning.** The lock is a lease of
`LOOP_INTERVAL`, refreshed at the end of each iteration and checked only at startup. Escalation
makes an iteration much slower: each send waits up to
`(1 + L1_TX_MAX_BUMPS) * TX_CONFIRMATION_TIMEOUT`, and an iteration can send twice (aggregation
submission and a checkpoint). With the defaults the slowest iteration is ~540s against a 60s
lease, so a restart during an escalation can read the lease as expired and start a second
proposer, which would then contend for nonces.

The imbalance predates escalation (two 60s sends already exceeded a 60s lease) but escalation
widens it by the bump factor. `Proposer::new` now warns when the gap is wide, naming the numbers
and the knobs; it does not refuse to start, since the right setting depends on how the deployment
supervises restarts. Raising `LOOP_INTERVAL` is usually the correct answer.

**Residual risk.** If all attempts time out, the final transaction stays in the mempool and the
next iteration queues behind it — escalation makes that far less likely (the price ends ~1.95x
higher) but does not make it impossible. `clear-stuck-txs` remains the backstop.

**Not addressed:** `checkpointBlockHash` is inherently time-limited — it reads `blockhash()`,
which covers only 256 blocks, so a checkpoint transaction stuck beyond ~51 minutes is guaranteed
to revert. Escalation makes long stalls unlikely rather than impossible; recognising and
re-issuing an expired checkpoint is still unimplemented.

### 3.10d Chain lock: owner-scoped lease, heartbeat, and progress watchdog

**The defect (pre-existing on `main`).** The chain lock — the only thing stopping two proposers
from sending from one account and fighting over nonces — did not hold while a proposer worked:

- Its lease was `LOOP_INTERVAL`, renewed only at the *end* of an iteration, after which `run`
  sleeps for exactly `LOOP_INTERVAL`. The lease therefore lapsed at the start of **every**
  iteration and stayed lapsed until that iteration finished; any iteration that returned early
  skipped the renewal altogether. Raising `LOOP_INTERVAL` cannot fix a sleep equal to the lease.
- Acquisition was check-then-insert — two statements — so two proposers could both pass.
- Renewal was an unconditional `UPDATE … WHERE chain id`, so either process could keep the other's
  lease alive; nothing recorded who held it.

The send path in §3.5a (bounded attempts, confirmation waits) lengthens iterations and so widened
the window, which is how this was found.

**What replaces it** (`validity/src/chain_lock.rs`, `validity/migrations/07_chain_lock_owner.sql`):

| Piece | Behaviour |
|---|---|
| Schema | `chain_locks` gains `owner_id TEXT` and `lease_expires_at TIMESTAMPTZ` (additive, nullable). All expiry is judged by the database clock. |
| Acquire | One `INSERT … ON CONFLICT DO UPDATE … WHERE … RETURNING` — takes the row only if it is new, its lease has lapsed, or (legacy row, `owner_id IS NULL`) its `locked_at` is older than one lease. Atomic: of two racing proposers exactly one wins (tested over 20 rounds). |
| Startup | `main` acquires **before** constructing the proposer, retrying every 10s. A holder that keeps renewing keeps this process waiting indefinitely; one that has exited is replaced as soon as its lease lapses. (Previously a held lock made startup fail and the supervisor crash-loop.) |
| Lease / renew | 120s lease, renewed every 30s by a background heartbeat independent of the loop, each statement bounded by 10s. Renew is `UPDATE … WHERE owner_id = me AND lease_expires_at > NOW()`: a lapsed lease is **not** revived. |
| Lost lease | "Not the holder" on renew marks the lease lost **immediately**; so does reaching the local deadline without a renewal. Losing it ends the process: `main` selects on it, and returning drops every task. The supervisor restarts the process, which waits to re-acquire. |
| Send guard | `ChainLease::ensure_held()` is passed to the signer as a `BroadcastGuard` (`SignerLock::send_transaction_request_guarded`) and runs **immediately before every broadcast** of a send — the first attempt and each fee-escalation replacement, after filling and (KMS) signing — plus once up front. A refusal stops the send at once with `BroadcastRefused`: no further broadcast, no escalation. Not just a flag kept by the heartbeat: it compares a conservative local deadline — the instant *before* the last successful renew statement + lease − 15s — on **both** the monotonic and the wall clock. A paused process that resumes before the heartbeat runs is refused; the wall clock covers a suspended VM, during which Linux's monotonic clock (what Rust's `Instant` uses) does not advance. |
| Progress watchdog | The heartbeat keeps renewing as long as the process lives, so a wedged loop would hold the lock forever (the 2026-09-20 8-core-pod stall ran 7.5h with probes green). The watchdog fires when no iteration has completed for `PROGRESS_WATCHDOG_SECS` **and no L1 send is in flight**, then marks the lease lost — stopping renewal and exiting for a restart. "Start a send" (`LoopProgress::begin_send`) and "declare the loop stalled" take the **same mutex**, so they cannot interleave: either the send registers first and the watchdog stands down, or the watchdog decides first and the send is refused before anything is signed — and no send starts after that. It never cuts a send short (a broadcast may already have reached the node); sends are bounded on their own (§3.5a). Default threshold: `LOOP_INTERVAL + 2 × worst-case send + 600s` (2940s with defaults); `0` disables it. |
| Metrics | `succinct_chain_lock_held`, `succinct_chain_lock_lease_remaining_secs`, `succinct_chain_lock_renew_failures`, `succinct_loop_last_progress_timestamp`. The old startup warning comparing `LOOP_INTERVAL` with the worst-case send is gone — the lease no longer depends on `LOOP_INTERVAL`. |

**Operational rules — read before deploying or rolling back:**

- **Old and new versions must never run as active proposers at the same time.** An older binary
  neither checks `owner_id` nor maintains `lease_expires_at`; its renewal is still the
  unconditional `UPDATE`. The legacy-row rule (a row with no owner is held for one lease after
  `locked_at`) covers a *normal* startup next to an old holder, not a paused-and-resumed old
  process or a rollback. When switching or rolling back the process: **stop the running
  version, then start the other**, and let it acquire the lock.
- **Rolling back to a release without migration 7 (e.g. `mantle-v1.6.x`) needs one SQL step.**
  The older binary ignores the two new columns, but sqlx refuses to start on a database that
  records a migration the binary does not ship ("migration 7 was previously applied but is
  missing"), so the older release exits at startup (QA finding 7). With every proposer for the
  database stopped, run `validity/migrations-rollback/07_chain_lock_owner.sql`
  (`psql "$DATABASE_URL" -f …`) before starting the older release. It forgets migration 7 and
  resets `owner_id` / `lease_expires_at` to NULL, so the row is a legacy row again; the columns
  stay, and re-upgrading re-applies migration 7 (it is idempotent). Covered end to end by
  `rollback_sql_lets_a_release_without_migration_7_start`. The same holds for any later
  migration: a release that adds one needs its own rollback SQL.
- Alert on `time() - succinct_loop_last_progress_timestamp` well below the watchdog threshold
  (e.g. 15 min): a renewing heartbeat says nothing about the main loop. Also alert on
  `succinct_chain_lock_held == 0` for a running proposer, and on `succinct_chain_lock_renew_failures`
  rising.
- `clear-stuck-txs` sends from the same account and takes no lock. Stop the proposer before
  running it.

**What the lock guarantees — and what it does not.** It guarantees that two proposers never keep
running side by side contending for the account's nonces, and that a superseded holder broadcasts
**at most one more transaction**: the one whose guard check had already passed. It does **not**
give strict mutual exclusion at the instant of broadcast. No client-side check can: there is always
an `.await` between "the check passed" and "the bytes left the process", and a process paused there
resumes straight into the send. Closing that gap needs fencing at the receiver — e.g. an owner
epoch the L2OO contract checks on every `proposeL2Output` / `checkpointBlockHash` — which is a
contract upgrade and out of scope here (decided 2026-10-19; revisit only if broadcast-time
exclusivity becomes a hard requirement).

**Accepted behaviours — deliberate, not defects.** Every way a send can meet a lost lease or a
stall was walked through; these three remain by design. Re-raise them only with a scenario that
breaks the stated reason.

| Behaviour | Why it is accepted |
|---|---|
| The guard passes, and then — before `eth_sendRawTransaction` reaches the node — the lease lapses and a successor takes over: the process is **paused after the check** (scheduling, GC, VM suspend) past the lease, or the request is delayed in transit past the 15s safety margin. The old process then broadcasts that one transaction. | Bounded to that single transaction: on resuming, the next guard check (any fee-escalation replacement) refuses, and the heartbeat finds the local deadline passed or the lease taken, marks it lost and ends the process. The transaction is at nonce N, which the successor either also uses (so at most one of the two is mined) or builds on (N+1 after N) — never a conflicting branch. Its content is a legitimate proposer action: a `checkpointBlockHash` is harmless, and a `proposeL2Output` carries a proof the contract verifies (a duplicate is rejected on chain). Cost: some gas, and the successor may see one "nonce consumed by a transaction other than ours" failure, which the existing retry path handles next iteration. |
| A lease lost while a send is already broadcast and waiting for confirmations ends the process mid-wait. | Equivalent to a crash at that point. Continuing to hold work after losing the lock is exactly what the lock forbids; the broadcast transaction either lands or is replaced, and the next holder reads the chain and the database as after any crash. The watchdog, unlike lease loss, never ends a send (above). |
| `clear-stuck-txs` uses the unguarded send path and takes no lock. | It is a manual recovery tool, run while the proposer is stopped (operational rule above). Making it take the lock would block it in exactly the situation it exists for — a proposer that is wedged but alive. |

**Tests.** Embedded-Postgres tests cover exclusivity, the concurrent race, renew-by-holder-only,
lapse-then-takeover (and the old holder failing to renew), the legacy row at 119s (held) and 121s
(taken), and end to end: after a takeover the very next renewal marks the lease lost and sends are
refused. Unit tests cover the dual-clock deadline, the safety margin, sticky loss, the watchdog
never firing during a send, no send starting after a stall is declared, and a 2000-round
real-thread race in which a starting send and the stall declaration never both succeed. Signer
tests: a guard refusing up front sends nothing (no network), and against anvil a guard that
passes up front but refuses at broadcast time leaves the account nonce unchanged — the
transaction was signed but never broadcast.

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

- **kona family `v1.6.0` → `v1.6.2`** (`mantle-xyz/mantle-v2`, 25 entries).
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

1. **On-chain vkeys must be updated.** Both vkeys change in this sync (SP1 6.1.0 → 6.4.0, plus
   the kona/revm retag — the guest embeds the cargo-git checkout path, so a retag alone moves
   them). Values produced by `just vkeys` from the ELFs committed here:

   | Program | On-chain field | Value |
   |---|---|---|
   | Range | `rangeVkeyCommitment` | `0x2a928ed475bd7d8b7a54fac7666c68eb62d36fa15fafa8006b885b3237a7bd21` |
   | Aggregation | `aggregationVkey` | `0x005ec5d81cbc4a9a70334f16cb0078d55ae20da550819bd0dc9c5ed12913b407` |

   For reference, the range vkey chain is `0x1dc93827…` (what the cluster was rejecting
   requests for) → `0x327b7c74…` (v1.6.1-rc0 build, never deployed) → the values above. Update the v117 oracle accordingly — the setter is
   `updateAggregationVkey`, note the casing. The concrete on-chain update procedure is a known
   gap in this document.

   Re-derive at any time with `just vkeys`; it runs SP1 setup() over the `elf/*` files in the
   working tree, so rebuild and stage the ELFs first or it reports the old artifacts' vkeys.
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
5. **A vkey change needs no cluster-side ELF registration.** The cluster submission path builds
   `ClusterElf::NewElf`, and `setup_artifacts` uploads the whole guest binary to the artifact
   store on every request — the ELF travels with the request, so a new vkey works as soon as the
   new image is deployed. What does need updating is the pair of on-chain vkeys. If requests are
   instead rejected with `FAILED_PRECONDITION: program not registered for vk_hash <...>`, the
   refusal comes from whatever admits requests by vk_hash in front of the prover, not from the
   cluster's artifact handling. That is classified no-bisect (§3.10a), so the proposer retries
   whole ranges and recovers by itself once the vkey is admitted — but produces nothing until
   then.
6. `validity/Cargo.toml`'s `tonic` pin exists so `is_transient_transport_error` can downcast to
   the same `tonic::Status` type the pinned sp1-sdk uses. If SP1 6.4.0 pulls a different tonic,
   re-verify that downcast — a silent mismatch sends transient transport faults back into range
   bisection (§3.9).

### 3.11a SP1 6.4.0 → 6.10.0 on the v3.12.0 baseline (ahead of the kona v1.8.0 line)

The kona v1.8.0 / revm line (PR #52, upstream v3.14.x) cannot ship before mantle-v2 and revm release
in December, but the SP1 version has to move now. This bumps **only** the SP1 family on `main`;
kona, revm, alloy and alloy-evm stay exactly as in §1.

**Why now — prover-network lockstep, not the advisory itself.** The self-hosted prover network
moved to SP1 6.10.0 (sp1-cluster `mantle-v2.9.0-rc.1`, upstream sp1-cluster v2.9.0); op-succinct
has always moved with it (6.1.0 → 6.4.0 was the same pairing). The latest SP1 advisory,
GHSA-p7v2-jjv3-8p8f ("Native Rust Verifier: Mismatched Tensor Lengths", fixed in 6.8.1), affects
native Rust verifiers that accept Core/Compressed proofs from untrusted provers — the prover
network's side. op-succinct's production path is unaffected per the advisory: range proofs are
verified in-circuit by the aggregation program (`verify_sp1_proof`) and the result is a Plonk
proof verified on chain. The only native verify call is the dev script `scripts/prove/bin/agg.rs`.

| What | Change |
|---|---|
| `Cargo.toml` | 9 SP1 crates `=6.4.0` → `=6.10.0`; sp1-cluster `v2.7.2` → `v2.9.0` (3 crates). Lockfile: all 49 `sp1-*`/`slop-*` crates resolve to 6.10.0, no drift. |
| `[patch.crates-io]` | sha2 `0.10.9`, sha3 `0.10.8`, substrate-bn, p256, k256, tiny-keccak → their `sp1-6.10.0` tags. **Not** taken from upstream v3.14.3: `sha2-v11` (sha2 0.11 is only in the host tree, via `aws-sigv4`; the guests use 0.10 because revm here is 38, not 42) and sha3 `0.11.0` (main's alloy-primitives 1.5.x uses sha3 0.10.8 — the 0.11 patch would be unused and keccak acceleration silently lost, see §3.11). Rule: the patch tag follows the *patched crate's* version in our tree; `[[patch.unused]]` must stay empty. |
| `validity/Cargo.toml` + `validity/src/proposer.rs` | **tonic `0.12` → `0.14`.** SP1 6.10.0 moved sp1-sdk (and sp1-prover, sp1-cluster) to tonic 0.14; validity pinned 0.12 so that `downcast_ref::<tonic::Status>()` sees the SDK's type (§3.10a). With the pin left at 0.12 everything compiled and every test passed — they build `Status` from *our* tonic — but every production downcast returned None, and the string fallback missed too because tonic 0.13+ renders `code: '<description>'` instead of `status: <Code>`: UNAVAILABLE and FAILED_PRECONDITION (the latter is exactly what the vkey switch produces) would have bisected ranges. Fixed by moving the pin, teaching the fallback both renderings, and adding `tonic_matches_the_sdk`, which reads `Cargo.lock` and fails whenever validity's tonic differs from sp1-sdk's. Found in review; the comment above the pin had warned about exactly this. Related: tonic 0.13+ no longer renders `Status` metadata in `Display`, so the admission-shed marker (`x-sp1-admission-shed`, sent both as trailer and message prefix by the prove-network gateway/router) is now read from the downcast `Status`'s metadata **and** message, with the rendered-text match kept for errors that are not a downcastable `Status` (e.g. the cluster-direct path, which stringifies its eyre errors). |
| `utils/proof/src/lib.rs` | sp1-cluster v2.9.0's `ClusterServiceClient::new` takes an access token: plumbed through `ClusterProofConfig` exactly as upstream v3.14.3 (#1038). The token is read by sp1-cluster from `CLI_CLUSTER_ACCESS_TOKEN`, else `CLUSTER_ACCESS_TOKEN` (empty = unset); auth is off by default. Only the cluster-direct mode uses it — Mantle proves through the prove-network router. |
| `justfile`, `utils/build`, `validity/Dockerfile`, 4 CI workflows, `book/advanced/verify-binaries.md` | SP1 toolchain / docker tag `6.4.0` → `6.10.0`. |
| `elf/` | Rebuilt with `just build-elfs` (docker `v6.10.0`). **Both vkeys change** — see below. |

**Known gap, cluster-direct mode only.** sp1-cluster v2.9.0 adds bearer-token auth. A cluster that requires it, reached without `CLUSTER_ACCESS_TOKEN`, answers `UNAUTHENTICATED`, which the proposer treats as a range failure and bisects. Not reachable in Mantle's deployment (proofs go through the prove-network router with `SP1_PROVER=network`); set the token if cluster-direct mode is ever used against an authenticated cluster.

**On-chain verifier: no change.** The Plonk circuit — and with it the verifier selector a proof
carries — is set by `SP1_CIRCUIT_VERSION`, which is `v6.1.0` in both 6.4.0 and 6.10.0 (`sp1-prover`
and `sp1-recursion-gnark-ffi` crates). The existing SP1VerifierGateway route keeps verifying (sp1-contracts
ships `v6.0.0` and `v6.1.0` verifiers for the v6 line). Only the vkeys move.

**vkeys (this build):**

| | Before (`mantle-v1.6.x`, SP1 6.4.0) | After (SP1 6.10.0) |
|---|---|---|
| range vkey commitment | `0x1e2b863405480e5509bcae955f9e23e1170419ba26af2f271cd0199f1e228fdb` | `0x477a0bf8430d434109eb0d054931ea240f4c87147079de9b43096cf674047cd1` |
| aggregation vkey | `0x00fa36417110bce994f3054a68baef78ca51dee1a38659c70e108da7eb3d6bbf` | `0x0076cf345898ec11c4fd78f6839f2c4fe2cdffd53e967ea6d6d2ab5de0edafe8` |

**Deployment:**
1. Ship together with the prover network's SP1 6.10.0 upgrade (same window); a 6.4.0 proposer and a
   6.10.0 cluster are not a supported pairing.
2. Before starting the new proposer, the L2OO owner calls `updateRangeVkeyCommitment` and
   `updateAggregationVkey` on every network (mainnet: owner Safe). `rollupConfigHash` is unchanged
   (no kona change). In-flight requests with the old commitments are cancelled by the proposer on
   startup and re-proven.

**Merging with the kona v1.8.0 line later (PR #52, `mantle/kona-v1.8.0`).** That branch was cut
before this change and is on SP1 **6.8.1** / sp1-cluster **v2.8.4**. Whichever way the two meet —
the branch syncing upstream v3.14.3 (which itself moves to SP1 6.10.0 / sp1-cluster v2.9.0), the
branch merging `main`, or `main` finally merging the branch — handle these explicitly:

1. **SP1 version lines** (`Cargo.toml` SP1 block, `justfile` docker tag, `utils/build`,
   `validity/Dockerfile`, the 4 CI workflows, `book/advanced/verify-binaries.md`). The branch's
   6.8.1 and this 6.10.0 conflict line-by-line; resolve to **6.10.0** (and sp1-cluster `v2.9.0`).
   Never go back below the prover network's version. If the branch syncs v3.14.3 first, these
   lines come out identical on both sides and merge cleanly.
2. **`[patch.crates-io]` — do not resolve by picking a side.** The right tags follow the crate
   versions in the *merged* tree, not either branch:
   - sha3: the v1.8.0 tree (alloy-primitives 1.7.x) has only sha3 **0.11.0** → `patch-sha3-0.11.0-sp1-6.0.0`
     (there is **no** `0.11.0-sp1-6.10.0` tag; upstream v3.14.3 uses the 6.0.0 one too). `main`'s
     `patch-sha3-0.10.8-sp1-6.10.0` must go once sha3 0.10.8 has left the tree, or it becomes an
     unused patch.
   - sha2: keep `patch-sha2-0.10.9-sp1-6.10.0`. Add upstream's `sha2-v11` only if sha2 0.11 shows
     up in a **guest** tree (`cargo tree -p range -i sha2@0.11.0`, same for `aggregation`); with
     revm 41 it is host-only (via `aws-sigv4`), as it is here.
   - substrate-bn / p256 / k256 / tiny-keccak: the `sp1-6.10.0` tags, as here.
   - Check: `[[patch.unused]]` absent from `Cargo.lock`; every `sp1-*` / `slop-*` package at
     6.10.0 (the 6.8.1 bump on that branch drifted transitively once — re-pin with
     `cargo update -p <crate>@<wrong> --precise 6.10.0`).
3. **`utils/proof/src/lib.rs` `cluster_access_token`.** Added here *and* by upstream v3.14.3 (#1038)
   in the same shape. Keep exactly one copy — a duplicated field or argument means both sides were
   kept. Prefer upstream's text so later syncs stay clean.
4. **tonic pin (`validity/Cargo.toml`).** The branch still pins tonic `0.12` (its SP1 6.8.1 uses 0.12.3).
   It must become `0.14` together with SP1 6.10.0 — keep this side. `tonic_matches_the_sdk`
   fails if it does not; do not "fix" that test.
5. **`Cargo.lock`.** Do not hand-merge: take one side, then `cargo update -p` the conflicting
   crates (or regenerate) and re-run the checks in 2 and 4.
6. **`elf/` binaries always conflict** (both sides rebuilt them). Do not pick either side's ELFs:
   resolve the sources, then `just build-elfs` on the merged tree (docker `v6.10.0`; refill the
   `sp1-cargo-git` volume first — §3.7) and commit the new ELFs in the merge.
7. **vkeys move again** when the v1.8.0 line ships: the range program changes with kona/revm, the
   aggregation vkey usually too. Read both from `cargo run --release --bin config` on the merged
   tree; on-chain `updateRangeVkeyCommitment` / `updateAggregationVkey` for every network. Watch the
   first proposer start for `Rollup config hash mismatch` — kona's `RollupConfig` serialisation
   may differ, in which case `updateRollupConfigHash` with the logged `expected` value.
8. **This file.** §1's SP1 row conflicts (take the merged truth); this section (§3.11a) and the
   branch's §3.12 / §3.13 do not collide. Add the merge outcome to the merged branch's sync
   section, and a Version-map row when the release is cut.
9. **Merge, never squash** the sync PR (§4.4) — squashing loses the upstream merge-base.
10. **`validity/Dockerfile`.** PR #60 landed first and replaced the runtime stage with distroless
   (no shell, no SP1 toolchain). Merging `main` into this branch conflicted only there — this
   branch had bumped the runtime stage's `sp1up` line — and was resolved by taking `main`'s
   runtime stage: the only `sp1up` left is the builder's, at 6.10.0. The v1.8.0 branch carries
   its own Docker speed-ups (`0aaed1ba` `.dockerignore`, `89472c79` drop the SP1 toolchain from
   both stages, `dbfde143` slim runtime, `453d4119` nightly in the chef layer): when it merges
   `main`, keep `main`'s distroless runtime stage (supersedes `dbfde143`), and take the builder
   side of `89472c79` only after checking nothing in the build still calls `cargo prove`.

**Local test note.** `postgresql_embedded` (bundled) resolves the newest PostgreSQL 18.x at build time;
18.6.1's archive is missing the `UTC` / `Etc/UTC` zone links, so every DB test fails with
`invalid value for parameter "TimeZone": "UTC"`. Unrelated to SP1 (pristine `main` fails the same).
Workaround on a dev machine: `cp -Rn /usr/share/zoneinfo/ ~/.theseus/postgresql/18.6.1/share/timezone/`.

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
sp1up --version v6.10.0     # matches the SP1 version pinned in Cargo.toml
cargo prove --version
```

ELF rebuilds run inside Docker (`cargo prove build --docker --tag v6.10.0`), so the
host's nightly doesn't need to match the toolchain inside the Docker image —
the host just needs `cargo` and the SP1 CLI on PATH.

## 6. Conflict hot spots and time bombs

### 6.1 High-churn hot spots

| Location | Why it churns | Post-sync checks |
|---|---|---|
| `Cargo.toml` `[patch.crates-io]` | Every upstream dep bump might add/remove a revm-family crate. | Diff against `mantle-v2/rust/Cargo.toml` `[patch.crates-io]` — keep them in lock-step. |
| `Cargo.toml` mantle-v2 pins | 25 entries pin the same tag; bump them together. | `grep -c 'tag = "mantle-v1.6.3"' Cargo.toml` should equal 25, and `grep -c 'tag = "v107-mantle-arsia.2"'` should equal 16 for the revm family. Both move in lockstep. |
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
