# ============================================================================
# Mnemo — Multi-stage Dockerfile
# Target: <50MB production image
# ============================================================================

# ── Stage 1: Build ──────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder

WORKDIR /build

ARG ORT_VERSION=1.23.0
ARG TARGETARCH

# Install build dependencies (protobuf-compiler needed by mnemo-proto / tonic-build)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Download ONNX Runtime shared library for fastembed local embeddings.
RUN mkdir -p /opt/ort \
    && case "${TARGETARCH}" in \
        amd64) ORT_ARCH="x64" ;; \
        arm64) ORT_ARCH="aarch64" ;; \
        *) echo "Unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
       esac \
    && curl -fsSL "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}.tgz" -o /tmp/onnxruntime.tgz \
    && tar -xzf /tmp/onnxruntime.tgz -C /opt/ort \
    && rm /tmp/onnxruntime.tgz

# Cache dependency compilation
COPY Cargo.toml ./
COPY crates/mnemo-core/Cargo.toml crates/mnemo-core/Cargo.toml
COPY crates/mnemo-server/Cargo.toml crates/mnemo-server/Cargo.toml
COPY crates/mnemo-storage/Cargo.toml crates/mnemo-storage/Cargo.toml
COPY crates/mnemo-graph/Cargo.toml crates/mnemo-graph/Cargo.toml
COPY crates/mnemo-ingest/Cargo.toml crates/mnemo-ingest/Cargo.toml
COPY crates/mnemo-retrieval/Cargo.toml crates/mnemo-retrieval/Cargo.toml
COPY crates/mnemo-llm/Cargo.toml crates/mnemo-llm/Cargo.toml
COPY crates/mnemo-proto/Cargo.toml crates/mnemo-proto/Cargo.toml
COPY crates/mnemo-mcp/Cargo.toml crates/mnemo-mcp/Cargo.toml
COPY crates/mnemo-gnn/Cargo.toml crates/mnemo-gnn/Cargo.toml
COPY crates/mnemo-lora/Cargo.toml crates/mnemo-lora/Cargo.toml

# Create stub source files for dependency caching
RUN for crate in mnemo-core mnemo-server mnemo-storage mnemo-graph mnemo-ingest mnemo-retrieval mnemo-llm mnemo-proto mnemo-mcp mnemo-gnn mnemo-lora; do \
      mkdir -p "crates/$crate/src" && \
      echo "fn main() {}" > "crates/$crate/src/lib.rs"; \
    done

# Copy proto definitions and build.rs needed by mnemo-proto's tonic-build
COPY proto/ proto/
COPY crates/mnemo-proto/build.rs crates/mnemo-proto/build.rs

# Generate lockfile inside the build context
RUN cargo generate-lockfile

# Build dependencies only (cached layer)
RUN cargo build --release 2>/dev/null || true

# Copy actual source code
COPY crates/ crates/

# Touch source files to invalidate cache for our code only
RUN find crates -name "*.rs" -exec touch {} +

# Build the actual application (server + MCP stdio bridge)
RUN cargo build --release --bin mnemo-server --bin mnemo-mcp-server

# ── Stage 2: App payload for minimal runtime ────────────────────────
FROM debian:bookworm-slim AS runtime-rootfs

ARG ORT_VERSION=1.23.0
ARG TARGETARCH

RUN mkdir -p /rootfs/app/lib /rootfs/app/config /rootfs/app/.fastembed_cache

RUN --mount=from=builder,source=/opt/ort,target=/builder-ort,ro \
    --mount=from=builder,source=/,target=/builder-root,ro \
    case "${TARGETARCH}" in \
      amd64) \
        ORT_ARCH="x64"; \
        GNU_TRIPLE="x86_64-linux-gnu" \
        ;; \
      arm64) \
        ORT_ARCH="aarch64"; \
        GNU_TRIPLE="aarch64-linux-gnu" \
        ;; \
      *) echo "Unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && LIBDIR="/builder-root/lib/${GNU_TRIPLE}" \
    && if [ ! -f "${LIBDIR}/libssl.so.3" ]; then LIBDIR="/builder-root/usr/lib/${GNU_TRIPLE}"; fi \
    && cp "/builder-ort/onnxruntime-linux-${ORT_ARCH}-${ORT_VERSION}/lib/libonnxruntime.so.${ORT_VERSION}" /rootfs/app/lib/libonnxruntime.so.${ORT_VERSION} \
    && cp "${LIBDIR}/libssl.so.3" /rootfs/app/lib/libssl.so.3 \
    && cp "${LIBDIR}/libcrypto.so.3" /rootfs/app/lib/libcrypto.so.3 \
    && cp "${LIBDIR}/libgcc_s.so.1" /rootfs/app/lib/libgcc_s.so.1 \
    && cp "${LIBDIR}/libstdc++.so.6" /rootfs/app/lib/libstdc++.so.6

COPY --from=builder /build/target/release/mnemo-server /rootfs/app/mnemo-server
COPY --from=builder /build/target/release/mnemo-mcp-server /rootfs/app/mnemo-mcp-server
COPY config/ /rootfs/app/config/

RUN ln -s "libonnxruntime.so.${ORT_VERSION}" /rootfs/app/lib/libonnxruntime.so.1 \
    && ln -s "libonnxruntime.so.${ORT_VERSION}" /rootfs/app/lib/libonnxruntime.so

# ── Stage 3: Final image ────────────────────────────────────────────
FROM gcr.io/distroless/base-nossl-debian12:nonroot AS runtime

WORKDIR /app

COPY --from=runtime-rootfs --chown=65532:65532 /rootfs/app/ /app/

USER 65532:65532

EXPOSE 8080 50051

ENV HOME=/app
ENV FASTEMBED_CACHE_PATH=/app/.fastembed_cache
ENV MNEMO_CONFIG=/app/config/default.toml
ENV ORT_DYLIB_PATH=/app/lib/libonnxruntime.so.1.23.0
ENV LD_LIBRARY_PATH=/app/lib

ENTRYPOINT ["/app/mnemo-server"]
