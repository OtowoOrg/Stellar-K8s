# Build Targets

> **Issue #1781** — Reference for the Makefile targets that build the operator binaries, Wasm artifacts, shell completions, and container images.

## Overview

All build entry points live in the repository `Makefile`. They wrap plain `cargo`, `docker`, and `helm` commands so local builds and CI use the same flags (notably `--locked`, which keeps `Cargo.lock` authoritative). Run `make help` to list every target with its one-line description.

## Rust builds

| Target | Command it runs | Use it when |
|--------|-----------------|-------------|
| `make build` | `cargo build --release --locked` | You need the optimised release binaries in `target/release/`. |
| `make build-debug` | `cargo build --locked` | You want a debug binary for fast iteration. |
| `make build-check` | `cargo check --locked --all-targets` | You only need a type check; no binaries are produced. |
| `make build-all` | `build` + `docs-build` | You want the release binary and the documentation site together. |
| `make cache-warm` | `cargo fetch --locked` + `cargo build --locked --lib --bins` | You want to warm the dependency cache before a long build or CI run. |
| `make run-local` | `build`, then runs the release operator binary | You want to run the freshly built operator locally. |
| `make clean` | `cargo clean` | You want to remove Cargo build artifacts. |

The fastest pre-commit loop is `make quick`, which combines the formatting check with a compile check. `make build-check` is the fastest way to validate that every target (lib, bins, tests, benches) still compiles.

## Wasm artifacts

| Target | What it builds |
|--------|----------------|
| `make wasm-cache-build` | Compiles the bounded Soroban cache Wasm artifact (`stellar-wasm-cache`) for `wasm32-unknown-unknown` and fails if it exceeds 2 MiB. |

## Shell completions

| Target | Output |
|--------|--------|
| `make completions` | Generates Bash, Zsh, and Fish completion scripts into `./completions/`. |

## Container images

| Target | What it builds |
|--------|----------------|
| `make docker-build` | Fast local image from the host release binaries (builds them with `make build` first if missing). |
| `make docker-build-ci` | Reproducible CI image that compiles the binaries inside the container. |
| `make docker-multiarch` | Multi-arch image (`linux/amd64` + `linux/arm64`) via `docker buildx`. |

`docker-build` is the quickest local loop because it reuses host binaries; `docker-build-ci` is the authoritative image used by CI.

## Typical flows

```bash
# Fast compile validation
make build-check

# Local release binary
make build

# Release binary plus rendered documentation
make build-all

# Container image for local testing
make docker-build
```

## Cross-References

- Makefile: [`Makefile`](../../Makefile)
- Command matrix tests: [`command-matrix.md`](command-matrix.md)
- Documentation build: [`docs-build.md`](docs-build.md)
- Development workflow: [`DEVELOPMENT.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/DEVELOPMENT.md)
