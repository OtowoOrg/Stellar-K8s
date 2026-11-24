# Command Matrix Tests

## Overview

The command matrix test suite (`tests/command_matrix.rs`) verifies that all documented
Makefile pipeline targets compile and run successfully. Each test invokes a real
`make <target>` command and asserts a zero exit code.

These are **slow integration tests** — they are `#[ignore]` by default so they do
not run during normal `cargo test`. Run them explicitly when you need to validate
the full pipeline.

## Test Targets

| Test function             | Makefile target              | What it verifies                                        |
| ------------------------- | ---------------------------- | ------------------------------------------------------- |
| `command_fmt_check`       | `fmt-check`                  | `cargo fmt --all --check` passes                        |
| `command_lint`            | `lint`                       | Clippy passes with `K8S_OPENAPI_ENABLED_VERSION=1.30`  |
| `command_test`            | `test`                       | Unit, integration, and doc tests pass                   |
| `command_build`           | `build`                      | Release build completes and binary exists               |
| `command_quick`           | `quick`                      | Format check + `cargo check` pass                       |
| `command_shellcheck`      | `shellcheck`                 | All shell scripts pass shellcheck                       |
| `command_helm_lint`       | `helm-lint`                  | Helm charts pass lint and template rendering            |
| `command_check_api_docs`  | `check-api-docs`             | API reference docs are up to date                       |
| `command_completions`     | `completions`                | Shell completion scripts are generated for bash/zsh/fish |
| `command_link_check`      | `link-check`                 | Markdown internal links are valid                       |
| `command_check_third_party_licenses` | `check-third-party-licenses` | THIRD_PARTY_LICENSES.md is current               |

## Running

```bash
# Run all command matrix tests
cargo test command_matrix -- --ignored

# Run a single target (e.g. fmt-check only)
cargo test command_matrix::command_fmt_check -- --ignored

# Run with output visible
cargo test command_matrix -- --ignored --nocapture
```

## Requirements

The tests shell out to `make`, so the following must be available on `$PATH`:

- `make`
- `cargo` / `rustup` (for Rust targets)
- `helm` (for `helm-lint`)
- `python3` and the project's Python scripts (for `check-api-docs`, `link-check`)
- `shellcheck` (for `shellcheck`)

No Kubernetes cluster or Docker daemon is required — all tested targets run
entirely locally.

## When to Run

- **Pre-release gate**: Run the full matrix before cutting a release tag.
- **CI failure triage**: If CI fails on a specific target, re-run just that test
  locally to reproduce.
- **After large refactors**: Verify nothing in the pipeline has broken.
