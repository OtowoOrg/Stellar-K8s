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
| Validator seed custody | [key-sharding.md](key-sharding.md) | Shamir sharding, Vault Transit assembly, air-gapped ceremonies |
| Incident response playbook | [incident-response-playbook.md](incident-response-playbook.md) | Fast containment and forensic continuity |
| RPC Rate Limiting & DoS Mitigation | [rpc-dos-mitigation.md](rpc-dos-mitigation.md) | Defense-in-depth L7 ingress and edge security (#250) |
| Automated dependency updates | [dependency-updates.md](dependency-updates.md) | Dependabot PR review, CI gates, and merge process |

## Security Principles for Stellar Core Workloads

1. Validator signing material and TLS trust anchors are Tier-0 assets.
2. Validator east-west communication is explicitly allow-listed.
3. Public RPC ingress paths are isolated from validator trust zones.
4. Every privileged operation is attributable through audit trails.

## Related Existing Docs

- [rpc-dos-mitigation.md](rpc-dos-mitigation.md)
- [credentials-and-secrets.md](credentials-and-secrets.md)
- [pss.md](pss.md)
- [dependency-updates.md](dependency-updates.md)
- [../production-security-hardening.md](../production-security-hardening.md)
- [../gatekeeper-policies.md](../gatekeeper-policies.md)
