# syntax=docker/dockerfile:1.7
# ==============================================================================
# Stage 1: Chef - Dependency Caching Layer
# Multi-arch: supports linux/amd64 and linux/arm64 (Graviton, Apple Silicon)
# ==============================================================================
FROM --platform=$BUILDPLATFORM lukemathwalker/cargo-chef:latest-rust-1.93 AS chef
WORKDIR /app

# ==============================================================================
# Stage 2: Planner - Generate recipe.json for dependency caching
# ==============================================================================
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ==============================================================================
# Stage 3: Builder - Build dependencies (cached) then application
# TARGETPLATFORM / TARGETARCH are injected by docker buildx automatically.
# ==============================================================================
FROM chef AS builder

ARG TARGETPLATFORM
ARG TARGETARCH
ARG BUILDPLATFORM

# Install cross-compilation toolchains when building for arm64 on amd64 host.
RUN if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
      apt-get update -qq && \
      apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        libc6-dev-arm64-cross && \
      rustup target add aarch64-unknown-linux-gnu; \
    fi

# Set Cargo target based on TARGETARCH.
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

# Copy the recipe and build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
    cargo chef cook --release --target aarch64-unknown-linux-gnu --recipe-path recipe.json; \
  else \
    cargo chef cook --release --recipe-path recipe.json; \
  fi

# Now copy source and build binaries in a single step to share
# the dependency cache layer and avoid redundant recompilation.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
  --mount=type=cache,target=/usr/local/cargo/git \
  --mount=type=cache,target=/app/target \
  if [ "$TARGETARCH" = "arm64" ] && [ "$BUILDPLATFORM" != "$TARGETPLATFORM" ]; then \
    cargo build --release --target aarch64-unknown-linux-gnu \
      --bin stellar-operator \
      --bin kubectl-stellar \
      --bin stellar-sidecar \
      --bin stellar-watcher \
      --bin stellar-fork-detector && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-operator target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/kubectl-stellar target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-sidecar target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-watcher target/release/ && \
    cp target/aarch64-unknown-linux-gnu/release/stellar-fork-detector target/release/; \
  else \
    cargo build --release \
      --bin stellar-operator \
      --bin kubectl-stellar \
      --bin stellar-sidecar \
      --bin stellar-watcher \
      --bin stellar-fork-detector; \
  fi

# Strip binaries to reduce image size
RUN strip /app/target/release/stellar-operator \
    && strip /app/target/release/kubectl-stellar \
    && strip /app/target/release/stellar-sidecar \
    && strip /app/target/release/stellar-watcher \
    && strip /app/target/release/stellar-fork-detector

# ==============================================================================
# Stage 4: Local Binaries - Fast local packaging from host build artifacts
# ==============================================================================
FROM scratch AS local-binaries
COPY target/release/stellar-operator /stellar-operator
COPY target/release/kubectl-stellar /kubectl-stellar

# ==============================================================================
# Stage 5: Runtime Local - Minimal image for local dev (no container recompile)
# ==============================================================================
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime-local

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy prebuilt local binaries
COPY --from=local-binaries /stellar-operator /stellar-operator
COPY --from=local-binaries /kubectl-stellar /kubectl-stellar

# Run as non-root user (UID 65532 is the nonroot user in distroless)
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
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy stripped binaries
COPY --from=builder /app/target/release/stellar-operator /stellar-operator
COPY --from=builder /app/target/release/kubectl-stellar /kubectl-stellar
COPY --from=builder /app/target/release/stellar-sidecar /stellar-sidecar
COPY --from=builder /app/target/release/stellar-watcher /stellar-watcher
COPY --from=builder /app/target/release/stellar-fork-detector /stellar-fork-detector

# Run as non-root user (UID 65532 is the nonroot user in distroless)
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/stellar-operator", "--health-check"] || exit 1

ENTRYPOINT ["/stellar-operator"]
