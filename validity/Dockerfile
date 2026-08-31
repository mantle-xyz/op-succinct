# Chef stage: cargo-chef prebuilt against a matching rustc, rather than
# `cargo install cargo-chef` on top of plain rust:1.85 — the latest cargo-chef
# release pulls transitive deps (cargo-platform, cargo_metadata, guppy,
# target-spec) whose MSRV has since moved past 1.85, so compiling it fresh
# fails with "rustc 1.85.1 is not supported by the following packages".
# This tag pins a cargo-chef build against rustc 1.85 specifically.
FROM lukemathwalker/cargo-chef:latest-rust-1.85 AS chef
WORKDIR /build

# Planner stage: compute dependency-only recipes from the workspace
# Cargo.toml/Cargo.lock, independent of the rest of the source tree.
# `prepare --bin` only accepts a single binary, and validity / clear-stuck-txs
# live in two different workspace packages (validity/ vs scripts/utils/), so
# this needs two separate recipes rather than one shared one. Without --bin,
# cargo-chef pulls in every workspace member (including unrelated
# scripts/test crates that depend on much newer alloy-* versions requiring
# rustc up to 1.94), and `cook` fails trying to compile those against 1.85.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --bin validity --recipe-path recipe-validity.json && \
    cargo chef prepare --bin clear-stuck-txs --recipe-path recipe-clear-stuck-txs.json

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

# Build only the dependency graph first, keyed on the recipes (Cargo.toml/Cargo.lock)
# rather than the full source tree. As long as dependency versions don't change,
# these layers are reused from the registry build cache and the sp1-*/gnark crates
# don't get recompiled on every source change.
COPY --from=planner /build/recipe-validity.json recipe-validity.json
COPY --from=planner /build/recipe-clear-stuck-txs.json recipe-clear-stuck-txs.json
RUN --mount=type=ssh \
    cargo chef cook --release --bin validity --recipe-path recipe-validity.json && \
    cargo chef cook --release --bin clear-stuck-txs --recipe-path recipe-clear-stuck-txs.json

# Copy only what's needed for the build
COPY . .

# Build the server. Dependencies are already compiled by the cook step above,
# so this only (re)compiles the workspace's own crates that actually changed.
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
