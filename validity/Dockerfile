# Build stage
FROM rust:1.85 AS builder

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


WORKDIR /build

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up -v 6.4.0 && \
    ~/.sp1/bin/cargo-prove prove --version

# Copy only what's needed for the build
COPY . .

# Build the server
RUN --mount=type=ssh \
    --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/build/target \
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
