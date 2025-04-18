# syntax=docker/dockerfile:1.4

# Build stage
FROM ubuntu:24.04 AS builder

WORKDIR /build

# Install Rust and required dependencies
RUN apt-get update && apt-get install -y \
    curl \
    clang \
    build-essential \
    git \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH
RUN rustup install nightly
RUN rustup default nightly

# Verify Rust installation
RUN rustc --version && cargo --version

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up && \
    ~/.sp1/bin/cargo-prove prove --version
    
# Copy only what's needed for the build
COPY Cargo.toml Cargo.lock ./
COPY proposer/succinct ./proposer/succinct
COPY elf ./elf
COPY utils ./utils
COPY programs ./programs
COPY scripts ./scripts

# Build the server
RUN --mount=type=ssh \
    --mount=type=cache,target=/root/.cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --bin server --release && \
    cp target/release/server /build/server

# Final stage
FROM ubuntu:24.04

WORKDIR /app

# Install Rust and required dependencies
RUN apt-get update && apt-get install -y \
    curl \
    clang \
    build-essential \
    git \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH=/root/.cargo/bin:$PATH
RUN rustup install nightly
RUN rustup default nightly

# Verify Rust installation
RUN rustc --version && cargo --version

# Install SP1
RUN curl -L https://sp1.succinct.xyz | bash && \
    ~/.sp1/bin/sp1up && \
    ~/.sp1/bin/cargo-prove prove --version

# Copy only the built binaries from builder
COPY --from=builder /build/server /usr/local/bin/server
COPY resources ./resources

# Expose port based on environment variable or default to 3000
ENV PORT=${PORT:-3000}
EXPOSE $PORT

# Run the server from its permanent location
CMD ["/usr/local/bin/server"]
