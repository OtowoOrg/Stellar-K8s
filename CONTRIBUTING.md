# Contributing to Stellar-K8s

Thank you for contributing to Stellar-K8s! This guide explains how to work with the project, keep your pull requests ready for review, and follow our commit and merge conventions.

## Troubleshooting Quick Links

If you run into issues, jump to the relevant section below:
- [Setup Issues](#setup-issues)
- [Build Failures](#build-failures)
- [Cargo Issues](#cargo-issues)
- [Docker Issues](#docker-issues)
- [Kubernetes Issues](#kubernetes-issues)
- [CI Failures](#ci-failures)

## 1. Fork and Pull Request Workflow

We use a fork-and-pull-request model. The basic flow is:

1. **Fork** the repository on GitHub.
2. **Clone** your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/stellar-k8s.git
   cd stellar-k8s
   ```
3. **Add the upstream remote**:
   ```bash
   git remote add upstream https://github.com/OtowoOrg/Stellar-K8s.git
   ```
4. **Sync from upstream** before creating a branch:
   ```bash
   git fetch upstream
   git checkout main
   git merge upstream/main
   ```
5. **Create a new branch** for your work.
6. **Make focused commits**.
7. **Run local checks** before pushing.
8. **Push your branch** to your fork.
9. **Open a Pull Request** against the upstream `main` branch.

## 2. Branch Naming and Strategy

Use clear, descriptive branch names. Recommended prefixes:

- `feat/` for new features (e.g. `feat/auto-mtls`)
- `fix/` for bug fixes (e.g. `fix/panic-on-startup`)
- `docs/` for documentation updates (e.g. `docs/update-architecture`)
- `chore/` for maintenance or dependency changes (e.g. `chore/bump-kube-rs`)
- `test/` for test-related work (e.g. `test/e2e-service-mesh`)

### Branching Rules

- Always branch from the latest `main`.
- Do not work directly on `main`.
- Keep each branch scoped to a single feature, bug fix, or documentation item.
- Rebase or merge `main` into your branch before opening a PR if `main` has advanced.

### Merge Strategy

We prefer a clean history. When your PR is approved, maintainers will typically merge it using:

- **Squash and merge** for feature and fix branches
- **Rebase and merge** only when preserving a linear history is important

If your PR contains multiple logical changes, split it into separate branches and PRs.

## 3. PR Checklist

Before opening a PR, confirm the following:

- [ ] The code or documentation change is complete and focused.
- [ ] The PR targets the `main` branch.
- [ ] Your branch is up to date with `main`.
- [ ] You have run tests locally.
- [ ] You have run formatting and lint checks.
- [ ] You have added or updated documentation, if needed.
- [ ] Commit messages are clear, accurate, and follow our conventions.
- [ ] Every commit includes a DCO sign-off.
- [ ] The PR description is filled out completely using the template.
- [ ] The PR includes links to any related issues or design discussions.

### Required checks

Run these locally before submitting:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
make ci-local
```

If your change adds shell scripts or repository tooling, also run:

```bash
find scripts -type f -name "*.sh" -print0 | xargs -0 shellcheck -S error
```

## 4. Commit Message Examples

We follow [Conventional Commits](https://www.conventionalcommits.org/).

Correct examples:

```text
feat(cli): add support for --dry-run mode
fix(webhook): handle nil admission review objects
docs(contributing): clarify PR checklist and branch strategy
test(integration): add end-to-end service mesh coverage
chore(deps): bump kube-rs to 0.1.0
```

When to use each type:

- `feat:` new functionality
- `fix:` bug fixes
- `docs:` documentation-only changes
- `chore:` maintenance tasks and dependency updates
- `refactor:` code changes that do not add features or fix bugs
- `test:` adding or updating tests

Example with body and footer:

```text
fix(metrics): avoid panic when metrics registry is empty

This change adds a guard around metric registration so operator startup
continues even if no collector is present.

Signed-off-by: Alice Doe <alice@example.com>
```

## 5. Developer Certificate of Origin (DCO)

All commits must include a `Signed-off-by` line.

Add this automatically with:

```bash
git commit -s -m "fix: your fix description"
```

The sign-off must match the commit author. Unsigned commits may fail CI and block merge.

## 6. Pull Request Template

A PR template is provided in `.github/PULL_REQUEST_TEMPLATE.md` and will populate the PR description when you open a PR.

Fill out every section fully. Do not leave the template blank or remove required checklist items.

The template ensures your change includes:

- tests and validation
- documentation updates when required
- formatting and linting checks
- DCO sign-off

## 7. Development Environment

### Prerequisites

- Rust stable (1.88+)
- Kubernetes local cluster (`kind`, `minikube`, etc.)
- Docker
- `cargo-audit`
- `pre-commit` hooks

### Setup

Use the project make targets and scripts:

```bash
make dev-setup
bash scripts/setup-mac.sh  # macOS only
```

### Local checks

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
make quick
make ci-local
```

## 8. Coding Standards

- Format Rust code with `cargo fmt`.
- Use `cargo clippy --all-targets --all-features -- -D warnings` for linting.
- Add or update tests for code changes.
- Document behavior changes in code comments and docs.
- Keep PRs small and easy to review.

### Rust code conventions

- Module names use `snake_case`.
- Public types and functions require doc comments (`///`).
- Do not add `#[allow(dead_code)]` without a comment explaining why the code must stay.
- Unused imports must be removed before merging.
- Feature-gated code that is no longer used should be deleted, not suppressed.

### Documentation conventions

- Documentation files use `kebab-case.md` (e.g., `disk-scaling.md`).
- Files that belong to a topic area go in the matching `docs/<topic>/` subdirectory.
- Root-level docs (`README.md`, `DEVELOPMENT.md`, `CONTRIBUTING.md`) are entry points only — detailed content belongs in `docs/`.
- New doc files must be linked from `docs/README.md` under the appropriate section.

### Script conventions

- Scripts use `kebab-case.sh` (e.g., `setup-mac.sh`).
- Every script must pass `shellcheck -S error`.
- Historical or one-off scripts should be moved to `scripts/archive/` rather than left in the root of `scripts/`.

### Manifest and config conventions

- CRD YAML files follow the `stellar{feature}-crd.yaml` naming pattern under `config/crd/`.
- Example manifests in `examples/` use descriptive, feature-based names — not issue numbers.
- Generated manifests (CRDs, API reference, bundle) must be regenerated from their source before merging. See the [Regenerating Manifests](DEVELOPMENT.md#regenerating-manifests) table in DEVELOPMENT.md.

## 9. Repo Health Checklist

Run through this before marking a PR ready for review:

- [ ] `make ci-local` passes (format + lint + audit + test + build)
- [ ] No new `#[allow(dead_code)]` without an explanatory comment
- [ ] No unused imports in modified files
- [ ] Generated manifests are up to date with their source
- [ ] Shell scripts pass `shellcheck -S error`
- [ ] New doc files are linked from `docs/README.md`
- [ ] Commit messages follow Conventional Commits and include a `Signed-off-by` line

## 10. Need Help?

If you're stuck, open a Draft PR or create an issue to ask for guidance.

Refer to [README.md](README.md) and [DEVELOPMENT.md](DEVELOPMENT.md) for additional project setup and workflow information.

## Troubleshooting

### Setup Issues
- **Problem**: `make` or `cargo` commands not found.
  - **Solution**: Ensure you have installed the necessary dependencies from `DEVELOPMENT.md`.
- **Problem**: Minikube / Kind cluster fails to start.
  - **Solution**: Check your Docker daemon is running and has enough resources allocated (minimum 4GB RAM, 2 CPUs).

### Build Failures
- **Problem**: Code fails to compile due to missing dependencies.
  - **Solution**: Run `cargo fetch` or `cargo update` to ensure you have the latest crates. Also, ensure your system has `cmake`, `libssl-dev`, and `pkg-config` installed.
- **Problem**: Tests fail locally but pass on CI.
  - **Solution**: Run `make clean` and then rebuild. Sometimes local artifacts can get stale.

### Cargo Issues
- **Problem**: Cargo build is extremely slow.
  - **Solution**: We highly recommend using `sccache` to cache intermediate build results. Follow the instructions in `DEVELOPMENT.md` to set it up.

### Docker Issues
- **Problem**: Docker build fails with out of space errors.
  - **Solution**: Run `docker system prune` to free up space. The build requires at least 10GB of free space due to the multi-stage cargo caching.
- **Problem**: `make quick` fails during docker validation.
  - **Solution**: Make sure you have the latest base images pulled locally.

### Kubernetes Issues
- **Problem**: Operator pod is crashlooping.
  - **Solution**: Check the operator logs using `kubectl logs -n stellar-system -l app.kubernetes.io/name=stellar-operator`. Often, this is due to invalid RBAC permissions or missing secrets.
- **Problem**: Custom Resource Definitions (CRDs) not applying.
  - **Solution**: Ensure your KUBECONFIG points to the correct cluster. Run `make install` to manually install the CRDs into your cluster.

### CI Failures
- **Problem**: GitHub Actions workflow fails on linting.
  - **Solution**: Run `cargo fmt` and `cargo clippy` locally before pushing. Also, check `.pre-commit-config.yaml` to ensure your pre-commit hooks are installed.
- **Problem**: Link validation CI fails.
  - **Solution**: Make sure all Markdown links are valid and relative paths point to existing files.
