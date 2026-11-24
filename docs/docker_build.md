# Reproducible Docker Build Guide

This guide explains how to build deterministic Docker images for **Stellar-K8s**. The steps are incorporated directly into the repository so CI and local developers can reproduce identical image hashes.

## Why reproducibility matters
- Guarantees that the same source code produces the same image digests, aiding supply-chain security.
- Enables reliable caching in CI pipelines and easier verification of builds.
- Facilitates deterministic scanning for vulnerabilities.

## What we changed
1. **Pinned base images**
   - `lukemathwalker/cargo-chef:latest-rust-1.95-slim-bookworm` (tag `1.95-bookworm` is unpublished on Docker Hub).
   - `debian:bookworm-slim` is now referenced by its SHA256 digest.
2. **Source-date-epoch**
   - Added `ARG SOURCE_DATE_EPOCH=0` and `ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}` early in the Dockerfile. This forces the same timestamps for file metadata inside the image.
3. **Removed non-deterministic steps**
   - Consolidated `apt-get` clean-up and avoided installing optional packages that change `apt` timestamps.
   - All `COPY` commands are deterministic because files are sorted by Git.

## Building the image locally

```bash
# Fast local build (uses host release binaries)
make docker-build

# Reproducible CI build (compiles inside the container)
make docker-build-ci
```

Both targets use BuildKit and respect `SOURCE_DATE_EPOCH` when set:

```bash
DOCKER_BUILDKIT=1 docker build \
  --build-arg SOURCE_DATE_EPOCH=$(date +%s) \
  --target runtime-local \
  -t stellar-operator:latest .
```

## Purging local build cache

Run `make docker-cache-purge` to remove all unused build cache on the current
Docker daemon. Docker asks for confirmation before pruning and reports the
reclaimed disk space. This is daemon-wide, so other projects using that daemon
may also need to rebuild their cached layers. The next build may be slower.

The target uses `docker builder prune --all`; it does not prune application
images, containers, or volumes. It is an explicit maintenance command and is
not part of `make clean`, build targets, or CI. Remote registry and GitHub Actions
caches, and separate Buildx builders, are outside its scope.

To verify manually, inspect `docker system df` before and after running the
target, confirm the prune prompt, and check Docker's reclaimed-space report.
Reclaiming zero bytes is valid when there is no unused build cache.
See [Docker's builder prune reference](https://docs.docker.com/reference/cli/docker/builder/prune/)
for details.

## Pruning dangling local images

Run `make docker-image-prune` to remove dangling (untagged) images from the
current Docker daemon. Docker asks for confirmation before pruning. Tagged
images, containers, volumes, and remote registry images are not touched. The
target runs `docker image prune` and is not part of `make clean`, build targets,
or CI.

See [Docker's image prune reference](https://docs.docker.com/reference/cli/docker/image/prune/)
for details.

## Verifying reproducibility
Run the build **twice** with the same arguments and compare digests:
```bash
docker image inspect --format='{{.RepoDigests}}' stellar-operator:latest
```
Both runs should output the **same** digest (e.g. `stellar-operator@sha256:abcd1234…`).

## CI integration
Multi-arch images are built and published by the GitHub workflow [`release.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/release.yml) (multi-arch publish job). Release re-tagging is handled by [`.github/workflows/release.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/release.yml).
Multi-arch images are built and published by the CI multi-arch workflow (see [CI commands](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/CI_COMMANDS.md)). Release re-tagging is handled by [`.github/workflows/release.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/release.yml).

Local kind/e2e tests should prefer `make docker-build` or the shared [setup-kind-cluster](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/actions/setup-kind-cluster/action.yml) composite action instead of ad-hoc `docker build` scripts.

---
For any questions or to suggest additional deterministic steps, please open an issue.
