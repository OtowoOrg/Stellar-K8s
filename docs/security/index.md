# Security Compliance Guide

> Production security baseline for Stellar-K8s operator environments, mapped to SOC 2 Type II, GDPR, PCI-DSS, and CIS Kubernetes Benchmark controls.
>
> Tracking: Closes #999

---

## Contents

| Topic | File | Outcome |
|---|---|---|
| Compliance control mapping | [compliance-mapping.md](compliance-mapping.md) | Audit-ready control traceability |
| CIS hardening patterns | [cis-kubernetes-hardening.md](cis-kubernetes-hardening.md) | Automated cluster baseline enforcement |
| Secrets and least privilege | [secrets-access-control.md](secrets-access-control.md) | Minimized blast radius and secure rotation |
| Incident response playbook | [incident-response-playbook.md](incident-response-playbook.md) | Fast containment and forensic continuity |
| Automated dependency updates | [dependency-updates.md](dependency-updates.md) | Dependabot PR review, CI gates, and merge process |
| Security review gate | [security-review.md](security-review.md) | Consolidated dependency audit, Trivy scan, and Helm security gate |

## Security Principles for Stellar Core Workloads

1. Validator signing material and TLS trust anchors are Tier-0 assets.
2. Validator east-west communication is explicitly allow-listed.
3. Public RPC ingress paths are isolated from validator trust zones.
4. Every privileged operation is attributable through audit trails.

## Related Existing Docs

- [credentials-and-secrets.md](credentials-and-secrets.md)
- [pss.md](pss.md)
- [dependency-updates.md](dependency-updates.md)
- [SECURITY.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/SECURITY.md) — vulnerability reporting and disclosure policy
- [../production-security-hardening.md](../production-security-hardening.md)
- [../gatekeeper-policies.md](../gatekeeper-policies.md)
