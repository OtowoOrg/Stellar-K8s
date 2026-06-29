# Reproducible Docker Build Guide

This guide explains how to build deterministic Docker images for **Stellar‑K8s**. The steps are incorporated directly into the repository so CI and local developers can reproduce identical image hashes.

## Why reproducibility matters
- Guarantees that the same source code produces the same image digests, aiding supply‑chain security.
- Enables reliable caching in CI pipelines and easier verification of builds.
- Facilitates deterministic scanning for vulnerabilities.

## What we changed
1. **Pinned base images**
   - `lukemathwalker/cargo-chef:1.95-bookworm` (previously `latest‑rust‑1.95‑bookworm`).
   - `debian:bookworm-slim` is now referenced by its SHA256 digest.
2. **Source‑date‑epoch**
   - Added `ARG SOURCE_DATE_EPOCH=0` and `ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}` early in the Dockerfile. This forces the same timestamps for file metadata inside the image.
3. **Removed non‑deterministic steps**
   - Consolidated `apt‑get` clean‑up and avoided installing optional packages that change `apt` timestamps.
   - All `COPY` commands are deterministic because files are sorted by Git.

## Building the image locally
```bash
# Clean any previous images
docker rmi stellar-k8s:dev

# Build with reproducibility flag (SOURCE_DATE_EPOCH defaults to 0)
DOCKER_BUILDKIT=1 docker build \
  --build-arg SOURCE_DATE_EPOCH=$(date +%s) \
  -t stellar-k8s:dev -f Dockerfile .
```
The command uses BuildKit which respects the `SOURCE_DATE_EPOCH` argument.

## Verifying reproducibility
Run the build **twice** with the same arguments and compare digests:
```bash
docker image inspect --format='{{.RepoDigests}}' stellar-k8s:dev
```
Both runs should output the **same** digest (e.g. `stellar-k8s@sha256:abcd1234…`).

## CI integration
The GitHub workflow `docker-build.yml` now injects `SOURCE_DATE_EPOCH=$(date +%s)` automatically. The workflow also runs the verification step and fails if the digests differ.

---
For any questions or to suggest additional deterministic steps, please open an issue.
