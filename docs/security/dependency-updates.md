# Automated Dependency Updates

This document describes how Stellar-K8s receives, reviews, and merges automated
dependency updates. It closes the process gap for issue **#1332**.

Automated updates are provided by **[Dependabot](https://docs.github.com/en/code-security/dependabot)**
(configuration: [`.github/dependabot.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/dependabot.yml)).
**Renovate is not used** - do not enable a second update bot alongside Dependabot.

---

## How automated PRs are identified

Dependabot opens pull requests on a **monthly Monday** schedule for these ecosystems:

| Ecosystem | What it updates | Typical PR signals |
|-----------|-----------------|--------------------|
| `cargo` | Root `Cargo.toml` / `Cargo.lock` | Author `app/dependabot`; labels `dependencies`, `rust`; commit prefix `deps` |
| `github-actions` | Actions used under `.github/` | Labels `dependencies`, `github-actions`; commit prefix `ci` |
| `docker` | Root `Dockerfile` / `Dockerfile.dev` base images | Labels `dependencies`, `docker`; commit prefix `build` |

Additional cues:

- Branch names usually start with `dependabot/`
- Cargo updates are grouped (production minor/patch, kubernetes client, tokio, serialization, tracing, security crates)
- GitHub Actions updates are grouped into a single monthly PR when possible

Security advisories may also produce Dependabot alerts/PRs outside the monthly cadence when GitHub Dependabot security updates are enabled for the repository.

---

## CI checks that must pass before merge

Every Dependabot PR targets `main` and therefore runs the same pull-request CI as
human PRs. **Do not merge while required checks are red or still running.**

### Always relevant (all Dependabot PRs)

| Workflow / job | Role |
|----------------|------|
| [CI/CD Pipeline](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml) (`ci.yml`) | Preflight, hygiene, lint (as applicable), tests, and related PR gates |
| Conventional commits | Commit message format |

### Cargo / lockfile PRs (`Cargo.toml`, `Cargo.lock`)

| Check | Where | What it enforces |
|-------|-------|------------------|
| `security-audit` | `ci.yml` | `cargo audit` + third-party license reference check |
| `dep-graph-guard` | `ci.yml` | Caps unexpected `Cargo.lock` graph growth |
| Lint / format / tests | `ci.yml` | Compatibility: clippy/fmt when Rust core paths change; `make test` |
| Cargo Deny (licenses + bans) | [dependency-review.yml](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/dependency-review.yml) | `cargo deny check` against `deny.toml` |
| Dependency Diff | `dependency-review.yml` | Summarizes `Cargo.lock` name/version churn on the PR |

Local equivalent for cargo security/policy:

```bash
make audit          # scripts/dep-gate.sh - cargo audit + cargo deny (+ fuller checks)
```

Canonical ignore/exception lists live in [`.cargo/audit.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.cargo/audit.toml)
and [`deny.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/deny.toml). Do not paper over new advisories with one-off
inline CI flags.

### GitHub Actions PRs

Rely on `ci.yml` (and any path-triggered workflow validation). Confirm the PR does
not break composite actions under `.github/actions/`.

### Docker base-image PRs

`ci.yml` still runs on the PR. Image Trivy scanning of the published operator image
runs primarily on `main` via the docker/security-scan path after merge - treat
Dockerfile bumps carefully (toolchain pins in `.github/actions/setup-rust` and
sample versions may need a follow-up).

Scheduled reinforcement (not a substitute for green PR CI):

- [security-audit.yml](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/security-audit.yml) - daily audit / deny / SBOM / scorecard
- [container-image-security.yml](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/container-image-security.yml) - Trivy / Grype image scans on `main`

---

## What reviewers should verify

1. **CI is green** for the PR (see tables above).
2. **Scope matches the bot** - only expected manifest/lockfile/Dockerfile/workflow pins.
3. **For cargo:** `cargo audit` / deny results; whether `THIRD_PARTY_LICENSES.md` must be regenerated (`make third-party-licenses` / `make check-third-party-licenses`).
4. **For majors or wide groups:** compile/test impact, kube/k8s-openapi alignment, and pinned security crates (`anyhow`, `bytes`, etc. in `Cargo.toml`).
5. **No secrets or unrelated app changes** slipped into the PR.

---

## When an update can be approved

Approve when **all** of the following hold:

- Required CI checks passed
- Diff is limited to dependency (or Action/base-image) updates
- No new unignored CRITICAL/HIGH advisories (or any new ignore is justified in `.cargo/audit.toml` with rationale)
- For grouped cargo updates: smoke judgment that the set is coherent (same ecosystem group)

Routine **minor/patch** cargo groups and Actions bumps that are fully green are normal
candidates for merge during the monthly maintenance window described in
[maintenance-dashboard.md](../maintenance-dashboard.md).

---

## When to investigate instead of merging

Investigate (do not blind-merge) when:

- Any required CI job failed or was cancelled without a green retry
- The PR bumps **major** versions (especially `kube*`, `k8s-openapi`, `schemars`, or large framework jumps)
- `dep-graph-guard` fails (unexpected lockfile expansion)
- `cargo deny` or `cargo audit` fails
- Docker rust/`cargo-chef` tags diverge from the CI Rust toolchain pin
- The PR touches files outside dependency manifests without explanation
- Reviewers cannot tell whether a pin in `Cargo.toml` was intentional for a security fix

---

## Handling failing dependency updates

1. Read the failing job log (usually `Security Audit`, `Cargo Deny`, `Lint & Format`, or `Test`).
2. Reproduce locally with `make audit`, `make lint`, and `make test` as appropriate.
3. Options:
   - **Fix forward** on a human branch (resolve API breaks, regenerate licenses, adjust pins).
   - **Close** the Dependabot PR if the upgrade is not viable yet; open a tracking issue if needed.
   - **Adjust policy** only with justification (update `.cargo/audit.toml` / `deny.toml`, or Dependabot config via a reviewed PR owned by the teams in CODEOWNERS).
4. Do not merge with failing required checks.

Dependabot will often reopen or recreate updates on the next schedule once `main` moves.

---

## Major-version and breaking updates

- Prefer reviewing majors **separately** from large "production-dependencies" patch bundles when possible.
- Kubernetes client stack (`kube*`, `k8s-openapi`) must stay feature-aligned with
  `K8S_OPENAPI_ENABLED_VERSION` / crate features used in CI.
- Treat majors as feature work: extra review, possible follow-up commits, and explicit
  test focus - not a rubber-stamp maintenance merge.
- If a Dependabot group repeatedly fails on majors, change grouping/ignores in
  `.github/dependabot.yml` through a normal reviewed PR rather than merging red CI.

---

## Who approves (repository evidence only)

From [`.github/CODEOWNERS`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/CODEOWNERS):

| Path / change | Review request |
|---------------|----------------|
| Default (including most dependency PRs) | `@stellar-k8s-maintainers` |
| `.github/dependabot.yml` and workflows | `@stellar-k8s-maintainers` and `@devops-team` |
| `Dockerfile` | `@stellar-k8s-maintainers` and `@devops-team` |

There is no separate named "dependency approver" beyond CODEOWNERS and normal maintainer
review. Security-sensitive ignore list edits should follow [`SECURITY.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/SECURITY.md)
and keep rationales in `.cargo/audit.toml`.

Final merge authority is the repository maintainers who merge to `main` after review
and green CI (see [CONTRIBUTING.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md) merge conventions).

---

## Expected merge process

1. Dependabot opens the PR (labels + conventional commit prefix as configured).
2. Wait for CI (and dependency-review jobs when cargo files change) to finish green.
3. Maintainer reviews diff against this checklist.
4. Request/wait for CODEOWNERS review as applicable.
5. Merge using the repository's usual method (typically **squash and merge** for
   maintenance PRs per CONTRIBUTING).
6. Confirm `main` stays green; for Docker bumps, watch post-merge image security scan jobs.
7. If licenses changed, ensure `THIRD_PARTY_LICENSES.md` remains accurate on `main`.

---

## Configuration reference

| Artifact | Purpose |
|----------|---------|
| `.github/dependabot.yml` | Ecosystems, schedule, labels, commit prefixes, groups |
| `.github/workflows/ci.yml` | PR CI including `security-audit` and `dep-graph-guard` on dep changes |
| `.github/workflows/dependency-review.yml` | `cargo deny`, lockfile diff, scheduled stale/license reports |
| `.github/workflows/security-audit.yml` | Scheduled audit / deny / SBOM / OpenSSF Scorecard |
| `.cargo/audit.toml` | Justified `cargo audit` exceptions |
| `deny.toml` | License, ban, and advisory policy for `cargo deny` |
| `scripts/dep-gate.sh` / `make audit` | Local consolidated dependency gate |

---

## Related docs

- [Contributor Maintenance Dashboard](../maintenance-dashboard.md)
- [Release Process](../release-process.md) (dependency updates are typically PATCH-class)
- [Production Security Hardening](../production-security-hardening.md)
- [SECURITY.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/SECURITY.md)
