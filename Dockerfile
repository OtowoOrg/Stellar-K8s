# syntax=docker/dockerfile:1.7
# ==============================================================================
# Stage 1: Chef - Dependency Caching Layer
# (linux/amd64 only)
# ==============================================================================
FROM lukemathwalker/cargo-chef:latest-rust-1.95-bookworm AS chef
WORKDIR /app

# ==============================================================================
# Stage 2: Planner - Generate recipe.json for dependency caching
# ==============================================================================
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ==============================================================================
# Stage 3: Builder - Build dependencies (cached) then application
# ==============================================================================
FROM chef AS builder

# Install system dependencies
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
      cmake \
      libssl-dev \
      libsasl2-dev \
      pkg-config && \
    rm -rf /var/lib/apt/lists/*

# Copy the recipe and build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  cargo chef cook --release --recipe-path recipe.json

# Now copy source and build binaries in a single step to share
# the dependency cache layer and avoid redundant recompilation.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  cargo build --release \
    --bin stellar-operator \
    --bin kubectl-stellar \
    --bin stellar-sidecar \
    --bin stellar-watcher \
    --bin stellar-fork-detector \
    --bin stellar-health-sidecar && \
  mkdir -p /app/bin && \
  cp /app/target/release/stellar-operator /app/bin/ && \
  cp /app/target/release/kubectl-stellar /app/bin/ && \
  cp /app/target/release/stellar-sidecar /app/bin/ && \
  cp /app/target/release/stellar-watcher /app/bin/ && \
  cp /app/target/release/stellar-fork-detector /app/bin/ && \
  cp /app/target/release/stellar-health-sidecar /app/bin/ && \
  strip /app/bin/stellar-operator \
    /app/bin/kubectl-stellar \
    /app/bin/stellar-sidecar \
    /app/bin/stellar-watcher \
    /app/bin/stellar-fork-detector \
    /app/bin/stellar-health-sidecar

# ==============================================================================
# Stage 4: Local Binaries - Fast local packaging from host build artifacts
# ==============================================================================
FROM scratch AS local-binaries
COPY target/release/stellar-operator /stellar-operator
COPY target/release/kubectl-stellar /kubectl-stellar

# ==============================================================================
# Stage 5: Runtime Local - Minimal image for local dev (no container recompile)
# ==============================================================================
FROM debian:bookworm-slim AS runtime-local

# Install runtime dependencies for dynamic linking
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      libssl3 \
      libsasl2-2 \
      liblzma5 \
      libzstd1 \
      libbz2-1.0 && \
    rm -rf /var/lib/apt/lists/*

# Create nonroot user
RUN useradd -u 65532 -U -m -s /bin/bash nonroot

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy prebuilt local binaries
COPY --from=local-binaries /stellar-operator /stellar-operator
COPY --from=local-binaries /kubectl-stellar /kubectl-stellar

# Run as nonroot user
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/stellar-operator", "--health-check"] || exit 1

ENTRYPOINT ["/stellar-operator"]

# ==============================================================================
# Stage 6: Runtime - Minimal distroless image (~15-20MB total)
# ==============================================================================
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies for dynamic linking
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      libssl3 \
      libsasl2-2 \
      liblzma5 \
      libzstd1 \
      libbz2-1.0 && \
    rm -rf /var/lib/apt/lists/*

# Create nonroot user
RUN useradd -u 65532 -U -m -s /bin/bash nonroot

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy stripped binaries
COPY --from=builder /app/bin/stellar-operator /stellar-operator
COPY --from=builder /app/bin/kubectl-stellar /kubectl-stellar
COPY --from=builder /app/bin/stellar-sidecar /stellar-sidecar
COPY --from=builder /app/bin/stellar-watcher /stellar-watcher
COPY --from=builder /app/bin/stellar-fork-detector /stellar-fork-detector
COPY --from=builder /app/bin/stellar-health-sidecar /stellar-health-sidecar

# Run as nonroot user
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/stellar-operator", "--health-check"] || exit 1

ENTRYPOINT ["/stellar-operator"]
