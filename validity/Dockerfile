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

# [MANTLE] sp1-recursion-gnark-ffi's build script fetches Go modules, and one of
# them — github.com/consensys/gnark-ignition-verifier, pinned in its go.sum —
# now 404s on GitHub and is no longer served by proxy.golang.org (checked
# 2026-10-13; every SP1 version from 6.4.0 to 6.8.1 needs it). goproxy.cn still
# serves that exact version. Try the default proxy first and fall back; go
# verifies whatever it downloads against go.sum, so the fallback cannot swap
# content. Without this, any cold cook layer (i.e. every Cargo.lock change)
# fails. Same fix as mantle-xyz/sp1-cluster 7a825cd.
ENV GOPROXY="https://proxy.golang.org,https://goproxy.cn,direct"

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

# [MANTLE] The runtime image has no shell, so its writable working directory is made here. A
# separate stage on purpose: creating it in the builder would change the builder's last layer
# and with it the build cache, for a change that only concerns the runtime image.
FROM chef AS appdir
RUN mkdir -p /out/app/configs

# [MANTLE] Runtime: distroless `cc`, non-root. No shell, no package manager.
#
# Why `cc` and not `static` (what mantle-v2 uses for its Go services): these binaries are
# dynamically linked against glibc. readelf NEEDED for both validity-proposer and
# clear-stuck-txs:
#   libssl.so.3 / libcrypto.so.3   openssl-sys, via native-tls <- hyper-tls <-
#                                  alloy-transport-http. Not vendored (no openssl-src).
#   libstdc++.so.6 / libgcc_s.so.1 librocksdb-sys — RocksDB is C++ — via kona-host.
#   libc.so.6 / libm.so.6 / ld-linux-x86-64.so.2
# distroless/cc-debian12 ships exactly these plus the CA root store; distroless/static has none
# of the libraries. If a binary fails with "error while loading shared libraries", the missing
# .so is a dependency that appeared since — check it is in this image.
#
# glibc: the builder (rust 1.85 / cargo-chef) and this image are both Debian 12 (glibc 2.36). If
# the builder moves to a newer Debian, move this image with it or the binaries will refuse to
# load (GLIBC_2.xx not found).
#
# Replaces rust:1.85-slim + clang, pkg-config, libssl-dev, git, libclang-dev, jq, curl and an
# SP1 toolchain (sp1up). None of them is used at runtime: those are build tools, the ELFs are
# committed and embedded with include_bytes!, and nothing shells out to cargo-prove.
#
# Pinned by digest — the multi-arch index digest, so the same pin resolves on amd64 (CI) and
# arm64 (local). The tag is kept for readability only; the digest decides what is pulled.
# distroless republishes `nonroot` for security fixes, so bump this deliberately:
#   docker buildx imagetools inspect gcr.io/distroless/cc-debian12:nonroot   # top "Digest:"
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

# The proposer caches the rollup / L1 configs under ./configs (fetcher.rs) and a failed write of
# the rollup config is fatal. The image runs as nonroot (uid 65532), so that directory must be
# owned by it.
COPY --from=appdir --chown=65532:65532 /out/app /app
WORKDIR /app

# Copy only the built binaries from builder
COPY --from=builder /build/validity-proposer /usr/local/bin/validity-proposer
# [MANTLE] Operational tool, shipped in the same image on purpose: it builds its signer with the
# same Signer::from_env() as the proposer (whichever remote signer backend is configured), so it
# must run where that signer's credentials / identity already are. It never acts
# without --execute. Runs without a shell: `kubectl exec <pod> -- /usr/local/bin/clear-stuck-txs`.
# See scripts/utils/bin/clear_stuck_txs.rs.
COPY --from=builder /build/clear-stuck-txs /usr/local/bin/clear-stuck-txs

# Exec form: there is no /bin/sh in this image. Deployments must start the binary directly —
# a `command: [/bin/sh, -c]` wrapper will not run here.
CMD ["/usr/local/bin/validity-proposer"]
