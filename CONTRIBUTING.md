# Contributing to Stellar-K8s

First off, thank you for considering contributing to Stellar-K8s! This project aims to provide a robust, cloud-native Kubernetes operator for managing Stellar infrastructure.

This document provides a clear guide on how to contribute to the project, covering everything from our developer workflow to commit structures.

## 1. Fork Workflow

We use a standard fork and pull request workflow for contributions:

1. **Fork the repository** on GitHub.
2. **Clone your fork** locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/stellar-k8s.git
   cd stellar-k8s
   ```
3. **Add the upstream remote** so you can keep your fork synced:
   ```bash
   git remote add upstream https://github.com/stellar/stellar-k8s.git
   ```
4. **Create a new branch** for your feature or bugfix (see *Branch Naming* below).
5. **Commit your changes**, keeping them focused and atomic.
6. **Push to your fork** on GitHub.
7. **Open a Pull Request** against the `main` branch of the upstream repository.

   This will:
   - Install Rust toolchain and components
   - Install cargo-audit and cargo-watch
   - Install pre-commit hooks for automatic code quality checks

3. Run local checks before committing:
## 2. Branch Naming

Please use descriptive branch names based on the nature of your contribution. We recommend the following prefixes:

   # Run pre-commit hooks manually
   make pre-commit

   # Or comprehensive pre-push check
   make ci-local
   ```
- `feat/` for new features (e.g., `feat/auto-mtls`)
- `fix/` for bug fixes (e.g., `fix/panic-on-startup`)
- `docs/` for documentation updates (e.g., `docs/update-architecture`)
- `chore/` for maintenance tasks, refactoring, or dependency updates (e.g., `chore/bump-kube-rs`)
- `test/` for adding or improving tests (e.g., `test/e2e-service-mesh`)

## 3. Commit Conventions

We strictly follow [Conventional Commits](https://www.conventionalcommits.org/). This allows us to automate our changelog generation and semantic versioning.

Your commit messages should be formatted as follows:
```
<type>(<optional scope>): <description>

[optional body]

[optional footer(s)]
```

**Common types include:**
- `feat:` A new feature
- `fix:` A bug fix
- `docs:` Documentation only changes
- `chore:` Changes to the build process or auxiliary tools and libraries
- `refactor:` A code change that neither fixes a bug nor adds a feature
- `test:` Adding missing tests or correcting existing tests

## 4. Developer Certificate of Origin (DCO) Sign-off

To comply with open-source legal standards, **all commits must include a `Signed-off-by` line indicating that you agree to the [Developer Certificate of Origin (DCO)](https://developercertificate.org/)**.

You can automatically add this sign-off to your commits by using the `-s` or `--signoff` flag:
```bash
git commit -s -m "feat: your feature description"
```

This will append the following line to your commit message:
`Signed-off-by: Jane Doe <jane.doe@example.com>`

**Note:** The name and email used in the sign-off must match the author of the commit. PRs with unsigned commits will fail our CI pipeline checks.

## 5. Pull Request Template Usage

When you open a Pull Request, a template will automatically populate the description box. **You must fill out this template completely.**

The PR template includes a checklist to ensure your code:
- Passes all CI tests (`cargo test`)
- Is properly formatted (`cargo fmt`)
- Passes linting (`cargo clippy`)
- Includes a DCO sign-off

Please do not delete the template sections. PRs with empty descriptions or unchecked vital requirements will be heavily delayed or closed.

## 6. Development Environment

### Prerequisites

- **Rust**: Latest stable version (1.88+)
- **Kubernetes**: A local cluster like `kind` or `minikube`
- **Docker**: For building container images
- **Cargo-audit**: For security scans (`cargo install cargo-audit`)

### macOS Setup

For macOS users, we provide an automated setup script that installs all necessary dependencies:

```bash
bash scripts/setup-mac.sh
```

This script will:
- Install Homebrew (if not present)
- Install Rust and update to latest stable
- Install Docker, kubectl, kind, Helm, and GitHub CLI
- Install cargo-audit and pre-commit
- Verify all installations

After running the script, follow the manual steps printed at the end (Docker Desktop startup, GitHub CLI authentication, etc.).

### Setup & Local Checks

1. Setup development environment:
   ```bash
   make dev-setup
   ```
2. Run local checks before committing:
   ```bash
   make quick     # Use this for a fast compilation check
   make ci-local  # Comprehensive pre-push check mimicking CI
   ```

### Coding Standards

- **Formatting**: Always run `cargo fmt` before committing.
- **Linting**: We use Clippy. Ensure `cargo clippy --all-targets --all-features -- -D warnings` passes.
- **Shell scripts**: We lint scripts under `scripts/` with ShellCheck. Run `find scripts -type f -name "*.sh" -print0 | xargs -0 shellcheck -S error` locally.
- **Security**: All dependencies must be audited. We resolve all `RUSTSEC` advisories immediately.
- **Error Handling**: Prefer the `Result<T>` type defined in `src/error.rs` using `thiserror`.

## Need Help?
If you're stuck, feel free to open a Draft PR or reach out in the repository's discussions/issues!
