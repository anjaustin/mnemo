# ============================================================================
# Mnemo — Multi-stage Dockerfile
# Target: <50MB production image
# ============================================================================

# ── Stage 1: Build ──────────────────────────────────────────────────
FROM rust:stable-slim-bookworm AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependency compilation
COPY Cargo.toml ./
COPY crates/mnemo-core/Cargo.toml crates/mnemo-core/Cargo.toml
COPY crates/mnemo-server/Cargo.toml crates/mnemo-server/Cargo.toml
COPY crates/mnemo-storage/Cargo.toml crates/mnemo-storage/Cargo.toml
COPY crates/mnemo-graph/Cargo.toml crates/mnemo-graph/Cargo.toml
COPY crates/mnemo-ingest/Cargo.toml crates/mnemo-ingest/Cargo.toml
COPY crates/mnemo-retrieval/Cargo.toml crates/mnemo-retrieval/Cargo.toml
COPY crates/mnemo-llm/Cargo.toml crates/mnemo-llm/Cargo.toml

# Create stub source files for dependency caching
RUN for crate in mnemo-core mnemo-server mnemo-storage mnemo-graph mnemo-ingest mnemo-retrieval mnemo-llm; do \
      mkdir -p "crates/$crate/src" && \
      echo "fn main() {}" > "crates/$crate/src/lib.rs"; \
    done

# Generate lockfile inside the build context
RUN cargo generate-lockfile

# Build dependencies only (cached layer)
RUN cargo build --release 2>/dev/null || true

# Copy actual source code
COPY crates/ crates/

# Touch source files to invalidate cache for our code only
RUN find crates -name "*.rs" -exec touch {} +

# Build the actual application
RUN cargo build --release --bin mnemo-server

# ── Stage 2: Runtime ────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd --create-home --shell /bin/bash mnemo

WORKDIR /app

# Copy the binary
COPY --from=builder /build/target/release/mnemo-server /app/mnemo-server

# Copy default config
COPY config/ /app/config/

# Set ownership
RUN chown -R mnemo:mnemo /app

USER mnemo

EXPOSE 8080 50051

ENV MNEMO_CONFIG=/app/config/default.toml

ENTRYPOINT ["/app/mnemo-server"]
