# ==============================================================================
# Unified Processor Service - Dockerfile (Optimized Native Rust)
# ==============================================================================
# Multi-stage build for pure Rust service
# Port: 8090
# ==============================================================================

# ==============================================================================
# Stage 1: Rust builder
# ==============================================================================
FROM debian:bookworm-slim AS rust-builder

ARG RUST_VERSION=stable

# Install build-time dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    pkg-config \
    libssl-dev \
    libcurl4-openssl-dev \
    zlib1g-dev \
    cmake \
    build-essential \
    libsasl2-dev \
    librdkafka-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Rust via rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain ${RUST_VERSION}
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /app

# ---------------------------------------------------------------------------
# Dependency caching layer
# ---------------------------------------------------------------------------
COPY Cargo.toml Cargo.lock* ./

# Build a minimal stub that mirrors the actual lib.rs module layout so cargo
# can compile all crate dependencies without the real source files.
RUN mkdir -p \
        api \
        src/core \
        src/processors \
        src/infra \
        src/graph \
        src/utils && \
    echo 'fn main() {}' > api/index.rs && \
    printf 'pub mod core;\npub mod processors;\npub mod infra;\npub mod graph;\n' > src/lib.rs && \
    touch \
        src/core/mod.rs \
        src/processors/mod.rs \
        src/infra/mod.rs \
        src/graph/mod.rs

# Cache dependencies
RUN cargo build --release 2>/dev/null; \
    cargo clean -p repo-uni-proc 2>/dev/null; \
    true

# ---------------------------------------------------------------------------
# Real build
# ---------------------------------------------------------------------------
RUN rm -rf src/* api/*
COPY src/ ./src/
COPY api/ ./api/

RUN cargo build --release --features kafka

# ==============================================================================
# Stage 2: Runtime image (Lightweight native Rust)
# ==============================================================================
FROM debian:bookworm-slim AS runtime

# Install only shared libraries needed by compiled Rust binary
RUN apt-get update && apt-get install -y --no-install-recommends \
    dumb-init \
    curl \
    ca-certificates \
    libpq5 \
    libssl3 \
    librdkafka1 \
    libsasl2-2 \
    && rm -rf /var/lib/apt/lists/*

ENV LD_LIBRARY_PATH="/usr/lib/x86_64-linux-gnu:/usr/local/lib:$LD_LIBRARY_PATH"

# SECURITY: Create a non-root user and group
RUN groupadd -r appgroup && useradd -r -g appgroup appuser

WORKDIR /app

# Copy the compiled Rust binary and set ownership
COPY --from=rust-builder --chown=appuser:appgroup /app/target/release/unified-processor /usr/local/bin/unified-processor

# Ensure the appuser owns the working directory
RUN chown -R appuser:appgroup /app

# SECURITY: Switch to the non-root user
USER appuser

ENV PORT=8090
EXPOSE 8090

# Health check optimized for Render
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:${PORT:-8090}/health || exit 1

ENTRYPOINT ["dumb-init", "--"]
CMD ["unified-processor"]

