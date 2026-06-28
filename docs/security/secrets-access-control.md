# Secrets Rotation and Least-Privilege Access Control

> This runbook provides secure credential lifecycle management for Stellar validators, Horizon/RPC services, and operator control loops.

---

## 1) Vault Rotation Runbook (TLS + Signing Adjacent Secrets)

### Workflow

| Step | Action | Command/Config |
|---|---|---|
| 1 | Issue short-lived certs | Vault PKI role with 24h TTL |
| 2 | Inject into pods | Vault Agent Injector annotations |
| 3 | Reload workloads | SIGHUP or rolling restart with surge |
| 4 | Verify trust chain | OpenSSL and app-level health checks |

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: validator
  namespace: stellar-validator
  annotations:
    vault.hashicorp.com/agent-inject: "true"
    vault.hashicorp.com/role: "stellar-validator"
    vault.hashicorp.com/agent-inject-secret-tls.crt: "pki_int/issue/stellar-validator"
    vault.hashicorp.com/agent-inject-template-tls.crt: |
      {{- with secret "pki_int/issue/stellar-validator" "common_name=validator.stellar-validator.svc" -}}
      {{ .Data.certificate }}
      {{- end -}}
```

---

## 2) cert-manager Rotation Cycle

```yaml
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: validator-mtls
  namespace: stellar-validator
spec:
  secretName: validator-mtls
  duration: 2160h
  renewBefore: 360h
  commonName: validator.stellar-validator.svc
  dnsNames:
    - validator.stellar-validator.svc
  issuerRef:
    name: stellar-ca-issuer
    kind: ClusterIssuer
```

Runbook checkpoints:

- Alert when certificate expiration < 14 days.
- Ensure `renewBefore` supports safe rollout windows.
- Verify peer TLS handshakes during renewal by canarying one pod first.

---

## 3) RBAC Least-Privilege Templates

### Namespace Role for Operator Reconciliation

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: stellar-operator-namespace-role
  namespace: stellar-validator
rules:
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes", "stellarnodes/status", "stellarquorums"]
    verbs: ["get", "list", "watch", "patch", "update"]
  - apiGroups: [""]
    resources: ["configmaps", "events", "services", "persistentvolumeclaims"]
    verbs: ["get", "list", "watch", "create", "patch", "update"]
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "list", "watch"]
```

### ClusterRole for Leader Election + CRD Watch

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: stellar-operator-clusterrole
rules:
  - apiGroups: ["coordination.k8s.io"]
    resources: ["leases"]
    verbs: ["get", "list", "watch", "create", "update", "patch"]
  - apiGroups: ["apiextensions.k8s.io"]
    resources: ["customresourcedefinitions"]
    verbs: ["get", "list", "watch"]
  - apiGroups: ["stellar.org"]
    resources: ["stellarnodes"]
    verbs: ["get", "list", "watch"]
```

### Audit Access Quickly

```bash
kubectl auth can-i get secrets -n stellar-validator --as system:serviceaccount:stellar-system:stellar-operator
kubectl auth can-i create clusterrolebindings --as system:serviceaccount:stellar-system:stellar-operator
```

Expected:

- First command allowed only if read of required secrets is intentional.
- Second command must be denied.
