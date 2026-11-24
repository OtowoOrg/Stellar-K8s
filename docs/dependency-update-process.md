# Automated Dependency Updates & Security Patching

*Addresses issue #1418.*

This document describes how Stellar-K8s manages dependency freshness and
security patching in an automated, auditable way.

---

## Overview

| Ecosystem | Tool | Schedule | Auto-merge? |
|---|---|---|---|
| Rust (Cargo) | Dependabot | Weekly (Monday 09:00 UTC) | Patch + minor (non-crypto) |
| GitHub Actions | Dependabot | Weekly (Monday 09:00 UTC) | Patch + minor |
| Docker base images | Dependabot | Weekly (Monday 09:00 UTC) | Patch + minor |
| Helm charts | Dependabot | Weekly (Tuesday 09:00 UTC) | Patch + minor |
| Advisory scanning | `cargo-audit` / `cargo-deny` | Every push + nightly | Blocks CI |

---

## Dependabot Configuration

The configuration lives in [`.github/dependabot.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/dependabot.yml).

### Grouping Strategy

Related crates are grouped to minimise PR noise:

| Group | Crates | Rationale |
|---|---|---|
| `kubernetes-client` | `kube*`, `k8s-openapi` | Must be bumped together |
| `tokio-ecosystem` | `tokio*`, `hyper*`, `tower*`, `axum*` | Async runtime compatibility |
| `serialization` | `serde*`, `schemars` | Often release together |
| `tracing` | `tracing*`, `opentelemetry*` | Observability stack |
| `security-crypto` | `rcgen`, `rustls*`, `ring`, `openssl*` | Requires human review |
| `production-dependencies` | all remaining | Catch-all for patch bumps |

### PR Labels

| Label | Meaning |
|---|---|
| `dependencies` | All automated dependency PRs |
| `automerge-candidate` | Eligible for auto-merge after CI |
| `security` | Security advisory PRs — **never auto-merged** |
| `rust` / `docker` / `helm` / `github-actions` | Ecosystem tag |

---

## Auto-Merge Workflow

[`.github/workflows/dep-auto-merge.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/dep-auto-merge.yml)
runs on every Dependabot PR and:

1. Fetches Dependabot metadata (update type, package ecosystem).
2. **Approves** PRs that are `semver-patch` or `semver-minor`.
3. **Enables GitHub auto-merge** (squash strategy) so the PR merges as soon
   as all required status checks pass.
4. **Does nothing** for `semver-major` — those require a human reviewer.
5. **Posts a warning comment** and does not enable auto-merge for any PR
   carrying the `security` label.

### Branch Protection Prerequisites

For auto-merge to work, the `main` branch must have:

- "Require status checks to pass before merging" enabled.
- The following checks required: `ci / build`, `ci / test`, `ci / security-audit`.
- "Require a pull request before merging" enabled (the workflow provides the approval).

---

## CI Compatibility Checks

Dependabot PRs go through the full CI suite (`ci.yml`), which includes:

- `cargo build --workspace` — compilation check.
- `cargo test --workspace` — unit + integration tests.
- `cargo deny check` — license and advisory policy.
- `cargo audit` — RUSTSEC advisory database scan.
- Trivy image scan — container vulnerability gate.

A PR can only be merged (manually or automatically) after all checks are green.

---

## Security Advisory Patching Process

When `cargo-audit` or Dependabot finds a **security advisory**:

1. Dependabot opens a PR labelled `security` within hours of the advisory
   being published to [RustSec](https://rustsec.org/).
2. The auto-merge workflow posts a comment requiring human review.
3. A maintainer reviews the advisory, the diff, and the upstream changelog.
4. If the fix is safe, the maintainer approves and merges.
5. A patch release is cut following the [Release Process](release-process.md).

### Emergency Patching (Critical CVE)

For CVSS ≥ 9.0 advisories:

```bash
# 1. Branch from main
git checkout -b fix/cve-YYYY-NNNNN

# 2. Apply fix (usually a version bump in Cargo.toml)
cargo update -p <crate>

# 3. Verify no regressions
cargo test --workspace
cargo audit

# 4. Open a PR — add the security label manually so reviewers are notified
gh pr create --label security --title "fix(deps): patch CVE-YYYY-NNNNN in <crate>"
```

---

## Renovate (Optional Alternative)

If your organisation prefers [Renovate](https://docs.renovatebot.com/) over
Dependabot, the equivalent configuration is:

```json
{
  "extends": ["config:base"],
  "schedule": ["every weekend"],
  "packageRules": [
    { "matchUpdateTypes": ["patch", "minor"], "automerge": true },
    { "matchPackagePatterns": ["^kube", "k8s-openapi"], "groupName": "kubernetes-client" },
    { "matchPackagePatterns": ["^tokio", "^hyper", "^tower", "^axum"], "groupName": "tokio-ecosystem" }
  ]
}
```

Place this in `renovate.json` at the repository root and remove
`.github/dependabot.yml` to avoid duplicate PRs.

---

## Verification

- Dependabot PRs appear in the [Pull Requests](https://github.com/OtowoOrg/Stellar-K8s/pulls?q=is%3Apr+author%3Aapp%2Fdependabot)
  tab weekly.
- Each PR shows a green CI badge before merging.
- The `DEPENDENCY_SECURITY_AUDIT.md` document is regenerated on every release.
