# Contributing to Stellar-K8s

Thank you for contributing to Stellar-K8s! This guide explains how to work with the project, keep your pull requests ready for review, and follow our commit and merge conventions.

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

## 9. Need Help?

If you're stuck, open a Draft PR or create an issue to ask for guidance.

Refer to [README.md](README.md) and [DEVELOPMENT.md](DEVELOPMENT.md) for additional project setup and workflow information.
