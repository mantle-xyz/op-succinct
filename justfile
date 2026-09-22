default:
  @just --list

# Runs the op-succinct program for a single block.
run-single l2_block_num use-cache="false" prove="false":
  #!/usr/bin/env bash
  CACHE_FLAG=""
  if [ "{{use-cache}}" = "true" ]; then
    CACHE_FLAG="--use-cache"
  fi
  PROVE_FLAG=""
  if [ "{{prove}}" = "true" ]; then
    PROVE_FLAG="--prove"
  fi
  cargo run --bin single --release -- --l2-block {{l2_block_num}} $CACHE_FLAG $PROVE_FLAG

# Runs the op-succinct program for multiple blocks.
run-multi start end use-cache="false" prove="false":
  #!/usr/bin/env bash
  CACHE_FLAG=""
  if [ "{{use-cache}}" = "true" ]; then
    CACHE_FLAG="--use-cache"
  fi
  PROVE_FLAG=""
  if [ "{{prove}}" = "true" ]; then
    PROVE_FLAG="--prove"
  fi

  cargo run --bin multi --release -- --start {{start}} --end {{end}} $CACHE_FLAG $PROVE_FLAG

# Runs the cost estimator for a given block range.
# If no range is provided, runs for the last 5 finalized blocks.
cost-estimator *args='':
  #!/usr/bin/env bash
  if [ -z "{{args}}" ]; then
    cargo run --bin cost-estimator --release
  else
    cargo run --bin cost-estimator --release -- {{args}}
  fi

  # Output the data required for the ZKVM execution.
  echo "$L1_HEAD $L2_OUTPUT_ROOT $L2_CLAIM $L2_BLOCK_NUMBER $L2_CHAIN_ID"

upgrade-l2oo l1_rpc admin_pk etherscan_api_key="":
  #!/usr/bin/env bash
  VERIFY=""
  ETHERSCAN_API_KEY="{{etherscan_api_key}}"
  if [ $ETHERSCAN_API_KEY != "" ]; then
    VERIFY="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY"
  fi

  L1_RPC="{{l1_rpc}}"
  ADMIN_PK="{{admin_pk}}"

  cd contracts && forge script script/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader  --rpc-url $L1_RPC --private-key $ADMIN_PK $VERIFY --broadcast --slow

# Deploy mock verifier
deploy-mock-verifier env_file=".env":
    #!/usr/bin/env bash
    set -a
    source {{env_file}}
    set +a
    
    if [ -z "$L1_RPC" ]; then
        echo "L1_RPC not set in {{env_file}}"
        exit 1
    fi
    
    if [ -z "$PRIVATE_KEY" ]; then
        echo "PRIVATE_KEY not set in {{env_file}}"
        exit 1
    fi

    cd contracts

    VERIFY=""
    if [ -n "${ETHERSCAN_API_KEY:-}" ]; then
      VERIFY="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY"
    fi
    
    forge script script/DeployMockVerifier.s.sol:DeployMockVerifier \
    --rpc-url $L1_RPC \
    --private-key $PRIVATE_KEY \
    --broadcast \
    $VERIFY

# Deploy the OPSuccinct L2 Output Oracle
deploy-oracle env_file=".env" *features='':
    #!/usr/bin/env bash
    set -aeo pipefail
    
    # First fetch rollup config using the env file
    if [ -z "{{features}}" ]; then
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release -- --env-file {{env_file}}
    else
        echo "Fetching rollup config with features: {{features}}"
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release --features {{features}} -- --env-file {{env_file}}
    fi
    
    # Load environment variables
    source {{env_file}}

    # cd into contracts directory
    cd contracts

    VERIFY=""
    if [ -n "${ETHERSCAN_API_KEY:-}" ]; then
      VERIFY="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY"
    fi
    
    ENV_VARS=""
    if [ -n "${ADMIN_PK:-}" ]; then ENV_VARS="$ENV_VARS ADMIN_PK=$ADMIN_PK"; fi
    if [ -n "${DEPLOY_PK:-}" ]; then ENV_VARS="$ENV_VARS DEPLOY_PK=$DEPLOY_PK"; fi

    # Run the forge deployment script
    $ENV_VARS forge script script/OPSuccinctDeployer.s.sol:OPSuccinctDeployer \
        --rpc-url $L1_RPC \
        --private-key $PRIVATE_KEY \
        --broadcast \
        $VERIFY

# Upgrade the OPSuccinct L2 Output Oracle
upgrade-oracle env_file=".env" *features='':
    #!/usr/bin/env bash
    set -euo pipefail
    
    # First fetch rollup config using the env file
    if [ -z "{{features}}" ]; then
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release -- --env-file {{env_file}}
    else
        echo "Fetching rollup config with features: {{features}}"
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release --features {{features}} -- --env-file {{env_file}}
    fi
    
    # Load environment variables
    source {{env_file}}

    # cd into contracts directory
    cd contracts

    # forge install
    forge install
    
    # Run the forge upgrade script
    
    ENV_VARS="L2OO_ADDRESS=$L2OO_ADDRESS"
    if [ -n "${EXECUTE_UPGRADE_CALL:-}" ]; then ENV_VARS="$ENV_VARS EXECUTE_UPGRADE_CALL=$EXECUTE_UPGRADE_CALL"; fi
    if [ -n "${ADMIN_PK:-}" ]; then ENV_VARS="$ENV_VARS ADMIN_PK=$ADMIN_PK"; fi
    if [ -n "${DEPLOY_PK:-}" ]; then ENV_VARS="$ENV_VARS DEPLOY_PK=$DEPLOY_PK"; fi

    
    
    VERIFY_FLAGS=""
    if [ -n "${ETHERSCAN_API_KEY:-}" ]; then
        VERIFY_FLAGS="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY"
    fi

    if [ "${EXECUTE_UPGRADE_CALL:-true}" = "false" ]; then
        env $ENV_VARS forge script script/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader \
            --rpc-url $L1_RPC \
            --private-key $PRIVATE_KEY
    else
        env $ENV_VARS forge script script/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader \
            --rpc-url $L1_RPC \
            --private-key $PRIVATE_KEY \
            $VERIFY_FLAGS \
            --broadcast
    fi

# Add a new OpSuccinctConfig to the L2 Output Oracle
add-config config_name env_file=".env" *features='':
    #!/usr/bin/env bash
    set -euo pipefail
    
    # First fetch rollup config using the env file
    if [ -z "{{features}}" ]; then
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release -- --env-file {{env_file}}
    else
        echo "Fetching rollup config with features: {{features}}"
        RUST_LOG=info cargo run --bin fetch-l2oo-config --release --features {{features}} -- --env-file {{env_file}}
    fi
    
    # Load environment variables
    source {{env_file}}

    # cd into contracts directory
    cd contracts

    # forge install
    forge install
    
    # Run the forge script to add config
    env L2OO_ADDRESS="$L2OO_ADDRESS" \
        ${EXECUTE_UPGRADE_CALL:+EXECUTE_UPGRADE_CALL="$EXECUTE_UPGRADE_CALL"} \
        ${ADMIN_PK:+ADMIN_PK="$ADMIN_PK"} \
        ${DEPLOY_PK:+DEPLOY_PK="$DEPLOY_PK"} \
        forge script script/OPSuccinctParameterUpdater.s.sol:OPSuccinctParameterUpdater \
        --sig "addConfig(string)" "{{config_name}}" \
        --rpc-url $L1_RPC \
        --private-key $PRIVATE_KEY \
        --broadcast

# Remove an OpSuccinctConfig from the L2 Output Oracle  
remove-config config_name env_file=".env":
    #!/usr/bin/env bash
    set -euo pipefail
    
    # Load environment variables
    source {{env_file}}

    # cd into contracts directory
    cd contracts

    # forge install
    forge install
    
    # Run the forge script to remove config
    env L2OO_ADDRESS="$L2OO_ADDRESS" \
        ${EXECUTE_UPGRADE_CALL:+EXECUTE_UPGRADE_CALL="$EXECUTE_UPGRADE_CALL"} \
        ${ADMIN_PK:+ADMIN_PK="$ADMIN_PK"} \
        ${DEPLOY_PK:+DEPLOY_PK="$DEPLOY_PK"} \
        forge script script/OPSuccinctParameterUpdater.s.sol:OPSuccinctParameterUpdater \
        --sig "removeConfig(string)" "{{config_name}}" \
        --rpc-url $L1_RPC \
        --private-key $PRIVATE_KEY \
        --broadcast

# Generate verification key hashes.
#
# [MANTLE] Ethereum DA only — the celestia/eigenda/altda blocks are gone with those crates
# (Validity-Oracle-only fork, see MANTLE_CHANGES.md §3.1). They also could not have worked here:
# `--features celestia` no longer exists, and the original recipe swallowed that error via
# `2>&1` + grep, printing an empty cell instead of failing.
#
# These hashes are what go on chain (`rangeVkeyCommitment` / `aggregationVkey`). `config` runs
# SP1 setup() over the COMMITTED `elf/*` files, so run it after `just build-elfs` and commit the
# ELFs — otherwise it reports the vkeys of the old artifacts.
vkeys:
    #!/usr/bin/env bash
    set -euo pipefail

    echo "Generating verification key hashes from the committed ELFs..."
    echo ""

    OUTPUT=$(RUST_LOG=error cargo run --release --bin config)
    RANGE=$(echo "$OUTPUT" | grep "Range Verification Key Hash" | awk '{print $NF}')
    AGG=$(echo "$OUTPUT" | grep "Aggregation Verification Key Hash" | awk '{print $NF}')

    if [ -z "$RANGE" ] || [ -z "$AGG" ]; then
      echo "ERROR: could not parse vkeys from \`config\` output:" >&2
      echo "$OUTPUT" >&2
      exit 1
    fi

    echo "## Verification Key Hashes"
    echo ""
    echo "| Program | Verification Key Hash |"
    echo "|--------|------------------------|"
    echo "| Range Verification Key (rangeVkeyCommitment) | **$RANGE** |"
    echo "| Aggregation Verification Key (aggregationVkey) | **$AGG** |"

# [MANTLE] Verify every tag-pinned git dependency still resolves to the commit in Cargo.lock.
#
# Cargo.lock pins a 40-char commit SHA; the `tag=` is only a hint for finding it. cargo will
# reuse a commit already present in its git cache WITHOUT any network access, so if a mutable
# tag (anything `-rc`, or a re-cut release) is force-pushed to a new commit, a build keeps
# silently using the OLD code — while Cargo.toml and MANTLE_CHANGES.md claim the tag. That is
# unrecoverable for ELFs: the guest embeds the cargo-git checkout path (URL hash + short
# commit), so the vkey would correspond to code nobody can identify later.
#
# Run this before `build-elfs`, and on any machine where the cargo git cache is warm.
verify-git-pins:
    #!/usr/bin/env bash
    set -uo pipefail
    tmp=$(mktemp)
    grep -oE 'git\+https://[^"?]+\?tag=[^#"]+#[0-9a-f]{40}' Cargo.lock | sort -u > "$tmp"
    count=$(wc -l < "$tmp" | tr -d ' ')

    # A gate that checks nothing is worse than no gate: it reports success. If the pattern stops
    # matching (a Cargo.lock format change, say), fail loudly instead of silently passing.
    if [ "$count" -eq 0 ]; then
      rm -f "$tmp"
      echo "ERROR: found no tag-pinned git dependencies in Cargo.lock." >&2
      echo "Either there genuinely are none, or the pattern in this recipe no longer matches" >&2
      echo "the lockfile format. Check before assuming the former." >&2
      exit 1
    fi

    echo "Verifying $count tag-pinned git dependencies against their remotes..."
    fail=0
    unreachable=0
    while IFS= read -r pin; do
      url="${pin#git+}"; url="${url%%\?tag=*}"
      rest="${pin#*\?tag=}"; tag="${rest%%#*}"; sha="${rest##*#}"

      # Ask for both the peeled and unpeeled ref in one round trip, and keep the exit status:
      # a network or auth failure must not look like "tag moved", or the operator may go and
      # "fix" a lockfile that was correct all along.
      if ! out=$(git ls-remote "$url" "refs/tags/$tag^{}" "refs/tags/$tag" 2>/dev/null); then
        printf '  UNREACHABLE %-32s %s (could not query remote)\n' "$tag" "${url##*/}"
        unreachable=1
        continue
      fi

      # Prefer the peeled ref (annotated tags); fall back to the plain one (lightweight tags).
      remote=$(printf '%s\n' "$out" | awk '$2 ~ /\^\{\}$/ {print $1; exit}')
      if [ -z "$remote" ]; then
        remote=$(printf '%s\n' "$out" | awk 'NR==1{print $1}')
      fi

      if [ -z "$remote" ]; then
        printf '  MISSING   %-34s %s (tag not found on remote)\n' "$tag" "${url##*/}"
        fail=1
      elif [ "$remote" = "$sha" ]; then
        printf '  OK        %-34s %s\n' "$tag" "${url##*/}"
      else
        printf '  MISMATCH  %-34s %s\n            lock:   %s\n            remote: %s\n' \
          "$tag" "${url##*/}" "$sha" "$remote"
        fail=1
      fi
    done < "$tmp"
    rm -f "$tmp"

    if [ "$fail" -ne 0 ]; then
      echo
      echo "ERROR: a pinned tag no longer points at the commit Cargo.lock records." >&2
      echo "Building now would reuse the OLD commit from the cargo git cache without" >&2
      echo "touching the network, producing ELFs whose vkey does not match the tag." >&2
      echo "Re-resolve deliberately (cargo update -p <crate>) and re-commit Cargo.lock." >&2
      exit 1
    fi
    if [ "$unreachable" -ne 0 ]; then
      echo
      echo "ERROR: could not reach every remote, so the pins are unverified." >&2
      echo "This is a connectivity problem, NOT a reason to touch Cargo.lock." >&2
      exit 1
    fi
    echo "All pins agree with their remotes."

# Build all ELF files.
#
# [MANTLE] Gated on `verify-git-pins` — see the rationale there.
#
# Docker caching is ON by default (the named volumes `sp1-cargo-git` /
# `sp1-cargo-registry`); `cargo-prove --no-docker-cache` turns it off. Caching only affects
# download time, not the artifacts: cargo resolves by the commit SHA in Cargo.lock either way.
#
# `ghcr.io/succinctlabs/sp1` ships an amd64-only image, so on Apple Silicon this runs through
# Docker's emulation layer — slower, but the output is the same: the guest target is
# riscv64im-succinct-zkvm-elf (cross-compiled; the committed ELFs are 64-bit RISC-V) and the
# in-container environment is identical, which is what `--docker` exists for. `.github/workflows/elf.yml` rebuilds on x64 and requires `git status --porcelain
# elf/` to be empty, so CI is the final arbiter if a host ever does diverge.
build-elfs: verify-git-pins build-range-elfs build-agg-elf

# Build ELF files for range programs.
#
# [MANTLE] Two adjustments vs. upstream Succinct Labs' justfile:
#   1. Drop the `celestia`, `eigenda` and `altda` blocks — Phase 2 deleted those
#      crates (Validity-Oracle-only runtime). Only `ethereum` remains.
#   2. Pass `--ignore-rust-version` to `cargo-prove`. SP1's docker image has
#      historically shipped an older rustc than our mantle-v2 deps (kona-genesis,
#      alloy-op-evm, etc.) declare via `rust-version = "1.94"`. That build
#      compiles those crates fine — the floor is a policy declaration, not a hard
#      requirement — so we tell cargo to skip the MSRV check.
#      TODO(v3.12.0 sync): re-test whether the SP1 v6.4.0 image still needs this.
#      If its bundled rustc is >= 1.94, drop the flag from both recipes below.
build-range-elfs:
    #!/usr/bin/env bash

    cd programs/range/ethereum
    ~/.sp1/bin/cargo-prove prove build --elf-name range-elf-embedded --docker --tag v6.4.0 --output-directory ../../../elf --ignore-rust-version

# Build ELF file for aggregation program.
#
# [MANTLE] `--ignore-rust-version` for the same SP1-1.93 vs mantle-v2-1.94
# reason documented on `build-range-elfs` above.
build-agg-elf:
    #!/usr/bin/env bash

    cd programs/aggregation
    ~/.sp1/bin/cargo-prove prove build --elf-name aggregation-elf --docker --tag v6.4.0 --output-directory ../../elf --ignore-rust-version

# Run all unit tests except for the specified ones.
tests:
   cargo t --release \
    -- \
    --skip test_cycle_count_diff \
    --skip test_post_to_github

# Run DA-specific host utility tests
# [MANTLE] Ethereum DA only — this fork is Validity-Oracle-only (MANTLE_CHANGES.md §3.1),
# so the EigenDA/Celestia/AltDA host-utils crates and their SRS setup are gone.
da-integration-tests da="ethereum":
    #!/usr/bin/env bash
    set -euo pipefail

    cargo t -p op-succinct-{{da}}-host-utils --features integration --release -- --test-threads=1 --nocapture

forge-build *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail

    cd contracts

    forge build {{ARGS}}

    # Forge build compiles only the src/ graph; the scripts/ graph is compiled by `forge script`.
    # On the first invocation, `forge script` may compile a small set of dependencies.
    # To avoid paying this cost in every CI test, we pre‑warm the script cache once here.
    #
    # Notes:
    # - A single `forge script <any script> --skip-simulation` is sufficient to compile the script
    #   dependency graph into the cache.
    forge script "script/DeployMockVerifier.s.sol" \
    --skip "/**/test/**" \
    --sig "idonotexist()" \
    --skip-simulation \
    2>/dev/null || true
