# Mantle op-succinct Patches

This file is the authoritative registry of every Mantle modification stacked on top of
the upstream Succinct Labs `op-succinct` baseline. It is the primary reference when
synchronizing future upstream changes.

**Whenever Mantle changes are added, modified, or removed, update this file.**

## 1. Current baseline

| Item | Value |
|---|---|
| Upstream tracking point | succinctlabs/op-succinct tag `v3.8.1` @ `1e8e32e0` |
| Mantle branch | `mantle/op-succinct-v3.8.1` (this repo, `origin` = `mantle-xyz/op-succinct`) |
| Older Mantle fork (deprecated) | `origin/main` HEAD `664a1bd4` (≈ v3.4.1 era + 68 ad-hoc commits; superseded by this branch) |
| Rust toolchain | 1.94 (see `rust-toolchain.toml`) |
| Dependency source: kona / op-alloy / alloy-op-evm | `mantlenetworkio/mantle-v2` rust subtree @ `58c0204c5` (post kona-client/v1.5.1 sync + alloy-evm fork wiring) |
| Dependency source: revm family | `mantle-xyz/revm @ mantle-elysium` via `[patch.crates-io]` |
| Dependency source: alloy-evm | `mantle-xyz/evm @ mantle-v0.34.0` via `[patch.crates-io]` |
| Contracts baseline | `mantle-xyz/op-succinct` tag `v1.1.7-2` (a.k.a. "v117"); ported into `contracts/` |

### Migration status

| Phase | Scope | Status |
|---|---|---|
| Phase 2 | bump workspace deps to mantle-v2/rust + alloy 2.x + revm 38 + drop EigenDA/Celestia + adapt source to new APIs | ✅ |
| Phase 3 | port v1.1.7-2 contracts onto v3.8.1 baseline + drop Fault Proof feature wholesale + fix v117 internal inconsistencies | ✅ |
| Phase 4 follow-up | redirect `alloy-evm` to `mantle-xyz/evm @ mantle-v0.34.0` fork (in this repo's `[patch.crates-io]`) | ✅ |
| Phase 5 audit | systematic audit of all 68 `mantle-xyz/op-succinct origin/main` Mantle commits vs v3.8.1 baseline | ✅ |
| Phase 5 ports | SP1 error propagation (1 line) + GCP HSM Mantle env-var compat (60 lines) | ✅ |
| op-node compat layer | `5efd6ead` from old fork — deferred | ⏸️ |
| Upstream sync to v4.x | upstream is at v4.3.1; v3.8.1 → v4.x is its own phase | ⏸️ |

## 2. Architecture decisions

### 2.1 mantle-v2/rust supplies the Mantle protocol layer

Every kona-genesis / kona-protocol / kona-derive / kona-executor / kona-host / op-alloy /
alloy-op-evm dependency in `Cargo.toml` is sourced from `mantlenetworkio/mantle-v2` at a
pinned `rev = "58c0204c5"`. That commit is `mantle-v2/rust/upgrade-develop-20260511`
post the kona-client/v1.5.1 sync and the Phase-4 alloy-evm fork wiring. The mantle-v2/rust
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

# alloy-evm → mantle-xyz/evm @ mantle-v0.34.0
alloy-evm
```

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
`mantlenetworkio/mantle-v2` (commit `aad7b8a8`) rather than `ethereum-optimism/optimism`.
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
Mantle's protocol/business-logic deltas. Only a couple of small ports remained — see §3.

## 3. Mantle changes registry

Every change carries a `[MANTLE]` source comment. Discover all sites with:

```bash
grep -rn "\[MANTLE\]" . --include="*.rs" --include="*.toml" --include="*.sol" \
  | grep -v "target/" | grep -v "contracts/lib/"
```

### 3.1 Cargo workspace dependency wiring (Phase 2 / 4)

| File | Change |
|---|---|
| `Cargo.toml` | All `kona-*`, `op-alloy*`, `alloy-op-evm*` deps switched from crates.io / official kona repo to `mantlenetworkio/mantle-v2` git at the pinned `rev = "58c0204c5"`. |
| `Cargo.toml` `[patch.crates-io]` | All 13 revm-family crates redirected to `mantle-xyz/revm @ mantle-elysium`. |
| `Cargo.toml` `[patch.crates-io]` | `alloy-evm` redirected to `mantle-xyz/evm @ mantle-v0.34.0`. |
| `Cargo.toml` | EigenDA and Celestia DA-backend crates dropped (`utils/eigenda/*`, `programs/range/*/celestia`, `programs/range/*/eigenda`, etc.). Validity-Oracle-only path. |

When bumping the mantle-v2 rev, refresh **every** `rev = "..."` in this file (a
`replace_all` of the old → new SHA is the canonical move). Phase 5 confirmed there are
25 such pins.

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

### 3.7 Toolchain pins — `rust-toolchain.toml` + `mise.toml`

| File | Pin | Why |
|---|---|---|
| `rust-toolchain.toml` | `nightly-2026-02-15` (rustc 1.95-nightly) | Upstream v3.8.1 pinned `nightly-2025-09-15` (rustc 1.92-nightly). After Phase 2 swapped deps to mantle-v2, those crates' `rust-version = "1.94"` declaration started rejecting 1.92-nightly. Bumped to 1.95-nightly which keeps the `rustc-dev` component build scripts need. |
| `mise.toml` | `forge = cast = anvil = "1.2.3"`, `svm-rs = "0.5.19"` | Upstream v3.8.1's `bindings/build.rs` calls `forge bind` to generate `bindings/src/codegen/` (gitignored). Without forge on PATH, build.rs prints a warning and skips generation, then `lib.rs:7 mod codegen;` fails to find the module. Pinning forge via mise mirrors mantle-v2/mise.toml so a fresh machine just needs `mise install` rather than a manual Foundry install. `rust` is intentionally NOT pinned here — `rust-toolchain.toml` already drives it and a mise rust pin would silently override (we hit exactly this with mantle-v2's mise.toml earlier). |

## 4. Sync workflow

When a new upstream Succinct Labs release lands (e.g. v3.9.0, v4.0.0):

### 4.1 Pre-sync dry-run

```bash
git remote update upstream
git checkout -b sync-dryrun-v<X.Y.Z> mantle/op-succinct-v3.8.1
git merge --no-commit --no-ff v<X.Y.Z>
git diff --name-only --diff-filter=U   # list conflicting files
grep -l "\[MANTLE\]" $(git diff --name-only --diff-filter=U)  # files with our markers
git merge --abort
```

### 4.2 Sync run

```bash
git checkout -b mantle/op-succinct-v<X.Y.Z> mantle/op-succinct-v3.8.1
git merge v<X.Y.Z>
# resolve conflicts — `[MANTLE]` comments mark every site we touched
```

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
# Open a PR against origin/mantle/op-succinct-v3.8.1 (or whatever the prior
# Mantle branch is), get review, then update §1 baseline in this file and merge.
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
| `mise: command not found` after running the installer | mise binary lives at `~/.local/bin/mise` but PATH doesn't include it yet | `eval "$(~/.local/bin/mise activate bash)"` for the current shell; the `echo … >> ~/.bashrc` line above seeds future shells |
| `mise install` finishes instantly with "all tools are installed" but `forge --version` returns the wrong version | Shell didn't pick up mise's PATH shim, so an older `forge` from `~/.foundry/bin/` or system pkg manager is winning | re-source rc files; check `which forge` vs `mise which forge`; `mise exec -- forge --version` to bypass PATH and confirm mise's copy works |
| `git submodule update` hangs or errors out on github.com | network / proxy / firewall on the server can't reach github.com | configure git http proxy or pull through an internal mirror |

### 5.5 Optional: SP1 program builds

The Rust workspace builds without SP1; you only need SP1 if you intend to regenerate
the on-chain ELFs (`programs/range/*`, `programs/aggregation/*`).

```bash
curl -L https://sp1.succinct.xyz | bash
sp1up --version v6.1.0      # matches the SP1 version v3.8.1 was tagged against
cargo prove --version
```

ELF rebuilds run inside Docker (`cargo prove build --docker --tag v6.1.0`), so the
host's nightly doesn't need to match the toolchain inside the Docker image —
the host just needs `cargo` and the SP1 CLI on PATH.

## 6. Conflict hot spots and time bombs

### 6.1 High-churn hot spots

| Location | Why it churns | Post-sync checks |
|---|---|---|
| `Cargo.toml` `[patch.crates-io]` | Every upstream dep bump might add/remove a revm-family crate. | Diff against `mantle-v2/rust/Cargo.toml` `[patch.crates-io]` — keep them in lock-step. |
| `Cargo.toml` mantle-v2 rev pins | 25 places pin to the same SHA; bump them together. | `grep -c 'rev = "..."' Cargo.toml` should equal 25 (or whatever the current count). |
| `utils/signer/src/lib.rs::from_env` | New auth backends or env-var conventions arrive in alloy-signer-gcp. | Verify the `HSM_API_NAME` branch still compiles + the precedence ordering still puts Mantle compat first. |
| `bindings/build.rs` `required_contracts` | New contract ABIs land upstream. | Diff vs upstream's list; if a new FP-related ABI appears, drop it (FP is gone). |
| `validity/src/proposer.rs` | The proposer flow is the most-edited file in this repo. | Look for any spot where upstream replaced our checkpoint-validation logic — `historicBlockHashes` cross-check must stay. |
| `contracts/foundry.toml` remappings | Upstream may rename or split source dirs. | Re-run `forge build`; missing-import errors point at the broken remap. |

### 6.2 Time bombs

| Risk | Trigger | Mitigation |
|---|---|---|
| **alloy-evm major bump** | Upstream raises alloy-evm to v0.35+ | Coordinate with `mantle-xyz/evm` to catch up before bumping `mantle-v2`; then resync this repo's `[patch.crates-io]`. |
| **op-revm v19 → v20+ drift** | mantle-elysium does not track upstream op-revm. New OpSpecId variants surface. | `cargo build` will flag non-exhaustive matches. The KARST treatment in mantle-v2/rust kona genesis sync is the reference pattern (comment out the unsupported arm with `[MANTLE]` rationale). |
| **mantle-xyz/op-succinct origin/main divergence** | Someone lands new Mantle features directly on `origin/main` instead of this v3.8.1 branch. | Treat this branch as the source of truth going forward; pull-and-port new origin/main commits the way Phase 5 did. Add an entry to §3 for each port. |
| **Contracts protocol change** | Mantle network upgrade lands new on-chain contracts. | New v117-style port from the canonical Mantle contracts release into `contracts/`. The contracts side is decoupled from the Rust workspace; bump independently. |
| **GCP HSM auth model shift** | Mantle adopts Workload Identity Federation; ops stops setting `HSM_CREDENTIALS`. | The upstream 4-env branch and metadata-service fallback are already in place — no code change needed; just stop setting `HSM_API_NAME` and configure the cluster identity instead. |

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
| `mantle-xyz/evm @ mantle-v0.34.0` | alloy-evm fork pinned via `[patch.crates-io]` (see §2.2). |
| `mantle-xyz/revm @ mantle-elysium` | revm fork pinned via `[patch.crates-io]` (see §2.2). |
| `mantle-xyz/op-succinct @ v1.1.7-2` | v117 contract source (see §2.3 / §3.2). |
