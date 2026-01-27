# CI Commands Reference

This document lists the exact commands that run in CI. You can run these locally before pushing to ensure your changes will pass all checks.

## Quick Check (Pre-Commit)

**Purpose**: Fast validation before committing (~30 seconds)

```bash
# Check if code is properly formatted
cargo fmt --all --check

# Compile check (faster than full build)
cargo check --workspace
```

**What it does**: Verifies your code is formatted and compiles without errors.

---

## Full CI Pipeline

These are the exact steps GitHub Actions runs. Run them locally to catch issues before pushing.

### 1. Security Audit

**Purpose**: Detect security vulnerabilities in dependencies

```bash
# Install cargo-audit if not already installed
cargo install --locked cargo-audit

# Check for unsound code (memory safety issues)
cargo audit --deny unsound
```

**What it checks**:

- Memory safety vulnerabilities (RUSTSEC advisories)
- Blocks unsound code that could cause crashes or security issues
- Warns about unmaintained dependencies (non-blocking)

---

### 2. Format Check

**Purpose**: Enforce consistent code style

```bash
# Verify all code follows Rust formatting standards
cargo fmt --all --check
```

**What it checks**: Code formatting according to rustfmt rules. If this fails, run `cargo fmt --all` to auto-fix.

---

### 3. Lint (Clippy)

**Purpose**: Catch common mistakes and enforce best practices

```bash
# Run Clippy with zero-warnings policy
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

**What it checks**:

- Code quality issues
- Common mistakes and anti-patterns
- Performance improvements
- Best practice violations

**Note**: `-D warnings` means any warning = CI failure (zero-warning policy)

---

### 4. Test Suite

**Purpose**: Verify all functionality works correctly

```bash
# Run all unit tests, integration tests, and binary tests
cargo test --workspace --all-features --verbose

# Run documentation tests
cargo test --doc --workspace
```

**What it runs**:
- **62+ total tests** across the entire workspace
  - 52 `StellarNodeSpec` validation tests (CRD validation)
  - 5 kubectl plugin tests (table/JSON/YAML formatting)
  - Controller logic tests
  - Webhook tests
  - Documentation example tests

**Note**: CI runs both command sets. Locally, you can skip doc tests if examples are outdated.

---

### 5. Build Release

**Purpose**: Verify production build succeeds

```bash
# Build optimized release binary with locked dependencies
cargo build --release --locked
```

**What it does**:

- Builds with optimizations enabled
- Uses exact dependency versions from Cargo.lock (--locked)
- Produces production-ready binary in `target/release/stellar-operator`

---

### 6. Docker Build

**Purpose**: Verify container image builds successfully

```bash
# Build Docker image for local testing
docker build -t stellar-operator:local .
```

**What it does**:

- Multi-stage build using latest stable Rust
- Creates minimal distroless runtime image
- Supports multi-arch (amd64/arm64) in CI

---

## Using Make Commands

**Easier alternative**: Use the Makefile targets instead of running commands manually.

```bash
# One-time setup (installs Rust, components, tools)
make dev-setup

# Fast pre-commit check (~30 seconds)
make quick

# Full CI validation locally (~2-3 minutes)
make ci-local

# Show all available commands
make help
```

### What Each Make Command Does:

**`make dev-setup`**: Sets up development environment

- Updates Rust to latest stable
- Installs clippy and rustfmt
- Installs cargo-audit and cargo-watch

**`make quick`**: Fast pre-commit check

1. Checks code formatting
2. Runs compile check

**`make ci-local`**: Full CI pipeline (runs all commands above)

1. Checks formatting (`cargo fmt --all --check`)
2. Runs clippy with zero warnings (`cargo clippy --workspace --all-targets --all-features -- -D warnings`)
3. Security audit (`cargo audit --deny unsound`)
4. Runs all 62+ tests (`cargo test --workspace --all-features --verbose`)
5. Builds release binary (`cargo build --release --locked`)

**Tip**: See sections 1-6 above for what each step does and how to troubleshoot failures.

---

## CI Workflows

GitHub Actions automatically runs these workflows:

- **[ci.yml](workflows/ci.yml)**: Main pipeline
  - Runs on every push to `main` and all PRs
  - Security audit → Lint → Test → Build → Docker → Security Scan
- **[benchmark.yml](workflows/benchmark.yml)**: Performance tests
  - Runs on push to `main` and PRs
  - Measures operator performance with k6
- **[release.yml](workflows/release.yml)**: Release automation
  - Runs on version tags (v*.*.\*)
  - Builds multi-platform binaries and Docker images
  - Creates GitHub release with artifacts

---

## Troubleshooting

### Format Check Fails

```bash
# Auto-fix formatting
cargo fmt --all
```

### Clippy Warnings

```bash
# See detailed clippy suggestions
cargo clippy --workspace --all-targets --all-features
```

### Test Failures

```bash
# Run tests with detailed output
cargo test --workspace --verbose -- --nocapture
```

### Build Failures

```bash
# Clean and rebuild
cargo clean
cargo build --release --locked
```
