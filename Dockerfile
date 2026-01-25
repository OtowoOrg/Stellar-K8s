# ==============================================================================
# Stage 1: Chef - Dependency Caching Layer
# ==============================================================================
FROM lukemathwalker/cargo-chef:latest-rust-1.93 AS chef
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

# Copy the recipe and build dependencies first (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Now copy source and build the application
COPY . .
RUN cargo build --release --bin stellar-operator

# Strip the binary to reduce size
RUN strip /app/target/release/stellar-operator

# ==============================================================================
# Stage 4: Runtime - Minimal distroless image (~15-20MB total)
# ==============================================================================
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/stellar/stellar-k8s"
LABEL org.opencontainers.image.description="Stellar-K8s Kubernetes Operator"
LABEL org.opencontainers.image.licenses="Apache-2.0"

# Copy the stripped binary
COPY --from=builder /app/target/release/stellar-operator /stellar-operator

# Run as non-root user (UID 65532 is the nonroot user in distroless)
USER nonroot:nonroot

# Expose metrics and REST API ports
EXPOSE 8080 9090

# Health check endpoint
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD ["/stellar-operator", "--health-check"] || exit 1

ENTRYPOINT ["/stellar-operator"]
