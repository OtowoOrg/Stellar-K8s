# Dependency Audit

*Addresses issues #1714, #1816.*

This document describes how to run the dependency audit for Stellar-K8s,
what each gate checks, and where the results are enforced in CI.

---

## Quick start

```bash
make dependency-audit               # full lockfile gate (audit + deny + licenses + locked build)
make dependency-audit AUDIT_QUICK=1 # fast gate (audit + deny only, for day-to-day use)
make audit                          # alias for full dependency audit
make audit-quick                    # alias for fast dependency audit
```

Both targets wrap [`scripts/dep-gate.sh`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/dep-gate.sh), the single
consolidated lockfile gate. The script relies on
[`.cargo/audit.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.cargo/audit.toml) and [`deny.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/deny.toml)
as the canonical configuration files — advisories are suppressed there with
written justification, never with inline `--ignore` flags.

---

## What the full gate checks

| # | Check | Tool | Fails when |
|---|-------|------|------------|
| 1 | Vulnerability scan | `cargo audit` | An unignored advisory affects `Cargo.lock` |
| 2 | Dependency policy | `cargo deny check` | A license, ban, advisory, or source rule in `deny.toml` is violated |
| 3 | License drift | `generate-third-party-licenses.sh --check` | [`THIRD_PARTY_LICENSES.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/THIRD_PARTY_LICENSES.md) is stale — run `make third-party-licenses` |
| 4 | Lockfile integrity | `cargo build --locked --release` | `Cargo.lock` is inconsistent with `Cargo.toml` — run `cargo update` |

`make audit-quick` runs checks 1–2 only, so dependency-only changes can be
verified in seconds without a full release build.

---

## Configuration files

- [`.cargo/audit.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.cargo/audit.toml) — ignored advisories. Every
  entry names the affected transitive crate and why it cannot be remediated
  yet (e.g. upstream ecosystem constraints). Re-evaluate entries when upstream
  publishes a fix.
- [`deny.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/deny.toml) — license allow-list, banned crates
  (e.g. `openssl` in favour of `rustls`), and advisory/source policy.
- [`.github/dependabot.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/dependabot.yml) — monthly automated
  update schedule for `cargo`, `github-actions`, and `docker` ecosystems.

---

## CI enforcement

| Workflow | Trigger | What it runs |
|----------|---------|--------------|
| [`ci.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/ci.yml) (`security-audit` job) | PR / push touching dependencies | `cargo audit` |
| [`dependency-review.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/dependency-review.yml) | Dep changes, weekly schedule | `cargo deny`, stale-dep report, lockfile diff, license inventory |
| [`security-audit.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/security-audit.yml) | Daily schedule, manual dispatch | `cargo deny`, `cargo audit`, SBOM upload |
| [`dep-auto-merge.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/dep-auto-merge.yml) | Dependabot PRs | Auto-merge when gates pass |

See also [Automated Dependency Updates](dependency-updates.md) for how
Dependabot PRs are identified and merged, [Security Review](security-review.md)
and [Vulnerability Scanning](vulnerability-scan.md) for container and Helm
scanning gates, and the
[Security Compliance Guide](compliance-guide.md) for the wider policy context.

---

## Adding a new exception

1. Run `make audit` and read which check failed.
2. For a vulnerability: confirm the affected code path is unreachable or
   unexploitable in this project, then add the advisory ID to the `ignore`
   list in `.cargo/audit.toml` with a comment recording the justification
   and a re-evaluation condition.
3. For a license or ban: update `deny.toml` and regenerate the license
   inventory with `make third-party-licenses`.
4. Re-run `make audit` until all four checks pass.
