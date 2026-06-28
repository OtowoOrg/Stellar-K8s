# Security Isolation and Service Mesh mTLS

> Stellar validator networks should be treated as high-trust, low-latency enclaves. Public RPC/API planes must remain isolated by policy and identity.

---

## 1) Calico GlobalNetworkPolicy Templates

### Validator East-West Allowlist

```yaml
apiVersion: projectcalico.org/v3
kind: GlobalNetworkPolicy
metadata:
  name: stellar-validator-eastwest
spec:
  order: 100
  selector: app.kubernetes.io/component == "validator"
  types: [Ingress, Egress]
  ingress:
    - action: Allow
      protocol: TCP
      source:
        selector: app.kubernetes.io/component == "validator"
      destination:
        ports: [11625]
    - action: Allow
      protocol: TCP
      source:
        namespaceSelector: kubernetes.io/metadata.name == "monitoring"
      destination:
        ports: [9100]
  egress:
    - action: Allow
      protocol: TCP
      destination:
        selector: app.kubernetes.io/component == "validator"
        ports: [11625]
    - action: Allow
      protocol: UDP
      destination:
        nets: [169.254.169.253/32]
        ports: [53]
  doNotTrack: false
  preDNAT: false
```

### Default Deny for Validator Namespace

```yaml
apiVersion: projectcalico.org/v3
kind: GlobalNetworkPolicy
metadata:
  name: stellar-validator-default-deny
spec:
  order: 200
  namespaceSelector: kubernetes.io/metadata.name == "stellar-validator"
  types: [Ingress, Egress]
  ingress:
    - action: Deny
  egress:
    - action: Deny
```

---

## 2) CiliumClusterwideNetworkPolicy Templates

```yaml
apiVersion: cilium.io/v2
kind: CiliumClusterwideNetworkPolicy
metadata:
  name: stellar-validator-policy
spec:
  endpointSelector:
    matchLabels:
      app.kubernetes.io/component: validator
  ingress:
    - fromEndpoints:
        - matchLabels:
            app.kubernetes.io/component: validator
      toPorts:
        - ports:
            - port: "11625"
              protocol: TCP
    - fromEndpoints:
        - matchLabels:
            k8s:io.kubernetes.pod.namespace: monitoring
      toPorts:
        - ports:
            - port: "9100"
              protocol: TCP
  egress:
    - toEndpoints:
        - matchLabels:
            app.kubernetes.io/component: validator
      toPorts:
        - ports:
            - port: "11625"
              protocol: TCP
    - toEntities:
        - kube-apiserver
```

---

## 3) mTLS Blueprint with Istio

### Architectural Pattern

| Layer | Control | Notes |
|---|---|---|
| Transport identity | SPIFFE SAN in workload certs | Cert rotation via Istiod |
| Traffic policy | PeerAuthentication strict mode | Reject plaintext lateral traffic |
| Authorization | AuthorizationPolicy by namespace + principal | Separate validator from RPC principals |

```yaml
apiVersion: security.istio.io/v1beta1
kind: PeerAuthentication
metadata:
  name: validator-strict-mtls
  namespace: stellar-validator
spec:
  mtls:
    mode: STRICT
---
apiVersion: security.istio.io/v1beta1
kind: AuthorizationPolicy
metadata:
  name: validator-allow-only-validator
  namespace: stellar-validator
spec:
  selector:
    matchLabels:
      app.kubernetes.io/component: validator
  rules:
    - from:
        - source:
            principals:
              - cluster.local/ns/stellar-validator/sa/validator
```

## 4) mTLS Blueprint with Linkerd

```yaml
apiVersion: policy.linkerd.io/v1alpha1
kind: Server
metadata:
  name: validator-server
  namespace: stellar-validator
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: validator
  port: 11625
---
apiVersion: policy.linkerd.io/v1alpha1
kind: ServerAuthorization
metadata:
  name: validator-only
  namespace: stellar-validator
spec:
  server:
    name: validator-server
  client:
    meshTLS:
      serviceAccounts:
        - name: validator
          namespace: stellar-validator
```

## 5) Operational Notes

- Exclude latency-critical validator ports from L7 processing when possible.
- Keep mesh sidecar resources pinned to avoid CPU throttling on consensus paths.
- Confirm mTLS policy behavior during certificate rotations and pod restarts.
