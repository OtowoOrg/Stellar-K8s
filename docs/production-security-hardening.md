# Production Security Hardening Guide

This guide consolidates the project-specific controls, deployment patterns, and operational procedures needed to secure `Stellar-K8s` for production use. It is written for platform engineers running the operator and managed Stellar workloads in shared or dedicated Kubernetes clusters.

This document complements, but does not replace, the more focused guides already present in this repository:

- `docs/security/pss.md`
- `docs/network-isolation.md`
- `docs/secret-management-kms.md`
- `docs/secret-rotation.md`
- `docs/compliance-reporting.md`
- `docs/SECURITY_TESTING.md`
- `docs/incident-response/post-mortem.md`
- `SECURITY.md`

## Security Objectives

Production deployments should be hardened to meet these goals:

1. Prevent privilege escalation inside the cluster.
2. Prevent cross-network peering or data-plane mistakes between Mainnet, Testnet, and custom networks.
3. Keep validator seeds, database credentials, and API tokens out of source control, pod specs, and logs.
4. Restrict operator and API access to least privilege.
5. Produce durable audit evidence for security operations and compliance reviews.
6. Continuously scan code, images, and manifests before release and after deployment.
7. Prepare responders with repeatable incident handling templates.

## Recommended Deployment Model

Use the following baseline model in production:

- Run the operator in a dedicated namespace such as `stellar-system`.
- Run Mainnet, Testnet, and other network environments in separate namespaces.
- Set `watchNamespace` whenever possible to avoid cluster-wide operator privileges.
- Enable namespace-level network isolation and per-node `spec.networkPolicy`.
- Enforce Kubernetes Pod Security Standards at the `restricted` level.
- Store high-value secrets in KMS, Vault, or an External Secrets workflow instead of hand-managed Kubernetes Secret manifests.
- Protect every operator/API endpoint with Kubernetes RBAC and, if enabled, OIDC.
- Enable Kubernetes API server audit logs and operator audit persistence.
- Keep production images pinned by digest and sourced only from approved registries.

## Threat Model Summary

The hardening guidance below primarily addresses:

- Privileged or misconfigured pods that could escape containment.
- Lateral movement between namespaces or between Stellar networks.
- Secret leakage through Git, pod environment, logs, or over-broad RBAC.
- Unauthorized use of the optional REST API and operational endpoints.
- Tampering with compliance evidence and audit trails.
- Supply chain exposure from vulnerable dependencies, images, or manifests.

## Pod Security Standards

`Stellar-K8s` already documents and enforces a `restricted` Pod Security Standard posture. Production environments should treat that as mandatory, not advisory.

### Namespace policy

Apply these labels to every namespace that hosts Stellar workloads:

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: stellar-mainnet
  labels:
    pod-security.kubernetes.io/enforce: restricted
    pod-security.kubernetes.io/enforce-version: latest
    pod-security.kubernetes.io/warn: restricted
    pod-security.kubernetes.io/warn-version: latest
    pod-security.kubernetes.io/audit: restricted
    pod-security.kubernetes.io/audit-version: latest
    stellar.org/network: mainnet
```

### Required workload settings

Every production pod should retain the repo's current hardened defaults:

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 10000
  runAsGroup: 10000
  fsGroup: 10000
  seccompProfile:
    type: RuntimeDefault
```

Every container should retain:

```yaml
securityContext:
  allowPrivilegeEscalation: false
  privileged: false
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  capabilities:
    drop: ["ALL"]
  seccompProfile:
    type: RuntimeDefault
```

### Production rules

- Do not override `runAsUser` to `0`.
- Do not add Linux capabilities unless there is a documented exception and a change record.
- Do not enable `hostNetwork`, `hostPID`, `hostIPC`, or `privileged`.
- Do not mount Docker or container runtime sockets into any workload.
- Treat user-supplied sidecars as untrusted until they pass the same PSS review as first-party containers.
- Keep the forensic snapshot exception operationally gated and auditable because it intentionally uses elevated capabilities for investigations.

### Pod security verification checklist

- Namespace labels show `restricted` for `enforce`, `warn`, and `audit`.
- Pods run as non-root with `RuntimeDefault` seccomp.
- All containers drop `ALL` capabilities.
- Images do not require a writable root filesystem.
- Sidecars are reviewed for PSS compliance before rollout.

## Network Security Best Practices

Network controls should assume that a single policy can be deleted or drift over time. Use multiple layers.

### 1. Separate namespaces by Stellar network

Never mix Mainnet and Testnet nodes in the same namespace. Use a dedicated namespace label as the trust anchor:

```yaml
metadata:
  labels:
    stellar.org/network: mainnet
```

Recommended namespace split:

- `stellar-system` for the operator and supporting controllers.
- `stellar-mainnet` for Mainnet validators and services.
- `stellar-mainnet-horizon` if Horizon or supporting APIs need separate blast radius.
- `stellar-testnet` for Testnet workloads.
- `monitoring` for Prometheus and Grafana only if it is not shared with untrusted workloads.

### 2. Enable namespace-level isolation

Use the Helm chart's namespace-level policies to block cross-network traffic:

```yaml
watchNamespace: "stellar-mainnet"

networkIsolation:
  enabled: true
  labelReleaseNamespace: true
  releaseNamespaceNetwork: "mainnet"
  mainnetNamespaces:
    - stellar-mainnet
    - stellar-mainnet-horizon
  testnetNamespaces:
    - stellar-testnet
  allowMonitoringNamespace: true
  monitoringNamespace: monitoring
```

This activates the chart templates that create:

- `deny-non-mainnet-ingress`
- `deny-non-mainnet-egress`
- `deny-non-testnet-ingress`
- `deny-non-testnet-egress`

### 3. Enable per-node NetworkPolicy

In addition to namespace isolation, enable per-node policies on every `StellarNode`:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: validator-mainnet-a
  namespace: stellar-mainnet
spec:
  nodeType: Validator
  network: Mainnet
  networkPolicy:
    enabled: true
    allowNamespaces:
      - stellar-mainnet-horizon
    allowMetricsScrape: true
    metricsNamespace: monitoring
```

### 4. Restrict ingress and egress aggressively

- Allow validator peer traffic only from same-network namespaces.
- Limit Horizon or Soroban RPC exposure with Ingress allowlists, mTLS, or private load balancers.
- Keep the optional REST API private behind cluster networking and authentication.
- Permit DNS egress only to trusted cluster DNS.
- Do not grant blanket egress to `0.0.0.0/0` unless an external dependency is explicitly required and documented.

### 5. Protect the operator and webhook path

- Run the operator in a dedicated namespace.
- Protect the admission webhook with valid TLS material and renewal procedures.
- Restrict which namespaces can reach the operator/API service.
- Use a CNI that enforces NetworkPolicy correctly. Calico and Cilium are suitable examples; Flannel alone is not sufficient.

### 6. Add transport encryption

- Enable mTLS between internal services where supported.
- Use TLS for database connections used by Horizon or other supporting services.
- Use TLS for external secret backends, audit sinks, and webhook integrations.

## Secret Management Guidelines

High-value secrets in a Stellar deployment include validator seed phrases, Horizon database passwords, Soroban RPC credentials, TLS keys, OIDC client secrets, and audit sink tokens.

### Secret handling principles

- Never store production secrets in Git.
- Never hardcode secrets in Helm values committed to source control.
- Prefer external secret backends over manually created Kubernetes Secrets.
- Encrypt etcd at rest at the cluster level.
- Limit which service accounts can read specific Secrets.
- Rotate credentials on a fixed schedule and after every suspected compromise.
- Avoid logging plaintext secret values, even in debug mode.

### Preferred storage hierarchy

1. External KMS or Vault-backed secret source.
2. External Secrets or `SecretPolicy`-driven synchronization into Kubernetes.
3. Kubernetes Secrets only as the final delivery mechanism to pods.

### Example: KMS-backed `SecretPolicy`

```yaml
apiVersion: stellar.org/v1alpha1
kind: SecretPolicy
metadata:
  name: validator-seed-policy
  namespace: stellar-mainnet
spec:
  secretName: validator-seed
  provider: aws
  aws:
    keyId: "arn:aws:kms:us-east-1:123456789:key/11111111-2222-3333-4444-555555555555"
    region: us-east-1
  rotation:
    interval: "720h"
    zeroDowntime: true
    versionRetention: 5
  sync:
    targetClusters: ["cluster-dr"]
    syncInterval: "5m"
  audit:
    enabled: true
    anomalyDetection: true
  encryptInTransit: true
```

### Example: database secret rotation

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: horizon-mainnet
  namespace: stellar-mainnet-horizon
spec:
  nodeType: Horizon
  network: Mainnet
  database:
    host: postgres.stellar-mainnet-horizon.svc.cluster.local
    port: 5432
    database: horizon
    user: horizon
    passwordSecret: horizon-db-credentials
  secretRotation:
    enabled: true
    schedule: "0 0 1 */3 *"
    passwordLength: 40
    auditLoggingEnabled: true
```

### Validator seed recommendations

- Use a dedicated secret per validator, not a shared seed across multiple nodes.
- Grant read access only to the workload service account that needs the seed.
- Store seed material separately from database credentials and TLS keys.
- Rotate immediately if any pod, node, CI job, or support channel may have exposed the value.

### Secret review checklist

- etcd encryption at rest enabled.
- External secret source uses IAM, workload identity, or Vault auth instead of static root credentials.
- Secret readers are limited by namespace and service account.
- Rotation schedule is documented for each secret class.
- Secret access is audited.
- Backup copies are encrypted and access-controlled.

## RBAC Configuration Examples

RBAC should be designed so that each actor has only the permissions required for its function.

### 1. Namespace-scoped operator deployment

Prefer a namespace-scoped operator installation whenever cluster-wide watch is not required:

```yaml
watchNamespace: "stellar-mainnet"
```

This allows the chart to render a namespaced `Role` for core resources while keeping the cluster-scoped namespace read permission separate.

### 2. Minimal namespace reader

The repository already separates namespace label reads into a dedicated cluster role. That is the correct pattern for production:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: stellar-operator-namespace-reader
rules:
  - apiGroups: [""]
    resources: ["namespaces"]
    verbs: ["get", "list"]
```

### 3. Example operator role for a single namespace

Use the chart-generated role as the baseline and review it before apply time. A representative namespace-scoped profile looks like this:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: stellar-operator
  namespace: stellar-mainnet
rules:
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes", "stellarbenchmarks", "benchmarkreports"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["pods", "pods/log", "services", "configmaps", "persistentvolumeclaims"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "list", "watch"]
  - apiGroups: ["apps"]
    resources: ["deployments", "statefulsets"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["networking.k8s.io"]
    resources: ["networkpolicies"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  - apiGroups: ["coordination.k8s.io"]
    resources: ["leases"]
    verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

Production review points:

- Keep `secrets` read-only unless a dedicated rotation path requires write access.
- Use a different service account for supporting jobs instead of broadening the operator role.
- Do not bind cluster-admin to the operator.

### 4. Example REST API auditor role

The API auth layer supports Kubernetes token validation plus custom `admin`, `operate`, and `audit` verb checks against `stellarnodes`. If the REST API is enabled, create explicit roles instead of reusing admin accounts:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: stellar-api-auditor
  namespace: stellar-system
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: stellar-api-auditor
rules:
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes"]
    verbs: ["get", "list", "watch", "audit"]
  - apiGroups: [""]
    resources: ["pods", "services", "events"]
    verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: stellar-api-auditor
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: stellar-api-auditor
subjects:
  - kind: ServiceAccount
    name: stellar-api-auditor
    namespace: stellar-system
```

### 5. RBAC governance checklist

- Review every `ClusterRoleBinding` in production monthly.
- Alert on changes to RBAC and CRD definitions.
- Restrict who can label namespaces because namespace labels drive isolation policy.
- Require peer review for any permission expansion involving `secrets`, `namespaces`, `pods/ephemeralcontainers`, or `nodes`.

## Audit Logging Setup

Production-grade auditing should include both Kubernetes control-plane auditing and operator-level activity auditing.

### Layer 1: Kubernetes API server audit logs

Enable API server audit logging so there is an immutable record of:

- Secret reads and updates.
- Role and RoleBinding changes.
- Namespace label changes.
- `StellarNode`, `SecretPolicy`, and CRD changes.
- Admission webhook rejections and security-relevant write attempts.

Example audit policy:

```yaml
apiVersion: audit.k8s.io/v1
kind: Policy
rules:
  - level: Metadata
    resources:
      - group: "rbac.authorization.k8s.io"
        resources: ["roles", "rolebindings", "clusterroles", "clusterrolebindings"]
      - group: "stellar.org"
        resources: ["stellarnodes", "secretpolicies", "stellarbenchmarks", "benchmarkreports"]
      - group: ""
        resources: ["secrets", "namespaces"]
  - level: RequestResponse
    verbs: ["create", "update", "patch", "delete"]
    resources:
      - group: "stellar.org"
        resources: ["stellarnodes", "secretpolicies"]
  - level: Metadata
    omitStages:
      - RequestReceived
```

Operational guidance:

- Ship audit logs to long-retention object storage or a SIEM.
- Apply immutable retention where supported.
- Limit who can read full audit payloads because request bodies can contain sensitive metadata.

### Layer 2: operator audit configuration

The operator configuration schema supports an `audit` section loaded from `/etc/stellar-operator/config.yaml`. Use that file to enable persistence for operator-side audit events.

Example:

```yaml
defaultResources:
  validator:
    requests:
      cpu: "500m"
      memory: "1Gi"
    limits:
      cpu: "2"
      memory: "4Gi"
  horizon:
    requests:
      cpu: "250m"
      memory: "512Mi"
    limits:
      cpu: "2"
      memory: "4Gi"
  sorobanRpc:
    requests:
      cpu: "500m"
      memory: "2Gi"
    limits:
      cpu: "4"
      memory: "8Gi"
audit:
  enabled: true
  s3:
    bucket: "stellar-prod-audit-logs"
    prefix: "cluster-a/"
    region: "us-east-1"
    objectLock: true
```

This supports the repo's current audit sink pattern:

- In-memory bounded audit log for recent activity.
- Optional durable persistence to S3.
- Structured JSON records suitable for SIEM ingestion.

### Operator audit events to retain

The codebase records or exposes audit-relevant activity for operations such as:

- Node create, update, delete, suspend, and resume.
- Forensic snapshot requests.
- Manual maintenance triggers.
- DR drill or restore actions.
- CVE patch triggers.
- Config and webhook registration changes.
- Secret encryption, rotation, sync, and anomaly events.

### Audit retention recommendations

- Hot searchable retention: at least 90 days.
- Cold immutable retention: at least 1 year, or longer if compliance requires it.
- Time synchronization: ensure cluster nodes, S3 timestamps, and SIEM infrastructure use NTP.
- Access control: separate read access for responders from write access for pipeline components.

## Compliance Checklist

Use this checklist as the minimum production readiness gate.

| Control Area | Requirement | Status |
| --- | --- | --- |
| Cluster hardening | Kubernetes version supported and patched | [ ] |
| Cluster hardening | etcd encryption at rest enabled | [ ] |
| Cluster hardening | API server audit logging enabled | [ ] |
| Namespace isolation | Mainnet/Testnet/custom networks isolated by namespace | [ ] |
| Namespace isolation | `stellar.org/network` labels applied and protected | [ ] |
| Workload security | PSS `restricted` enforced on all Stellar namespaces | [ ] |
| Workload security | Pods run non-root with seccomp and dropped capabilities | [ ] |
| Workload security | Images pinned by digest and sourced from approved registries | [ ] |
| Network security | Helm `networkIsolation.enabled=true` in production | [ ] |
| Network security | Per-node `spec.networkPolicy.enabled=true` on all nodes | [ ] |
| Network security | Monitoring namespace is not shared with untrusted workloads | [ ] |
| Secrets | Validator seeds stored outside Git and delivered via approved secret backend | [ ] |
| Secrets | Database and API credentials rotated on a defined schedule | [ ] |
| Secrets | Secret access reviewed and audited | [ ] |
| RBAC | Operator uses namespace-scoped watch where feasible | [ ] |
| RBAC | No cluster-admin binding for the operator | [ ] |
| RBAC | API roles split into reader/operator/admin/auditor personas | [ ] |
| Audit | Operator audit persistence enabled | [ ] |
| Audit | Audit logs shipped to immutable or append-only storage | [ ] |
| Monitoring | `monitoring/security-alerts.yaml` deployed and routed | [ ] |
| Monitoring | Security dashboards imported and reviewed by on-call staff | [ ] |
| Supply chain | Trivy and Checkov scans required before release | [ ] |
| Supply chain | Dependency advisories reviewed and triaged | [ ] |
| Incident response | Security contact path from `SECURITY.md` documented internally | [ ] |
| Incident response | Response templates stored and exercised in a drill | [ ] |

### Evidence sources

Use the following evidence for internal reviews or external audits:

- `docs/security/pss.md` for pod hardening posture.
- `docs/network-isolation.md` for isolation control design.
- `docs/secret-management-kms.md` and `docs/secret-rotation.md` for secret lifecycle controls.
- `docs/compliance-reporting.md` for supported framework reporting.
- `monitoring/security-alerts.yaml` and `monitoring/grafana-security.json` for runtime monitoring evidence.
- `.github/workflows/security-scan.yml` for CI scanning evidence.
- `SECURITY.md` for disclosure process and scanning posture.

## Security Scanning Procedures

Security scanning should happen before merge, before release, and on a production cadence for drift detection.

### 1. CI security scans

The repository already defines CI security scanning in `.github/workflows/security-scan.yml`.

Required CI gates:

- Trivy filesystem scan for source and dependency issues.
- Trivy container image scan for built operator images.
- Checkov scan for Helm and Kubernetes manifests.
- SARIF upload into GitHub Security for review and retention.

Recommended release policy:

- Block release on unresolved `CRITICAL` findings.
- Require explicit review for `HIGH` findings.
- Track accepted risk in a dated exception record with owner and expiry.

### 2. Dependency and image scanning

Use the existing repo guidance from `docs/SECURITY_TESTING.md` and `SECURITY.md` to run:

- `cargo audit` for Rust dependency advisories.
- Trivy or Grype for image and filesystem scans.
- SBOM generation for release artifacts.

Procedure:

1. Scan source dependencies after every dependency update.
2. Scan the final container image built from the production Dockerfile.
3. Re-scan released images on a recurring schedule because new CVEs appear after release.
4. Open or link findings in the security tracking workflow before promotion.

### 3. Manifest and cluster configuration scanning

For Kubernetes and Helm assets:

- Run Checkov against `charts/`.
- Run `kube-bench` against production-like clusters to measure CIS alignment.
- Run `kube-score` or equivalent to identify weak manifest posture.
- Review Gatekeeper or Kyverno policies for drift against expected controls.

### 4. Dynamic and runtime scanning

Use controlled non-production or maintenance windows for active scans:

- ZAP baseline scans for the operator REST API if exposed.
- Nuclei scans using `security/tests/nuclei-templates/`.
- k6 security or abuse scenarios to evaluate resiliency and rate limiting.
- Runtime alert validation using `monitoring/security-alerts.yaml`.

### 5. Scan triage workflow

For every finding:

1. Classify it as code, image, dependency, manifest, or runtime.
2. Determine whether the affected feature is enabled in production.
3. Map impact to confidentiality, integrity, availability, and key management.
4. Define mitigation: patch, configuration change, compensating control, or documented acceptance.
5. Record owner, due date, and verification evidence.

### 6. Production scanning cadence

Recommended minimum cadence:

- Per pull request: source, image, and IaC scans.
- Weekly: dependency advisory review and image re-scan.
- Monthly: CIS and policy drift review.
- Quarterly: penetration-style validation and incident response drill.

## Incident Response Templates

Use the templates below during a security incident. Keep them in your internal runbooks, ticketing platform, or on-call workspace.

### 1. Security incident intake template

```text
Incident ID:
Reported by:
Report channel:
Date/time detected (UTC):
Environment:
Affected namespace(s):
Affected component(s):
Initial severity:
Observed symptom:
Potential security impact:
Immediate containment taken:
Ticket / bridge / chat link:
```

### 2. Initial triage checklist

```text
[ ] Confirm whether Mainnet, Testnet, or both are affected
[ ] Identify impacted namespaces, nodes, and external endpoints
[ ] Check recent changes to RBAC, Secrets, NetworkPolicies, and StellarNode specs
[ ] Preserve logs, events, and relevant manifests before making broad changes
[ ] Determine whether keys, credentials, or seed material may be exposed
[ ] Decide whether to suspend automation, rotate secrets, or isolate namespaces
[ ] Notify the security lead and platform owner
```

### 3. Containment template

```text
Containment owner:
Containment start time (UTC):
Scope to isolate:
Containment actions:
- Block ingress/egress
- Scale down affected workloads
- Revoke API tokens
- Rotate credentials
- Remove compromised node from service
Business impact accepted during containment:
Validation steps:
```

### 4. Evidence collection template

```text
Evidence custodian:
Collection start time (UTC):
Namespaces collected:
Artifacts:
- kubectl get/describe output
- Relevant audit log objects
- Operator logs
- API server audit events
- Secret and RBAC change history
- NetworkPolicy manifests
- Container image digest and deployment version
Storage location:
Hash / integrity method:
Access restrictions:
```

### 5. Internal communication template

```text
Subject: [Security Incident] <summary> - <severity>

Status:
Started:
Impacted environments:
Customer or network impact:
What we know:
What we are doing:
What we need from stakeholders:
Next update time:
Incident commander:
```

### 6. Secret rotation decision template

```text
Secret class:
Reason for rotation:
Systems using the secret:
Rotation method:
Downtime expected:
Rollback plan:
Verification owner:
Completion time (UTC):
```

### 7. Recovery checklist

```text
[ ] Root cause identified or bounded
[ ] Compromised credentials rotated
[ ] Affected images or manifests replaced
[ ] NetworkPolicy and RBAC restored to approved baseline
[ ] Monitoring confirms healthy reconciliation and service recovery
[ ] Audit trail preserved
[ ] Follow-up tasks captured with owners and deadlines
```

### 8. Post-incident report template

Use `docs/incident-response/post-mortem.md` as the formal write-up template after stabilization.

## Go-Live Hardening Checklist

Before production cutover, verify all of the following:

- Dedicated namespaces exist for operator, workloads, and monitoring.
- Pod Security labels are enforced before the first production rollout.
- Network isolation policies are applied and tested.
- Every production `StellarNode` enables `spec.networkPolicy`.
- Validator seeds and database passwords come from approved secret backends.
- Operator service account privileges are reviewed and minimized.
- API server and operator audit logging are enabled and retained.
- Security scans and compliance checks are integrated into release approval.
- Incident response contacts and templates are available to the on-call team.

## Related References

- `src/controller/pss.rs`
- `src/controller/network_isolation.rs`
- `src/controller/audit_log.rs`
- `src/controller/audit_sink.rs`
- `src/controller/operator_config.rs`
- `src/rest_api/auth.rs`
- `charts/stellar-operator/templates/rbac.yaml`
- `charts/stellar-operator/templates/network-isolation.yaml`
- `.github/workflows/security-scan.yml`
