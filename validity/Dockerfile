# Only clear-stuck-txs (the smaller scripts/utils package) is pre-cooked via
# cargo-chef here; validity's own build is unchanged from the prior baseline.
# validity intentionally isn't cached yet - this is scoped deliberately, not
# a TODO. Verified via two consecutive builds on the same Cargo.lock: 36m16s
# (first, priming the cache) -> 11m07s (second, cook layer fully cached).
FROM lukemathwalker/cargo-chef:latest-rust-1.85 AS chef
WORKDIR /build

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

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up -v 6.4.0 && \
    ~/.sp1/bin/cargo-prove prove --version

# Pre-cook only clear-stuck-txs's dependency graph, keyed on the recipe
# (Cargo.toml/Cargo.lock) rather than the full source tree.
#
# rust-toolchain.toml pins a nightly channel (see that file for why — several
# deps require rustc >= 1.94, hence FROM rust:1.85 alone isn't actually what
# compiles anything real: rustup auto-switches based on this file). cook's
# reconstructed skeleton dir doesn't otherwise contain it, so without this
# COPY, cargo falls back to the image's baked-in 1.85 and errors out on any
# dependency with a higher MSRV — unrelated to which --bin is requested.
COPY --from=planner /build/rust-toolchain.toml rust-toolchain.toml
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
FROM rust:1.85-slim

WORKDIR /app

# Install required runtime dependencies
RUN apt-get update && apt-get install -y \
    curl \
    clang \
    pkg-config \
    libssl-dev \
    ca-certificates \
    git \
    libclang-dev \
    jq \
    && rm -rf /var/lib/apt/lists/*

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up -v 6.4.0 && \
    ~/.sp1/bin/cargo-prove prove --version

# Copy only the built binaries from builder
COPY --from=builder /build/validity-proposer /usr/local/bin/validity-proposer
# [MANTLE] Operational tool, shipped in the same image on purpose: it signs with the same GCP
# Cloud HSM the proposer uses, so it must run where those credentials already are. It never acts
# without --execute. See scripts/utils/bin/clear_stuck_txs.rs.
COPY --from=builder /build/clear-stuck-txs /usr/local/bin/clear-stuck-txs

# Run the server from its permanent location
CMD ["/usr/local/bin/validity-proposer"]
