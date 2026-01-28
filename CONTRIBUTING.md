# Contributing to Stellar-K8s

Thank you for your interest in contributing to Stellar-K8s! This project aims to provide a robust, cloud-native Kubernetes operator for managing Stellar infrastructure.

## Development Environment

### Prerequisites

- **Rust**: Latest stable version (1.75+)
- **Kubernetes**: A local cluster like `kind` or `minikube`
- **Docker**: For building container images
- **Cargo-audit**: For security scans (`cargo install cargo-audit`)

### Setup

1. Clone the repository:
   ```bash
   git clone https://github.com/OtowoOrg/Stellar-K8s.git
   cd Stellar-K8s
   ```
2. Setup development environment:

   ```bash
   make dev-setup
   ```

3. Run local checks before committing:

   ```bash
   # Quick check
   make quick

   # Or comprehensive pre-push check
   make ci-local
   ```

   **See [CI Commands Reference](.github/CI_COMMANDS.md) for the exact commands that run in CI, which you can run manually.**

## Coding Standards

- **Formatting**: Always run `cargo fmt` before committing.
- **Linting**: We use Clippy for linting. Ensure `cargo clippy --all-targets --all-features -- -D warnings` passes. We follow a "zero-warning" policy for pushes to `main`.
- **Security**: All dependencies must be audited. We resolve all `RUSTSEC` advisories immediately.
- **Error Handling**: Use `thiserror` for library errors and `anyhow` for application-level logic. Prefer the `Result<T>` type defined in `src/error.rs`.

## Security Policy

We take security seriously. If you find a vulnerability (e.g., in a dependency or the code), please do not open a public issue. Instead, follow the security reporting process described in [SECURITY.md](SECURITY.md) (if available) or contact the maintainers directly.

### Mitigating RUSTSEC Advisories

If a dependency scan fails due to a RUSTSEC advisory:

1. Identify the crate and version causing the issue.
2. Upgrade the dependency in `Cargo.toml`.
3. If the vulnerability is in an internal transitive dependency, use `cargo tree -i <vulnerable-crate>` to find the source and upgrade the parent.

## Pull Request Process

1. Create a new branch for your feature or fix.
2. Ensure all tests pass locally: `make ci-local` or use your personalized commands for your tests and changes. 
3. Ensure all 62+ unit tests pass, including the `StellarNodeSpec` validation tests.
4. Submit your PR against the `main` branch.
5. Wait for CI checks to pass (all workflows must be green ✓).
6. Fix conflicts if any and ensure your tests pass once you fix conflicts.

## Continuous Integration

Our CI pipeline (GitHub Actions) runs:

- **Security Audit**: Checks for known vulnerabilities (blocks unsound code).
- **Lint & Format**: Checks code style and Clippy warnings.
- **Test Suite**: Runs all unit tests.
- **Build**: Creates release binary.
- **Docker Build**: Multi-arch images (amd64/arm64).
- **Security Scan**: Runs Trivy on the container image.

**View the exact commands in [CI Commands Reference](.github/CI_COMMANDS.md).**

## Local Development Commands

```bash
make help          # Show all available targets
make fmt           # Auto-format code
make lint          # Run clippy
make audit         # Security audit
make test          # Run tests
make build         # Build release binary
make docker-build  # Build Docker image
make ci-local      # Full CI validation
```

## Troubleshooting

### Build Failures

```bash
cargo clean
make build
```

### Test Failures

```bash
cargo test --workspace --verbose -- --nocapture
```

### Dependency Issues

```bash
cargo update
cargo tree -i <crate-name>  # Find what depends on a crate
```
