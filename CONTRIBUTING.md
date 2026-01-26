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
   git clone https://github.com/stellar/stellar-k8s.git
   cd stellar-k8s
   ```
2. Run local checks:
   ```bash
   # Comprehensive pre-push check
   cargo fmt --all -- --check && \
   cargo clippy --all-targets --all-features -- -D warnings && \
   cargo test --all-features && \
   cargo test --doc && \
   cargo audit
   ```

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
2. Ensure all tests pass, including the 62+ unit tests for `StellarNodeSpec` validation.
3. Update the `CHANGELOG.md` (if applicable).
4. Submit your PR against the `develop` branch.

## Continuous Integration

Our CI pipeline (GitHub Actions) runs:
- **Lint & Format**: Checks code style and Clippy warnings.
- **Audit Dependencies**: Checks for known vulnerabilities in the dependency tree.
- **Test Suite**: Runs all unit and doc tests.
- **Build & Push**: Builds the Docker image and pushes to the registry.
- **Security Scan**: Runs Trivy on the built container image.
