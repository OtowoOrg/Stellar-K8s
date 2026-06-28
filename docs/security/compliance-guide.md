# Security Compliance Guide

This document establishes the security compliance posture for deploying and operating Stellar-K8s in enterprise environments. It maps operational controls to regulatory frameworks (**SOC 2**, **GDPR**, **PCI-DSS**) and security benchmarks (**CIS Kubernetes Benchmark**).

---

## 1. SOC 2 Type II Compliance

SOC 2 Type II audits require evidence that system security, availability, processing integrity, confidentiality, and privacy are maintained over time.

| Control Area | SOC 2 Trust Criteria | Stellar-K8s Implementation |
|---|---|---|
| **Access Control** | CC6.1, CC6.2, CC6.3 | Enforced via Kubernetes RBAC with least privilege. |
| **Audit Trails** | CC7.2, CC7.3 | Operator writes comprehensive structured JSON logs to stdout; Kubernetes API audit logging tracks CRD mutations. |
| **Data in Transit** | CC6.6 | Mutual TLS (mTLS) enforced between all nodes via Istio or cert-manager generated secrets. |
| **Vulnerability Scanning** | CC7.1 | Continuous integration scans of images via Trivy, combined with automated CVE runtime patching. |

---

## 2. GDPR Compliance (Data Privacy)

GDPR requires strict controls over the collection, processing, retention, and deletion of Personally Identifiable Information (PII).

### 2.1 Encryption at Rest
All validator and Horizon stateful data must be encrypted at the host storage level:
- Use cloud-provider storage classes that support encryption (e.g., AWS EBS encrypted volumes).
- Configure node-level encryption via dm-crypt/LUKS for bare-metal nodes.

### 2.2 Data Retention & Pruning
Stellar ledger histories can grow indefinitely and contain public keys (which are considered pseudonymous data under GDPR).
- **Pruning**: Enable automatic history archive pruning using the operator's prune command to limit retention to required windows (e.g., 30 days):
  ```bash
  stellar-operator prune-archive --archive-url s3://my-bucket/stellar-history --retention-days 30 --force
  ```
- **IP Address Privacy**: Redact peer node IP addresses in audit logs using log-redaction policies.

---

## 3. PCI-DSS Compliance (Payment & Sensitive Data)

For deployments handling transaction routing or financial settlements, PCI-DSS v4.0 controls apply.

### 3.1 Network Segmentation (Req 1 & Req 2)
Isolate payment-related validator nodes in dedicated namespaces with zero-trust egress:
- Enforce strict per-node NetworkPolicies (see Calico/Cilium configurations).
- Prevent non-payment workloads from running on the same physical nodes using node taints/tolerations.

### 3.2 Secret and Cryptographic Key Management (Req 3 & Req 4)
- **Validator Seed Protection**: Do not store validator seeds in plain text. Use AWS KMS or GCP KMS via `SecretPolicy` to decrypt seeds dynamically in memory.
- **Rotation**: Rotate database credentials periodically (e.g., every 90 days) via cron-based rotation:
  ```bash
  stellar-operator rotate-db-credentials --node-name my-validator --namespace stellar
  ```

---

## 4. CIS Kubernetes Benchmark Implementation

To pass CIS benchmarks, apply the following configurations to the underlying cluster and operator workloads:

### 4.1 Pod Security Standards (PSS)
Enforce the `restricted` PSS profile on all namespace runtimes:
```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: stellar
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/enforce-version: latest
```

### 4.2 Restrictive ServiceAccount & RBAC
- Disable auto-mounting of API tokens where not required: `automountServiceAccountToken: false` in node pods.
- Reconcile least-privilege operator RBAC permissions periodically (see `docs/production-security-hardening.md#rbac-configuration-examples`).

---

## 5. Security Hardening Checklist

Use this checklist to verify compliance before moving workloads to production:

- [ ] **Encryption**: etcd encryption at rest is enabled on the cluster control plane.
- [ ] **Pod Security**: Namespace has `pod-security.kubernetes.io/enforce: restricted` label.
- [ ] **Least Privilege**: Operator ServiceAccount has no cluster-admin privileges (scoped strictly to target namespaces).
- [ ] **Network Policies**: Default deny-all ingress/egress is active; explicit whitelists are configured for P2P ports.
- [ ] **mTLS**: Mutual TLS is enabled for all inter-pod communications.
- [ ] **Secret Management**: External KMS (AWS Key Management Service or GCP Cloud KMS) is integrated.
- [ ] **Logging**: Kubernetes API auditing is active, and operator logs are pushed to an immutable external SIEM.

---

## 6. Audit Logging Setup

### 6.1 Kubernetes API Audit Policy
Deploy this audit policy on your cluster API server to track Stellar-K8s modifications:
```yaml
apiVersion: audit.k8s.io/v1
kind: Policy
rules:
  # Log changes to StellarNode CRDs at RequestResponse level
  - level: RequestResponse
    resources:
      - group: "stellar.org"
        resources: ["stellarnodes", "stellarvalidators"]
  # Log secrets access at Metadata level to avoid logging secret payload
  - level: Metadata
    resources:
      - group: ""
        resources: ["secrets"]
```

---

## 7. Incident Response Playbook

In case of a security breach:

### Phase 1: Identification
- Detect anomalies through Prometheus alerts (e.g., unexpected outbound traffic from validator pods).

### Phase 2: Containment
- Isolate the pod network immediately using a deny-all NetworkPolicy (see `operator-troubleshooting.md#security-incident-response-procedures`).
- Scale down suspected deployments to `0` if data theft is actively suspected.

### Phase 3: Eradication
- Delete and rotate all compromised secrets and keys.
- Run container vulnerability scans (Trivy) to identify if the container image was compromised.

### Phase 4: Recovery & Post-Mortem
- Restore validator nodes using safe/validated container images.
- Review API audit logs to identify the ingress vector.
