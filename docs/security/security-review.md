# Security Review

> **Issues #1713, #1815** — Automated security-review CI workflow and local `make security-review` target consolidating dependency auditing, container scanning, and Helm security linting into a single gate.

## Overview

The security review gate runs on every pull request that touches source code, Cargo manifests, Helm charts, or the Dockerfile.  It also runs daily at 03:00 UTC to catch advisories published overnight.

| Check | Tool | Failure condition |
|---|---|---|
| Dependency vulnerabilities | `cargo-audit` | Any unexcepted RUSTSEC advisory |
| License / ban / source policy | `cargo-deny` | Policy violation in `deny.toml` |
| Container & filesystem scan | Trivy | CRITICAL or HIGH CVE |
| Helm chart misconfiguration | Trivy config scan + `helm lint --strict` | CRITICAL/HIGH finding or lint error |

SARIF results from Trivy are uploaded to the repository's **Security → Code scanning** tab for triage.

## Local Usage

```bash
# Full security review (audit + deny + report written to security/reports/review.md)
make security-review

# Custom report path or skip Helm lint when helm is not installed locally
make security-review SECURITY_REVIEW_REPORT=security/reports/custom-review.md SECURITY_REVIEW_SKIP_HELM=1

# Individual checks
make audit          # cargo-audit + cargo-deny via scripts/dep-gate.sh
make security-scan  # audit + shellcheck + shell safety
make security-all   # audit + shellcheck + SBOM generation
```

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SECURITY_REVIEW_REPORT` | `security/reports/review.md` | Output path for the consolidated Markdown review report |
| `SECURITY_REVIEW_SKIP_HELM` | `0` | Set to `1` to skip the `helm lint --strict` section during local runs |

## CI Behaviour

| Trigger | Jobs |
|---|---|
| PR / push touching `src/**`, `Cargo.*`, `deny.toml`, `charts/**`, `Dockerfile` | `dependency-audit`, `trivy-scan`, `helm-security` |
| Daily schedule (03:00 UTC) | All three jobs |
| `workflow_dispatch` | All three jobs |

All three jobs feed into a `security-summary` job that writes a consolidated Markdown table to the workflow summary.

## Artifacts

| Artifact | Retention |
|---|---|
| `sbom-<sha>/` (dependency tree + license JSON) | 90 days |
| `security/reports/review.md` (local only) | — |
| Trivy SARIF → Security tab | Permanent (GitHub managed) |

## Dependency Policy

`deny.toml` at the repository root governs which licenses are allowed, which crates are banned, and which registries are trusted.  All `cargo-deny` policy changes must be reviewed by a maintainer.

## Escalation

Security findings that cannot be remediated immediately must be tracked via a GitHub Security Advisory.  See [`security/incident-response-playbook.md`](incident-response-playbook.md) for the escalation procedure.

## Cross-References

- CI workflow: [`.github/workflows/security-review.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/security-review.yml)
- Existing audit workflow: [`.github/workflows/security-audit.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/security-audit.yml)
- Compliance guide: [`compliance-guide.md`](compliance-guide.md)
- Dependency audit: [`dependency-audit.md`](dependency-audit.md)
- Vulnerability scanning: [`vulnerability-scan.md`](vulnerability-scan.md)
- Dependency policy: [`dependency-updates.md`](dependency-updates.md)
- Incident response: [`incident-response-playbook.md`](incident-response-playbook.md)
