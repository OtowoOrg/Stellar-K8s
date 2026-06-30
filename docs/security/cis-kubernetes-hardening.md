# CIS Kubernetes Benchmark Hardening

> Baseline controls for managed and self-hosted Kubernetes clusters running Stellar-K8s.

---

## 1) API Server Hardening Flags

| Control | Recommended Flag | Target |
|---|---|---|
| Anonymous auth disabled | `--anonymous-auth=false` | kube-apiserver |
| Audit logging enabled | `--audit-policy-file=/etc/kubernetes/audit-policy.yaml` | kube-apiserver |
| Strong TLS | `--tls-min-version=VersionTLS12` | kube-apiserver |
| Encrypt secrets | `--encryption-provider-config=/etc/kubernetes/encryption-config.yaml` | kube-apiserver |
| Bound SA token volume | `--service-account-issuer`, `--service-account-signing-key-file` | kube-apiserver |

---

## 2) Kubelet Hardening

| Control | Kubelet Config | Recommended Value |
|---|---|---|
| Disable read-only port | `readOnlyPort` | `0` |
| Protect kernel defaults | `protectKernelDefaults` | `true` |
| Webhook authn/authz | `authentication.webhook.enabled`, `authorization.mode` | `true`, `Webhook` |
| Restrict cert bootstrap scope | Node bootstrap RBAC | Tight node CSR permissions |

```yaml
# kubelet-config-secure.yaml
apiVersion: kubelet.config.k8s.io/v1beta1
kind: KubeletConfiguration
readOnlyPort: 0
protectKernelDefaults: true
serverTLSBootstrap: true
authentication:
  anonymous:
    enabled: false
  webhook:
    enabled: true
authorization:
  mode: Webhook
```

---

## 3) etcd Encryption at Rest Pattern

```yaml
# encryption-config.yaml
apiVersion: apiserver.config.k8s.io/v1
kind: EncryptionConfiguration
resources:
  - resources:
      - secrets
      - configmaps
    providers:
      - aescbc:
          keys:
            - name: key1
              secret: REPLACE_WITH_BASE64_32_BYTE_KEY
      - identity: {}
```

Rollout steps:

1. Place file on all control-plane nodes.
2. Restart API server with encryption flag.
3. Re-encrypt existing resources:

```bash
kubectl get secrets --all-namespaces -o json | kubectl replace -f -
```

---

## 4) Policy as Code Enforcement

```yaml
apiVersion: constraints.gatekeeper.sh/v1beta1
kind: K8sPSPPrivilegedContainer
metadata:
  name: deny-privileged
spec:
  match:
    excludedNamespaces: ["kube-system", "gatekeeper-system"]
    kinds:
      - apiGroups: [""]
        kinds: ["Pod"]
```

## 5) Verification Commands

```bash
kubectl get --raw /metrics | grep apiserver_envelope_encryption
kubectl -n kube-system get cm kubelet-config -o yaml
kubectl auth can-i create pods --as system:anonymous
```

Expected:

- Envelope encryption metrics present and increasing over time.
- Anonymous API access denied.
- Kubelet read-only endpoint disabled on all nodes.
