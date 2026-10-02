# syntax=docker/dockerfile:1.7
# =============================================================================
# wasm-opt-sidecar — Alpine Linux container with Binaryen wasm-opt
#
# Purpose
# -------
# Runs as a sidecar Pod alongside the stellar-webhook operator to provide the
# wasm-opt bytecode optimizer as a lightweight HTTP service.  The main operator
# forwards raw WASM binaries to this sidecar's /optimize endpoint and receives
# the optimised bytes back — all within the 10-second Kubernetes webhook budget.
#
# Usage
# -----
# Build:
#   docker build -f sidecar/wasm-opt.Dockerfile -t ghcr.io/stellar/wasm-opt-sidecar:latest .
#
# Run locally (for testing):
#   docker run --rm -p 9080:9080 ghcr.io/stellar/wasm-opt-sidecar:latest
#
# Verify:
#   curl -s -X POST \
#     -H 'Content-Type: application/wasm' \
#     --data-binary @contract.wasm \
#     http://localhost:9080/optimize?level=3 > contract.optimised.wasm
#
# Image size target: < 30 MB (Alpine base + binaryen package only)
# =============================================================================

# =============================================================================
# Stage 1 — install Binaryen toolchain on Alpine
# =============================================================================
FROM alpine:3.21 AS base

# Metadata
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="wasm-opt HTTP sidecar for Stellar-K8s WASM bytecode optimisation"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL org.opencontainers.image.title="wasm-opt-sidecar"

# Install only what is strictly needed:
#   binaryen      — provides wasm-opt binary (dead-code elimination, memory
#                   packing, etc.)
#   tini          — PID 1 init shim for correct signal handling in containers
# No build tools are needed — wasm-opt runs entirely at runtime.
RUN apk add --no-cache \
        binaryen \
        tini \
    && wasm-opt --version \
    && echo "binaryen installed OK"

# =============================================================================
# Stage 2 — copy the pre-built sidecar HTTP server binary
#
# The sidecar binary is produced by the main Cargo workspace build.  If you
# are building the Docker image from within the CI pipeline the binary already
# exists under target/release/stellar-sidecar.  For purely sidecar-only builds
# uncomment the cargo build section below instead.
# =============================================================================
FROM base AS runtime

# Create a minimal non-root user.
RUN addgroup -S sidecar && adduser -S -G sidecar sidecar

# Copy the pre-built sidecar binary (built by the main Dockerfile builder stage).
# The sidecar binary wraps optimizer::run_sidecar_server and listens on :9080.
COPY --chown=sidecar:sidecar target/release/stellar-sidecar /usr/local/bin/stellar-sidecar
RUN chmod +x /usr/local/bin/stellar-sidecar

# wasm-opt must be on PATH for the subprocess optimizer path.
# binaryen installs it to /usr/bin/wasm-opt on Alpine.
ENV WASM_OPT_BIN=/usr/bin/wasm-opt
ENV WASM_OPT_LEVEL=3
ENV SIDECAR_BIND=0.0.0.0:9080

# Expose the optimizer HTTP port.
EXPOSE 9080

# Health check: the sidecar exposes GET /health → 200 OK.
HEALTHCHECK \
    --interval=15s \
    --timeout=5s \
    --start-period=5s \
    --retries=3 \
    CMD wget -qO- http://localhost:9080/health || exit 1

# Drop privileges before exec.
USER sidecar:sidecar

# Use tini as init to forward signals correctly.
ENTRYPOINT ["/sbin/tini", "--"]
CMD ["/usr/local/bin/stellar-sidecar", "--bind", "0.0.0.0:9080"]

# =============================================================================
# Stage 3 — self-contained builder variant (no pre-built binary required)
#
# Uncomment this stage and change the COPY above to use --from=builder if you
# want a fully self-contained image that does not depend on a prior cargo build.
# Note: this adds ~1 GB of build cache and is not recommended for CI.
# =============================================================================
# FROM rust:1.82-alpine AS builder
# RUN apk add --no-cache musl-dev pkgconf openssl-dev binaryen
# WORKDIR /src
# COPY . .
# RUN cargo build --release --bin stellar-sidecar
#
# Then replace the COPY above with:
# COPY --from=builder /src/target/release/stellar-sidecar /usr/local/bin/stellar-sidecar
