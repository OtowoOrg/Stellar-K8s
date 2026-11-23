# Contributing to Stellar-K8s

Thank you for contributing to Stellar-K8s! This guide explains how to work with the project, keep your pull requests ready for review, and follow our commit and merge conventions.

## Quick Start

New to the project? Start here:

1. [Development Environment](#7-development-environment) — install prerequisites and run `make dev-setup`.
2. [Local checks](#local-checks--canonical-workflow) — run `make health` and `make ci-local` before pushing.
3. [Commit Message Examples](#4-commit-message-examples) and [DCO](#5-developer-certificate-of-origin-dco) — format commits and sign off.
4. [PR Checklist](#3-pr-checklist) — confirm your pull request is ready for review.

For a narrative overview of the project's direction and design storyboards, see
the [PITCH storyboards](docs/pitch-storyboards.md).

## Troubleshooting Quick Links

If you run into issues, jump to the relevant section below:
- [Setup Issues](#setup-issues)
- [Build Failures](#build-failures)
- [Cargo Issues](#cargo-issues)
- [Docker Issues](#docker-issues)
- [Kubernetes Issues](#kubernetes-issues)
- [CI Failures](#ci-failures)

## 1. Fork and Pull Request Workflow

Maintainers can follow the [issue label review guide](docs/development/issue-label-review.md)
to check template labels and triage issues.

Reviewer routing lives in [CODEOWNERS](.github/CODEOWNERS). Keep the default
rule first and specific overrides last: GitHub uses the last matching rule,
not a union of matching owners. Remove shadowed duplicate patterns when trimming
the file, while preserving the effective owners. In particular, `/src/webhook/`
routes to `@security-team`, and the performance workflow overrides the general
workflow owners. Verify edits by comparing effective owners for those paths,
ordinary controller files, and an unmatched path before and after the change.
Maintainers should also check GitHub's CODEOWNERS view for syntax or access errors.

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

### Branch Protection

Changes to `main` must go through a pull request. The `main` branch protection rule should require:

- A pull request before merging.
- Passing status checks from the primary workflow [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (`CI/CD Pipeline`). Common jobs include **Lint & Format** (`lint`), **Test** (`test`), and **Security Audit** (`security-audit`, skipped when dependency files are unchanged). Do not treat a job named `build` as required; that job is not defined in `ci.yml`.

See [Dependency Update Process](docs/dependency-update-process.md#branch-protection-prerequisites) for the full list of settings that auto-merge depends on.

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

### Code owners

Reviewers are assigned automatically from [`.github/CODEOWNERS`](.github/CODEOWNERS)
(last matching rule wins). Paths such as `src/webhook/`, `charts/`, and `docs/` route
to specific teams, so expect those reviews when you touch them. Keep the file in sync
when paths move or teams change.

### Required checks

Before submitting, run the contributor health gates via `make` (not raw
`cargo` — Make targets set the workspace feature flags so results match CI):

```bash
make health        # Format + lint + tests + docs
make ci-local      # Full local CI gate (includes audit + issue-template-lint + test-bats + link-check)
```

The full checklist, command rationale, and per-step details live in the
[Canonical Repository Health Checklist](docs/development/repo-health-checklist.md).
If your change adds shell scripts, also run `make shellcheck`.

Spell checking runs in the `Codespell` CI job; run the same check locally with:
```bash
pip install codespell
make codespell
```
Add genuine false positives (one lowercase word per line) to `.codespellignore`.

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

- Rust 1.92+ (CI-enforced minimum — see `scripts/lib/versions.sh`)
- Kubernetes local cluster (`kind`, `minikube`, etc.)
- Docker
- `cargo-audit`
- `pre-commit` hooks

### Setup

Docker, kind, kubectl, Helm, and `gh` must currently be installed manually
for your OS (automated installers for these are tracked separately — see
[DEVELOPMENT.md § Prerequisites](DEVELOPMENT.md#prerequisites) for the
per-tool install links). Then run:

```bash
make dev-setup
```

`make dev-setup` installs the Rust toolchain/components, `cargo-audit` and
`cargo-watch`, and the `pre-commit` hooks, then runs
`stellar-bootstrap-verify` as a final step and prints a pass/fail report of
every required tool and version pin — see
[DEVELOPMENT.md § Troubleshooting](DEVELOPMENT.md#missing-or-outdated-tools)
if it reports anything missing or outdated.

### Local checks — Canonical Workflow

Always drive the local pipeline through `make` targets so results match CI:

```bash
make health        # Contributor health gate
make ci-local      # Full CI pipeline (fmt-check + lint + docs-lint + audit + test + build + issue-template-lint + test-bats + link-check)
```

Individual targets, when you want to iterate on one step at a time:

```bash
make fmt           # Format Rust code
make lint          # Clippy with the project's feature flags
make test          # Run the test suite
make docs-lint     # Validate documentation and links
```

See the [Canonical Repository Health Checklist](docs/development/repo-health-checklist.md)
for the full command set and per-step expectations.

### Script tests — bats harness

Shell scripts under `scripts/` are covered by a [bats](https://github.com/bats-core/bats-core)
test harness in `scripts/tests/`. CI runs these suites on every PR that touches
`scripts/`, so add or extend a suite whenever you change a script.

**Prerequisites** — install bats (and its helper libraries) locally:

```bash
# macOS
brew install bats-core

# Debian/Ubuntu
sudo apt-get install -y bats

# Any platform via npm
npm install -g bats
```

**Run the suites** — the same target the `Bats Suites` CI job uses (every suite
except `helm-drift.bats`, which needs `helm` and runs via `make test-helm-drift`):

```bash
make test-bats
```

To run a single suite while iterating:

```bash
bats scripts/tests/preflight.bats
```

**Adding a new suite** — create `scripts/tests/<script-name>.bats` next to the
script it exercises, then:

1. Start the file with `#!/usr/bin/env bats` and load shared helpers with
   `load 'test_helper'` if the suite needs common fixtures.
2. Add one `@test "<description>"` block per behavior you want to lock in.
3. Use `run <command>` and assert on `$status` / `$output` so failures are
   reported per test case.
4. Keep each suite self-contained: create any temp files under `$BATS_TEST_TMPDIR`
   and clean up after the test.
5. Verify locally with `bats scripts/tests/<script-name>.bats` before pushing.

**Reference suite** — [`scripts/tests/preflight.bats`](scripts/tests/preflight.bats)
is the canonical example: it shows the expected file layout, helper loading, and
assertion style to follow when adding new suites.

### Operational scripts index

The layout, naming, and required-header rules for `scripts/` are summarized in
[`docs/contributing/scripts-conventions.md`](docs/contributing/scripts-conventions.md).

[`scripts/README.md`](scripts/README.md) indexes every operational script in the
repository: what it does, the canonical invocation (and the `make` target that
wraps it, where one exists), and the suite in `scripts/tests/` that covers it. It
also separates the CI-only helpers in `scripts/ci/` from the scripts you are
expected to run locally.

To lint the GitHub issue templates locally — the same check CI runs when
`.github/ISSUE_TEMPLATE/` or the linter changes:

```bash
make issue-template-lint
```

The command exits non-zero and lists every offending template when an Issue Form
is missing required keys (`name`, `description`, `body`), uses an unsupported
`body` field type, or has a malformed `config.yml`. It is also part of
`make health`.

## How CI validates contributions

Every pull request against `main` runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml). Path filters skip some jobs when the corresponding files did not change. Before you open a PR, run the matching local targets:

| Local command | What it covers (Makefile) |
| ------------- | ------------------------- |
| `make quick` | `fmt-check` |
| `make fmt` / `make fmt-check` | rustfmt |
| `make lint` | clippy with workspace feature flags |
| `make test` | cargo test suite |
| `make codespell` | same spell-check as the CI `codespell` job |
| `make shellcheck` | shell scripts |
| `make test-bats` | bats suites under `scripts/tests/` (except helm-drift) |
| `make health` | format, lint, tests, and docs gates |
| `make ci-local` | `fmt-check` + `lint` + `audit` + `test` + `build` + `issue-template-lint` + `test-bats` |
| `make nav-coverage` | every `mkdocs.yml` nav path exists |
| `make docs-ci-build` | `mkdocs build --strict` |

Confirm a target exists with `make help` before documenting a new command.

## Reporting defects and documentation-only changes

- Use the [issue templates](https://github.com/OtowoOrg/Stellar-K8s/issues/new/choose) for bugs, features, epics, and support requests.
- Security reports follow [SECURITY.md](SECURITY.md).
- Documentation PRs still need passing CI. If you change `docs/**` or `mkdocs.yml`, run `make nav-coverage` and `make docs-ci-build`, and add the page to `mkdocs.yml` when you add a new doc.
- Keep documentation PRs focused; do not mix unrelated code changes.

## 8. Coding Standards

- Format Rust code with `make fmt`.
- Lint with `make lint` (clippy with the project's feature flags).
- Run tests with `make test`.
- Document behavior changes in code comments and docs.
- Keep PRs small and easy to review.

### Rust code conventions

- Module names use `snake_case`.
- Public types and functions require doc comments (`///`).
- Do not add `#[allow(dead_code)]` without a comment explaining why the code must stay.
- Unused imports must be removed before merging.
- Feature-gated code that is no longer used should be deleted, not suppressed.

### Container environment variable conventions

- **Seed injection deduplication**: A `StellarNode` may configure its validator seed via the legacy `spec.validatorConfig.seedSecretRef` (a plain Kubernetes Secret reference) or the newer `spec.validatorConfig.seedSecretSource` (KMS/ESO/CSI/Vault-backed). Both paths inject an environment variable named `STELLAR_CORE_SEED` into the pod spec. To prevent the API server from rejecting the pod due to duplicate environment variable names, the pod builder merges env vars **by name** using `merge_env_overrides` (see `src/controller/resources.rs`) instead of appending. The last writer wins, which gives `seedSecretSource` precedence over `seedSecretRef` — matching the precedence in `ValidatorConfig::resolve_seed_source()`. If both fields are set, only one `STELLAR_CORE_SEED` entry appears in the rendered pod spec, sourced from `seedSecretSource`.
- **No hard rejection**: The operator does **not** reject a CR that sets both `seedSecretRef` and `seedSecretSource`; it silently deduplicates. This preserves backward compatibility with existing clusters that may have both fields populated during migration.
- **Auditing other env vars**: The same `merge_env_overrides` mechanism is used for `stellarCoreEnv` (Validator), `horizonEnv` (Horizon), and any custom env vars injected via CSI/Vault. Contributors adding new env var injection paths **must** route them through `merge_env_overrides` (or `build_container` for the legacy `seedSecretRef` path) rather than using `Vec::extend` on the container's `env` list. A property-based test in `src/controller/seed_env_dedupe_test.rs` asserts uniqueness of all env var names across all three node types.

### Documentation conventions

- Documentation files use `kebab-case.md` (e.g., `disk-scaling.md`).
- Files that belong to a topic area go in the matching `docs/<topic>/` subdirectory.
- Root-level docs (`README.md`, `DEVELOPMENT.md`, `CONTRIBUTING.md`) are entry points only — detailed content belongs in `docs/`.
- New doc files must be added to `mkdocs.yml` under the appropriate section.
- Design storyboards and pitch material live in
  [`docs/pitch-storyboards.md`](docs/pitch-storyboards.md);
  link to them from `CONTRIBUTING.md` or `DEVELOPMENT.md` rather than duplicating
  their content.
- Change notes and release tracking procedures live in
  [`docs/change-notes.md`](docs/change-notes.md).

### Script conventions

- Scripts use `kebab-case.sh` (e.g., `cleanup.sh`).
- Every script must pass `shellcheck -S error`.
- Do not add one-off archive or batch scripts under `scripts/`. Use
  `scripts/cleanup.sh` (`make cleanup`) as the single cleanup entrypoint, or
  remove obsolete helpers entirely.
- Historical or one-off scripts should not be committed to the repository; keep only operational scripts under `scripts/`.

### Manifest and config conventions

- CRD YAML files follow the `stellar{feature}-crd.yaml` naming pattern under `config/crd/`.
- Example manifests in `examples/` use descriptive, feature-based names — not issue numbers.
- Generated manifests (CRDs, API reference, bundle) must be regenerated from their source before merging.

## 9. Getting Help

If you need help with your contribution, here are the best places to ask:

- **GitHub Discussions**: For questions about the project, design discussions, and general help
- **Issue Tracker**: For bug reports and feature requests (use the appropriate template)
- **PR Comments**: For questions about a specific pull request
- **Codeowners**: Tag the relevant team via the [CODEOWNERS](.github/CODEOWNERS) file for path-specific questions

### Before Asking

1. Search existing issues and discussions for your question
2. Check the [Troubleshooting Quick Links](#troubleshooting-quick-links) above
3. Review the [DEVELOPMENT.md](DEVELOPMENT.md) for detailed setup and debugging guides
4. Run `make health` and `make ci-local` to isolate local environment issues

When asking for help, please include:
- What you're trying to achieve
- What you've tried so far
- Any error messages (full output, not truncated)
- Your environment (OS, Rust version, Kubernetes version, etc.)
