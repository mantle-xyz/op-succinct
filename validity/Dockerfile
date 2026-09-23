# Only clear-stuck-txs (the smaller scripts/utils package) is pre-cooked via
# cargo-chef here; validity's own build is unchanged from the prior baseline.
# validity intentionally isn't cached yet - this is scoped deliberately, not
# a TODO. Verified via two consecutive builds on the same Cargo.lock: 36m16s
# (first, priming the cache) -> 11m07s (second, cook layer fully cached).
FROM lukemathwalker/cargo-chef:latest-rust-1.85 AS chef
WORKDIR /build

# [MANTLE] Install the pinned toolchain once, in the stage both planner and
# builder inherit. rust-toolchain.toml pins a nightly (see that file for why:
# several deps need rustc >= 1.94, so the image's baked-in 1.85 never compiles
# anything real — rustup switches toolchains based on this file). Left to
# itself, rustup installs that nightly lazily on the first cargo invocation,
# which happened twice per build: once in planner's `cargo chef prepare`
# (~130s, mostly downloading rustc/rustc-dev/llvm-tools) and again in cook.
# planner re-runs on every source change because it starts with `COPY . .`,
# so every code change paid for a fresh toolchain download. Copying only this
# file keeps the layer keyed on the toolchain pin alone.
#
# `cargo --version` rather than `rustup show` / `rustup toolchain install`:
# it goes through the same rustup-proxy auto-install that prepare and cook
# were already triggering, whose behaviour differs less across rustup releases.
COPY rust-toolchain.toml rust-toolchain.toml
RUN cargo --version && rustc --version

FROM chef AS planner
COPY . .
RUN cargo chef prepare --bin clear-stuck-txs --recipe-path recipe.json

# Build stage
FROM chef AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    libclang-dev \
    pkg-config \
    libssl-dev \
    git \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Install Go 1.24 (required for sp1-recursion-gnark-ffi)
RUN curl -LO https://go.dev/dl/go1.24.0.linux-amd64.tar.gz && \
    tar -C /usr/local -xzf go1.24.0.linux-amd64.tar.gz && \
    rm go1.24.0.linux-amd64.tar.gz
ENV PATH="/usr/local/go/bin:${PATH}"

# [MANTLE] No SP1 toolchain here. Upstream installs one (sp1up + cargo-prove) in
# both stages, but nothing in this repo invokes it: `build_all()` in
# utils/build has every `build_program` call commented out — upstream's copy is
# empty too — so no guest program is compiled during the image build, and the
# ELFs in elf/ are committed artefacts that utils/elfs `include_bytes!`s. A grep
# for `cargo-prove`, `~/.sp1` or `SP1_HOME` across the workspace returns nothing.
# sp1-sdk itself is an ordinary crate; the only external toolchain its build
# needs is Go, installed above for sp1-recursion-gnark-ffi.
# Installing it cost ~300MB of download and 4.8GB unpacked, per stage.

# Pre-cook only clear-stuck-txs's dependency graph, keyed on the recipe
# (Cargo.toml/Cargo.lock) rather than the full source tree.
#
# rust-toolchain.toml is already at /build/ from the chef stage, so cook runs
# on the pinned nightly. It must stay there: cook's reconstructed skeleton
# doesn't otherwise contain it, and without it cargo falls back to the image's
# baked-in 1.85 and errors out on any dependency with a higher MSRV.
COPY --from=planner /build/recipe.json recipe.json
RUN --mount=type=ssh \
    cargo chef cook --release --bin clear-stuck-txs --recipe-path recipe.json

# Copy only what's needed for the build
COPY . .

# Build the server. Unchanged from baseline other than clear-stuck-txs's
# deps already being compiled by the cook step above.
#
# Deliberately NOT using --mount=type=cache here (unlike the original
# baseline): a cache mount at /build/target would shadow the real,
# already-compiled target/ directory that cook just wrote directly into
# the image filesystem, throwing away everything cook just cached and
# defeating the whole point of this change.
RUN --mount=type=ssh \
    cargo build --bin validity --bin clear-stuck-txs --release && \
    cp target/release/validity /build/validity-proposer && \
    cp target/release/clear-stuck-txs /build/clear-stuck-txs

# Final stage
#
# [MANTLE] Runtime-only image. This was `rust:1.85-slim` carrying a build-time
# package set, but nothing in this stage compiles: it copies two finished
# binaries and execs one. Debian bookworm matches the builder (cargo-chef's
# rust-1.85 image is bookworm-based), so the glibc and OpenSSL ABIs line up.
FROM debian:bookworm-slim

WORKDIR /app

# [MANTLE] Exactly what the two binaries link against at runtime, resolved with
#   cargo tree -p op-succinct-validity --target x86_64-unknown-linux-gnu
# Resolving on a developer's macOS is misleading here: native-tls selects
# Security.framework there and openssl-sys never shows up, which would make
# libssl3 look unnecessary.
#
#   ca-certificates  root store for outbound HTTPS (L1/L2 RPCs, prover network)
#   libssl3          openssl-sys 0.9, reached via native-tls <- hyper-tls <-
#                    alloy-transport-http. Not vendored (no openssl-src in the
#                    lockfile), so it links the system libssl at runtime.
#   libstdc++6       librocksdb-sys — RocksDB is C++ — reached via kona-host.
#   curl             not linked by anything; kept purely for `kubectl exec`
#                    debugging. Drop it if you want the image smaller still.
#
# Removed as build-time only: clang, libclang-dev (bindgen), pkg-config,
# libssl-dev (headers; the runtime needs libssl3, not the -dev package), git
# and jq — no code path shells out to any of them (`Command::new` appears only
# in cost-estimator and gen_sp1_test_artifacts, neither of which ships here).
#
# If a binary ever fails to start with "error while loading shared libraries",
# the missing .so names the package to add back.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    libstdc++6 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# [MANTLE] No SP1 toolchain in the runtime image either — see the note in the
# builder stage. The proposer requests proofs over the network (or from the
# cluster) and submits them to L1; it never shells out to cargo-prove, and the
# circuit artefacts sp1-sdk may need are fetched by the SDK at runtime, not
# installed by sp1up (there is no circuits/ directory under ~/.sp1 after it runs).

# Copy only the built binaries from builder
COPY --from=builder /build/validity-proposer /usr/local/bin/validity-proposer
# [MANTLE] Operational tool, shipped in the same image on purpose: it signs with the same GCP
# Cloud HSM the proposer uses, so it must run where those credentials already are. It never acts
# without --execute. See scripts/utils/bin/clear_stuck_txs.rs.
COPY --from=builder /build/clear-stuck-txs /usr/local/bin/clear-stuck-txs

# Run the server from its permanent location
CMD ["/usr/local/bin/validity-proposer"]
