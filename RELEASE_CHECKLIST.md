# Release Checklist — Stellar-K8s

> **Single source of truth** for every release of `stellar-k8s`.  
> This file is parsed by `.github/workflows/release-gate.yml` to enforce each
> gate automatically before a tag is published.  
> **Do not remove or rename sections** — the gate script depends on them.

---

## Pre-Release Checklist

### 1. Version Bump
- [ ] `Cargo.toml` `version` field updated to the new semver
- [ ] `charts/stellar-operator/Chart.yaml` `version` and `appVersion` updated
- [ ] `CHANGELOG.md` has an entry for this version

### 2. CI Green
- [ ] All CI jobs pass on the release commit (`main` branch)
- [ ] `cargo audit` shows no unignored CRITICAL/HIGH advisories
- [ ] `cargo deny check` passes (licenses + bans)
- [ ] Helm lint passes (`helm lint charts/stellar-operator --strict`)
- [ ] Helm unit tests pass (`helm unittest charts/stellar-operator --strict`)

### 3. Docker Image
- [ ] Multi-arch image builds successfully (`linux/amd64`, `linux/arm64`)
- [ ] Trivy scan shows no new CRITICAL vulnerabilities
- [ ] Image tag matches the release version

### 4. Helm Chart
- [ ] Chart version bumped to match release version
- [ ] `helm template` renders without errors
- [ ] Values schema (`values.schema.json`) is up to date

### 5. Documentation
- [ ] `docs/api-reference.md` regenerated (`make generate-api-docs`)
- [ ] README Quick Start commands tested against the new version
- [ ] `CHANGELOG.md` reviewed for accuracy

### 6. Tag & Release
- [ ] Git tag is `v<semver>` (e.g. `v1.2.0`)
- [ ] Tag is pushed to `origin/main` (not a feature branch)
- [ ] GitHub Release draft created with changelog body
- [ ] Binary artifacts attached (linux-amd64, linux-arm64, darwin-amd64, darwin-arm64)
- [ ] Helm chart `.tgz` attached to the release

### 7. Post-Release
- [ ] Helm repository index updated
- [ ] Docker Hub / GHCR `latest` tag points to new release
- [ ] GitHub Release published (un-drafted)
- [ ] Release announcement drafted (Discord / GitHub Discussions)

---

## Validation Gate

The `.github/workflows/release-gate.yml` workflow runs automatically on every
`v*.*.*` tag push and verifies the following **hard gates** — a failing gate
blocks the release:

| Gate | Command | Failure Action |
|------|---------|----------------|
| CHANGELOG entry exists | grep | Abort |
| Helm unit tests pass | `helm unittest --strict` | Abort |

Semver format, Cargo.toml / Chart.yaml tag matching, `cargo audit`, and
`helm lint --strict` are enforced by `release.yml` and `ci.yml` — they are not
duplicated in the release gate.

Run the gate's checks locally before tagging:

```bash
grep -qE "^## \[?v?1\.2\.0\]?" CHANGELOG.md   # CHANGELOG entry exists
helm unittest charts/stellar-operator --strict
```
