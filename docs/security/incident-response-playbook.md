# Incident Response Playbook

> Playbook for suspected compromise involving Stellar validator or RPC infrastructure in Kubernetes.

---

## 1) Containment Priority Matrix

| Severity | Scenario | Immediate Action |
|---|---|---|
| Sev-1 | Validator pod compromise, key exposure risk | Isolate namespace, remove from quorum slice, rotate credentials |
| Sev-2 | RPC abuse/DDoS path | Cut ingress path, enforce rate limits, preserve forensic logs |
| Sev-3 | Misconfiguration exposure | Apply policy fix, verify blast radius, monitor |

---

## 2) Quarantine Workflow for Compromised Validator Pod

1. Freeze automation on affected namespace to prevent uncontrolled reconciliation drift.
2. Add emergency deny policy for pod egress except forensic sink.
3. Remove validator from active quorum declarations.
4. Snapshot PVC and capture runtime state before termination.

### Emergency Cilium Quarantine Policy

```yaml
apiVersion: cilium.io/v2
kind: CiliumNetworkPolicy
metadata:
  name: quarantine-validator
  namespace: stellar-validator
spec:
  endpointSelector:
    matchLabels:
      app.kubernetes.io/component: validator
      security.stellar.io/quarantine: "true"
  ingress:
    - {}
  egress:
    - toEntities:
        - host
```

### Emergency Calico Quarantine Policy

```yaml
apiVersion: projectcalico.org/v3
kind: NetworkPolicy
metadata:
  name: quarantine-validator
  namespace: stellar-validator
spec:
  selector: security.stellar.io/quarantine == "true"
  types: [Ingress, Egress]
  ingress:
    - action: Deny
  egress:
    - action: Deny
```

---

## 3) Audit Logging Policy (kube-apiserver)

```yaml
apiVersion: audit.k8s.io/v1
kind: Policy
rules:
  - level: Metadata
    resources:
      - group: ""
        resources: ["secrets", "serviceaccounts", "configmaps"]
    verbs: ["create", "update", "patch", "delete", "get", "list"]
  - level: RequestResponse
    resources:
      - group: "stellar.org"
        resources: ["stellarnodes", "stellarquorums"]
    verbs: ["create", "update", "patch", "delete"]
  - level: Metadata
    resources:
      - group: "rbac.authorization.k8s.io"
        resources: ["roles", "rolebindings", "clusterroles", "clusterrolebindings"]
  - level: None
    users: ["system:kube-proxy"]
    verbs: ["watch"]
    resources:
      - group: ""
        resources: ["endpoints", "services"]
```

---

## 4) Forensic Capture Commands

```bash
kubectl get pods -n stellar-validator -o wide
kubectl describe pod <pod-name> -n stellar-validator
kubectl logs <pod-name> -n stellar-validator --previous
kubectl cp stellar-validator/<pod-name>:/var/lib/stellar-core ./forensics/<pod-name>/ledger
crictl ps -a | grep stellar
crictl inspect <container-id>
```

### Expected Output Shape

- `kubectl describe pod`: clear event chronology with image digest and restart history.
- `crictl inspect`: runtime-level mounts, seccomp, capabilities, and process metadata.
- Audit stream: API actor identity, verb, object reference, and request URI.

## 5) Recovery Exit Criteria

- Compromised signing or TLS material rotated and attested.
- Quorum declarations updated and verified healthy.
- Policy drift remediated and validated by conformance checks.
- Post-incident review completed with corrective actions tracked.
