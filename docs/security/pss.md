# Pod Security Standards (PSS) — Zero-Trust Enforcement

Stellar-K8s enforces the Kubernetes **`restricted`** Pod Security Standard across every namespace it manages. This document describes the security posture, what is enforced, and how to stay compliant.

## Overview

The [Kubernetes Pod Security Standards](https://kubernetes.io/docs/concepts/security/pod-security-standards/) define three profiles:

| Profile | Description |
|---------|-------------|
| `privileged` | Unrestricted — no constraints |
| `baseline` | Prevents known privilege escalations |
| `restricted` | Hardened — follows current pod hardening best practices |

Stellar-K8s uses **`restricted`** for all managed namespaces. This is the highest security level and is appropriate for financial infrastructure where workload isolation is critical.

---

## Namespace Labeling

On every reconcile, the operator applies the following labels to the namespace containing the `StellarNode`:

```yaml
pod-security.kubernetes.io/enforce: restricted
pod-security.kubernetes.io/enforce-version: latest
pod-security.kubernetes.io/warn: restricted
pod-security.kubernetes.io/warn-version: latest
pod-security.kubernetes.io/audit: restricted
pod-security.kubernetes.io/audit-version: latest
```

This is implemented in `src/controller/pss.rs` via `ensure_namespace_pss_labels()`, called idempotently on every reconcile. The patch is a server-side apply, so existing namespace labels are preserved.

---

## Pod Security Context

Every pod created by the operator includes:

```yaml
securityContext:
  runAsNonRoot: true
  runAsUser: 10000
  runAsGroup: 10000
  fsGroup: 10000
  seccompProfile:
    type: RuntimeDefault
```

- `runAsNonRoot: true` — the container runtime rejects any image that would run as UID 0.
- `runAsUser/runAsGroup/fsGroup: 10000` — explicit non-root UID/GID for all processes and volume ownership.
- `seccompProfile: RuntimeDefault` — applies the container runtime's default seccomp filter, blocking ~300 dangerous syscalls.

---

## Container Security Context

Every container (main, init, and sidecar) includes:

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

| Setting | Why |
|---------|-----|
| `allowPrivilegeEscalation: false` | Prevents `setuid`/`setgid` binaries from gaining elevated privileges |
| `privileged: false` | Prevents access to host devices and kernel features |
| `readOnlyRootFilesystem: true` | Prevents runtime modification of the container filesystem |
| `capabilities.drop: ["ALL"]` | Removes all Linux capabilities; none are added back for standard containers |
| `seccompProfile: RuntimeDefault` | Syscall filtering via the runtime's default profile |

### Containers covered

| Container | Type | PSS-compliant |
|-----------|------|:---:|
| `stellar-node` | Main | ✅ |
| `horizon-db-migration` | Init | ✅ |
| `kms-fetcher` | Init | ✅ |
| `cloudhsm-client` | Sidecar | ✅ |
| `dedicatedhsm-client` | Sidecar | ✅ |
| `nat-traversal` | Sidecar | ✅ |
| User-defined `spec.sidecars` | Sidecar | Validated at admission |

---

## Admission Webhook Validation

The admission webhook (`/validate` endpoint) rejects any `StellarNode` whose `spec.sidecars` attempt to bypass PSS constraints. The following are blocked:

| Field | Forbidden value | Reason |
|-------|----------------|--------|
| `securityContext.privileged` | `true` | Grants host-level access |
| `securityContext.allowPrivilegeEscalation` | `true` | Allows privilege escalation via setuid |
| `securityContext.capabilities.add` | `NET_ADMIN`, `SYS_ADMIN`, `SYS_PTRACE`, `NET_RAW`, `SYS_MODULE` | High-risk capabilities |
| `securityContext.seccompProfile.type` | `Unconfined` | Disables syscall filtering |
| `securityContext.runAsUser` | `0` | Root execution |

### Example rejection

```yaml
# This StellarNode will be rejected by the webhook
apiVersion: stellar.org/v1alpha1
kind: StellarNode
spec:
  sidecars:
    - name: debug
      image: busybox
      securityContext:
        privileged: true   # ← REJECTED
```

Rejection message:
```
PSS 'restricted' violation(s) detected — zero-trust policy forbids these settings:
spec.sidecars[0].securityContext.privileged: privileged containers are forbidden under PSS 'restricted'
```

---

## Forensic Snapshot Exception

The forensic snapshot ephemeral container (`stellar-k8s-forensic-*`) adds `NET_RAW` and `SYS_PTRACE` capabilities for PCAP capture and core dump collection. This is an intentional, documented exception:

- It is only injected on-demand via annotation (`stellar.org/request-forensic-snapshot: "true"`).
- It is an ephemeral container — it cannot be pre-scheduled or persisted.
- It requires explicit operator action and is audited via Kubernetes Events.
- It is **not** subject to the sidecar PSS validation because it is not part of `spec.sidecars`.

---

## Compliance Verification

To verify PSS labels are applied to a namespace:

```bash
kubectl get namespace stellar --show-labels
```

Expected output includes:
```
pod-security.kubernetes.io/enforce=restricted
pod-security.kubernetes.io/enforce-version=latest
```

To verify a pod's security context:

```bash
kubectl get pod <pod-name> -n stellar -o jsonpath='{.spec.securityContext}'
kubectl get pod <pod-name> -n stellar -o jsonpath='{.spec.containers[0].securityContext}'
```

To test that a privileged pod is rejected:

```bash
kubectl run test --image=busybox --privileged -n stellar
# Expected: Error from server (Forbidden): pods "test" is forbidden:
# violates PodSecurity "restricted:latest"
```

---

## Threat Model

| Threat | Mitigation |
|--------|-----------|
| Container escape via privileged mode | `privileged: false` enforced on all containers |
| Privilege escalation via setuid binary | `allowPrivilegeEscalation: false` |
| Lateral movement via host network | No `hostNetwork: true` permitted |
| Kernel exploit via dangerous syscall | `seccompProfile: RuntimeDefault` |
| Filesystem tampering at runtime | `readOnlyRootFilesystem: true` |
| Root process compromise | `runAsNonRoot: true` + explicit UID 10000 |
| Capability abuse | `capabilities.drop: ALL` |
| Namespace escape | PSS `restricted` enforced at namespace level |

---

## References

- [Kubernetes Pod Security Standards](https://kubernetes.io/docs/concepts/security/pod-security-standards/)
- [Kubernetes Pod Security Admission](https://kubernetes.io/docs/concepts/security/pod-security-admission/)
- [CIS Kubernetes Benchmark](https://www.cisecurity.org/benchmark/kubernetes)
- `src/controller/pss.rs` — implementation
- `src/webhook/server.rs` — admission validation
