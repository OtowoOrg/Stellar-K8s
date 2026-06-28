# Regulatory Framework Mapping (SOC2, GDPR, PCI-DSS)

> This document maps operational controls for Stellar-K8s deployments to common enterprise audit frameworks.

---

## 1) SOC 2 Type II Mapping

| Trust Service Criteria | Stellar-K8s Control | Evidence Artifact |
|---|---|---|
| Security (CC6) Logical access | Namespace-scoped RBAC and workload identity | `kubectl auth can-i` reports, IAM binding exports |
| Availability (A1) Resilience | Multi-zone validator anti-affinity and PDBs | Deployment manifests, SLO reports |
| Confidentiality (C1) Data protection | etcd encryption at rest + mTLS in transit | EncryptionConfiguration, mesh policy snapshots |
| Processing Integrity (PI1) Change control | GitOps PR approvals and policy checks | Signed commits, ArgoCD sync history |

### SOC2 Procedure Notes

- Enforce two-person review for all validator spec changes.
- Retain operator audit logs for at least 13 months.
- Run quarterly access recertification for cluster-admin and operator roles.

---

## 2) GDPR Considerations

### Data Classification in Stellar-K8s

| Data Element | Typical Location | GDPR Consideration |
|---|---|---|
| Peer and client source IPs | ingress/controller logs, SIEM | Personal data in some jurisdictions |
| Node hostnames and operator IDs | metrics labels, audit logs | Pseudonymous but attributable metadata |
| Ledger content | immutable blockchain state | Generally public chain data; avoid introducing personal overlays |

### Guardrails

- Define strict retention windows for ingress/API logs containing source IPs.
- Apply log redaction on user-provided request parameters.
- Document lawful basis for operational telemetry collection.
- Keep transaction ingestion boundaries explicit to avoid storing additional personal fields in side systems.

---

## 3) PCI-DSS Mapping

> For environments that bridge payment processing systems with Stellar infrastructure, treat exposed API tiers as potentially in-scope segments unless formally segmented and validated.

| PCI Requirement | Implementation Pattern |
|---|---|
| Req. 1: Network security controls | Calico/Cilium default deny + explicit service allow rules |
| Req. 3: Protect stored account data | Keep cardholder data out of Stellar-K8s cluster; encrypt secret material |
| Req. 7: Restrict access by business need | Least-privilege Roles/ClusterRoles per component |
| Req. 10: Logging and monitoring | Kubernetes audit policy + SIEM forwarding |
| Req. 11: Security testing | Scheduled CIS scans, image scanning, policy conformance tests |

---

## 4) Control Matrix Example

```yaml
# controls/stellar-k8s-control-matrix.yaml
controls:
  - id: SK8S-AC-001
    framework: SOC2-CC6.1
    description: Operator role limited to Stellar CRDs and leases
    evidence:
      - rbac/clusterrole-operator.yaml
      - audit/operator-rbac-access.log
  - id: SK8S-DP-002
    framework: GDPR-Data-Minimization
    description: Source IP retention limited to 30 days in hot storage
    evidence:
      - loki/retention-policy.yaml
      - siem/log-retention-attestation.pdf
```

## 5) Audit Readiness Checklist

1. Validate all cluster role bindings map to named owners.
2. Confirm audit policy includes write paths for CRDs, secrets, and role bindings.
3. Confirm key and certificate rotation jobs are within policy windows.
4. Export evidence artifacts in immutable storage before audit windows close.
