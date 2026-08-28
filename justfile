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

  cd contracts && forge script script/validity/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader  --rpc-url $L1_RPC --private-key $ADMIN_PK $VERIFY --broadcast --slow

# Deploy OPSuccinct FDG contracts
deploy-fdg-contracts env_file=".env" *features='':
    #!/usr/bin/env bash
    set -aeo pipefail
    
    # First fetch FDG config using the env file
    echo "Fetching Fault Dispute Game configuration..."
    if [ -z "{{features}}" ]; then
        RUST_LOG=info cargo run --bin fetch-fault-dispute-game-config --release -- --env-file {{env_file}}
    else
        echo "Fetching fault dispute game config with features: {{features}}"
        RUST_LOG=info cargo run --bin fetch-fault-dispute-game-config --release --features {{features}} -- --env-file {{env_file}}
    fi
    
    # Load environment variables from project root
    source {{env_file}}
    
    # Load environment variables from contracts directory if it exists
    if [ -f "contracts/.env" ]; then
        source contracts/.env
    fi
    
    # Check if required environment variables are set
    if [ -z "${RPC_URL:-}" ] && [ -z "${L1_RPC:-}" ]; then
        echo "Error: Neither RPC_URL nor L1_RPC environment variable is set"
        exit 1
    fi
    
    if [ -z "${PRIVATE_KEY:-}" ]; then
        echo "Error: PRIVATE_KEY environment variable is not set"
        exit 1
    fi
    
    # Use RPC_URL if set, otherwise fall back to L1_RPC
    RPC_URL_TO_USE="${RPC_URL:-$L1_RPC}"
    echo "Using RPC URL: $RPC_URL_TO_USE"

    echo "Deploying FDG contracts..."
    
    # Change to contracts directory
    cd contracts

    # Install dependencies only if not already present 
    # (avoids git lock conflicts in parallel test runs)
    if [ ! -d "lib/forge-std" ]; then
        echo "Installing forge dependencies..."
        forge install
    else
        echo "Forge dependencies already installed, skipping..."
    fi

    # Build contracts
    echo "Building contracts..."
    forge build
    
    # Setup verification flags
    VERIFY=""
    if [ -n "${ETHERSCAN_API_KEY:-}" ]; then
        VERIFY="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY --retries 10 --delay 5"
        echo "Verification enabled with Etherscan"
    fi
    
    # Run deployment script
    echo "Running deployment script..."
    forge script script/fp/DeployOPSuccinctFDG.s.sol \
        --broadcast \
        --slow \
        --rpc-url "$RPC_URL_TO_USE" \
        --private-key "$PRIVATE_KEY" \
        $VERIFY
    
    echo "FDG contract deployment complete!"

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
    
    forge script script/validity/DeployMockVerifier.s.sol:DeployMockVerifier \
    --rpc-url $L1_RPC \
    --private-key $PRIVATE_KEY \
    --broadcast \
    $VERIFY

# Upgrade the game implementation contract (for hardfork/upgrade)
# This script deploys a new OPSuccinctFaultDisputeGame implementation and sets it in the factory.
# Required env vars: FACTORY_ADDRESS, GAME_TYPE, VERIFIER_ADDRESS, ANCHOR_STATE_REGISTRY, ACCESS_MANAGER,
#                    AGGREGATION_VKEY, RANGE_VKEY_COMMITMENT, ROLLUP_CONFIG_HASH,
#                    MAX_CHALLENGE_DURATION, MAX_PROVE_DURATION, CHALLENGER_BOND_WEI
upgrade-game-impl env_file=".env":
    #!/usr/bin/env bash
    set -aeo pipefail

    source {{env_file}}

    if [ -z "$L1_RPC" ]; then
        echo "L1_RPC not set in {{env_file}}"
        exit 1
    fi

    if [ -z "$PRIVATE_KEY" ]; then
        echo "PRIVATE_KEY not set in {{env_file}}"
        exit 1
    fi

    cd contracts

    echo "Upgrading game implementation..."
    forge script script/fp/UpgradeOPSuccinctFDG.s.sol \
        --rpc-url "$L1_RPC" \
        --private-key "$PRIVATE_KEY" \
        --broadcast \
        --slow

    echo "Game implementation upgrade complete!"

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
    $ENV_VARS forge script script/validity/OPSuccinctDeployer.s.sol:OPSuccinctDeployer \
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
        env $ENV_VARS forge script script/validity/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader \
            --rpc-url $L1_RPC \
            --private-key $PRIVATE_KEY
    else
        env $ENV_VARS forge script script/validity/OPSuccinctUpgrader.s.sol:OPSuccinctUpgrader \
            --rpc-url $L1_RPC \
            --private-key $PRIVATE_KEY \
            $VERIFY_FLAGS \
            --broadcast
    fi

deploy-dispute-game-factory env_file=".env":
    #!/usr/bin/env bash
    set -euo pipefail
    
    # Load environment variables
    source {{env_file}}

    # Check if required environment variables are set.
    if [ -z "${L2OO_ADDRESS:-}" ]; then
        echo "Error: L2OO_ADDRESS environment variable is not set"
        exit 1
    fi
    if [ -z "${PROPOSER_ADDRESSES:-}" ]; then
        echo "Error: PROPOSER_ADDRESSES environment variable is not set"
        exit 1
    fi

    # cd into contracts directory
    cd contracts

    # forge install
    forge install

    VERIFY=""
    if [ -n "$ETHERSCAN_API_KEY" ]; then
      VERIFY="--verify --verifier etherscan --etherscan-api-key $ETHERSCAN_API_KEY"
    fi
    
    # Run the forge deployment script
    env L2OO_ADDRESS=$L2OO_ADDRESS \
        PROPOSER_ADDRESSES=$PROPOSER_ADDRESSES \
        forge script script/validity/OPSuccinctDGFDeployer.s.sol:OPSuccinctDFGDeployer \
        --rpc-url $L1_RPC \
        --private-key $PRIVATE_KEY \
        --broadcast \
        $VERIFY

# Upgrade the OPSuccinct Fault Dispute Game implementation.
upgrade-fault-dispute-game env_file="fault-proof/.env.upgrade":
    #!/usr/bin/env bash
    set -aeo pipefail

    # Load environment variables
    source {{env_file}}

    # cd into contracts directory.
    cd contracts

    # Install dependencies.
    forge install

    # Run the forge upgrade script.
    if [ "${DRY_RUN}" = "false" ]; then
        if [ -z "${PRIVATE_KEY:-}" ]; then
            echo "Error: PRIVATE_KEY environment variable is required when DRY_RUN=false"
            exit 1
        fi

        forge script script/fp/UpgradeOPSuccinctFDG.s.sol:UpgradeOPSuccinctFDG \
            --rpc-url $L1_RPC \
            --private-key $PRIVATE_KEY \
            --etherscan-api-key $ETHERSCAN_API_KEY \
            --broadcast
    else
        forge script script/fp/UpgradeOPSuccinctFDG.s.sol:UpgradeOPSuccinctFDG \
            --sig "getUpgradeCalldata()"
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
        forge script script/validity/OPSuccinctParameterUpdater.s.sol:OPSuccinctParameterUpdater \
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
        forge script script/validity/OPSuccinctParameterUpdater.s.sol:OPSuccinctParameterUpdater \
        --sig "removeConfig(string)" "{{config_name}}" \
        --rpc-url $L1_RPC \
        --private-key $PRIVATE_KEY \
        --broadcast

# Generate verification key hashes for all DA variants.
vkeys:
    #!/usr/bin/env bash
    set -e

    echo "Generating verification key hashes..."
    echo ""

    # Ethereum DA
    ETH_OUTPUT=$(RUST_LOG=error cargo run --release --bin config 2>&1)
    ETH_RANGE=$(echo "$ETH_OUTPUT" | grep "Range Verification Key Hash" | awk '{print $NF}')
    AGG_KEY=$(echo "$ETH_OUTPUT" | grep "Aggregation Verification Key Hash" | awk '{print $NF}')

    # Celestia DA
    CEL_OUTPUT=$(RUST_LOG=error cargo run --release --bin config --features celestia 2>&1)
    CEL_RANGE=$(echo "$CEL_OUTPUT" | grep "Range Verification Key Hash" | awk '{print $NF}')

    # EigenDA
    EIGEN_OUTPUT=$(RUST_LOG=error cargo run --release --bin config --features eigenda 2>&1)
    EIGEN_RANGE=$(echo "$EIGEN_OUTPUT" | grep "Range Verification Key Hash" | awk '{print $NF}')

    echo "## Verification Key Hashes"
    echo ""
    echo "| Program | Verification Key Hash |"
    echo "|--------|------------------------|"
    echo "| Ethereum DA Range Verification Key | **$ETH_RANGE** |"
    echo "| Celestia DA Range Verification Key | **$CEL_RANGE** |"
    echo "| EigenDA Range Verification Key | **$EIGEN_RANGE** |"
    echo "| Aggregation Verification Key | **$AGG_KEY** |"

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
    echo "Verifying $(wc -l < "$tmp" | tr -d ' ') tag-pinned git dependencies against their remotes..."
    fail=0
    while IFS= read -r pin; do
      url="${pin#git+}"; url="${url%%\?tag=*}"
      rest="${pin#*\?tag=}"; tag="${rest%%#*}"; sha="${rest##*#}"
      # Annotated tags need the ^{} peel to reach the commit; fall back for lightweight tags.
      remote=$(git ls-remote "$url" "refs/tags/$tag^{}" 2>/dev/null | awk 'NR==1{print $1}')
      if [ -z "$remote" ]; then
        remote=$(git ls-remote "$url" "refs/tags/$tag" 2>/dev/null | awk 'NR==1{print $1}')
      fi
      if [ "$remote" = "$sha" ]; then
        printf '  OK        %-34s %s\n' "$tag" "${url##*/}"
      else
        printf '  MISMATCH  %-34s %s\n            lock:   %s\n            remote: %s\n' \
          "$tag" "${url##*/}" "$sha" "${remote:-<tag not found>}"
        fail=1
      fi
    done < "$tmp"
    rm -f "$tmp"
    if [ "$fail" -ne 0 ]; then
      echo
      echo "ERROR: a pinned tag no longer points at the commit Cargo.lock records."
      echo "Building now would reuse the OLD commit from the cargo git cache without"
      echo "touching the network, producing ELFs whose vkey does not match the tag."
      echo "Re-resolve deliberately (cargo update -p <crate>) and re-commit Cargo.lock."
      exit 1
    fi
    echo "All pins agree with their remotes."

# Build all ELF files.
#
# [MANTLE] Gated on `verify-git-pins` — see the rationale there. ELF builds must run on x86_64:
# the SP1 image (`ghcr.io/succinctlabs/sp1`) ships amd64 only, and `.github/workflows/elf.yml`
# checks `git status --porcelain elf/` is empty against an x64 rebuild, so artifacts produced
# under emulation cannot be trusted to match.
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

# Run fault-proof integration tests
# target: test file (integration, sync, etc.)
# da: DA feature (ethereum, eigenda, celestia). DA-agnostic tests like sync work with any.
fp-integration-tests target="integration" da="ethereum":
  cd fault-proof && cargo t --test {{target}} --release --features integration,{{da}} -- --test-threads=1 --nocapture

# Run DA-specific host utility tests
# da: ethereum, eigenda, celestia
da-integration-tests da="ethereum":
    #!/usr/bin/env bash
    set -euo pipefail

    # EigenDA tests require SRS file - create symlink if needed
    if [ "{{da}}" = "eigenda" ] && [ ! -e "utils/eigenda/host/resources" ]; then
        if [ ! -d "resources" ]; then
            echo "Error: resources/ directory not found. Run from workspace root."
            exit 1
        fi
        ln -sf ../../../resources utils/eigenda/host/resources
        echo "Created symlink: utils/eigenda/host/resources -> resources/"
    fi

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
    forge script "script/validity/DeployMockVerifier.s.sol" \
    --skip "/**/test/**" \
    --sig "idonotexist()" \
    --skip-simulation \
    2>/dev/null || true
