# Network Policy Templates

This guide collects reusable Kubernetes `NetworkPolicy` templates for the most common Stellar security scenarios. The templates are intentionally small and composable so you can adapt them to validator namespaces, API namespaces, and shared services without rewriting the policy model each time.

Use these templates together with the [Network Isolation Guide](./network-isolation.md) and the generated per-node policies from the operator.

## 1. Validator Isolation

Use this template for validator namespaces where you want to keep peer traffic, admin traffic, and metrics traffic inside the same Stellar network boundary.

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: validator-isolation
  namespace: <validator-namespace>
  labels:
    app.kubernetes.io/managed-by: stellar-operator
    stellar.org/policy-template: validator-isolation
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: stellar-node
      app.kubernetes.io/component: validator
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              stellar.org/network: <network>
      ports:
        - protocol: TCP
          port: 11625
        - protocol: TCP
          port: 11626
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: monitoring
      ports:
        - protocol: TCP
          port: 9090
  egress:
    - to:
        - namespaceSelector:
            matchLabels:
              stellar.org/network: <network>
      ports:
        - protocol: TCP
          port: 11625
        - protocol: TCP
          port: 11626
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
```

## 2. API Endpoint Protection

Use this template for Horizon, Soroban RPC, or other HTTP APIs that should only be reachable from the ingress controller, approved namespaces, or a small set of external CIDRs.

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: api-endpoint-protection
  namespace: <api-namespace>
  labels:
    app.kubernetes.io/managed-by: stellar-operator
    stellar.org/policy-template: api-endpoint-protection
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: horizon
  policyTypes:
    - Ingress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: ingress-nginx
      ports:
        - protocol: TCP
          port: 8000
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: <trusted-namespace>
      ports:
        - protocol: TCP
          port: 8000
    - from:
        - ipBlock:
            cidr: <approved-cidr>
      ports:
        - protocol: TCP
          port: 8000
```

If you expose a validator admin endpoint instead of Horizon, change the port to `11626` and keep the allow-list as small as possible.

## 3. Database Access Control

Use this template for PostgreSQL, PgBouncer, or any shared database where only a narrow set of application pods should connect.

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: database-access-control
  namespace: <database-namespace>
  labels:
    app.kubernetes.io/managed-by: stellar-operator
    stellar.org/policy-template: database-access-control
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: postgres
  policyTypes:
    - Ingress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: <application-namespace>
          podSelector:
            matchLabels:
              app.kubernetes.io/name: stellar-node
      ports:
        - protocol: TCP
          port: 5432
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: <application-namespace>
          podSelector:
            matchLabels:
              app.kubernetes.io/name: pgbouncer
      ports:
        - protocol: TCP
          port: 5432
```

## 4. Cross-Namespace Communication

Use this template when a namespace must talk to a limited set of other namespaces while still honoring the Stellar network label as the main trust boundary.

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: cross-namespace-communication
  namespace: <source-namespace>
  labels:
    app.kubernetes.io/managed-by: stellar-operator
    stellar.org/policy-template: cross-namespace-communication
spec:
  podSelector: {}
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              stellar.org/network: <network>
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: <trusted-namespace>
  egress:
    - to:
        - namespaceSelector:
            matchLabels:
              stellar.org/network: <network>
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: <trusted-namespace>
```

## 5. External Access Control

Use this template when a public API or a load balancer must accept traffic only from approved external networks.

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: external-access-control
  namespace: <public-service-namespace>
  labels:
    app.kubernetes.io/managed-by: stellar-operator
    stellar.org/policy-template: external-access-control
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: horizon
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - from:
        - ipBlock:
            cidr: <approved-external-cidr>
      ports:
        - protocol: TCP
          port: 8000
    - from:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: ingress-nginx
      ports:
        - protocol: TCP
          port: 8000
  egress:
    - to:
        - ipBlock:
            cidr: <history-archive-cidr>
      ports:
        - protocol: TCP
          port: 443
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
      ports:
        - protocol: UDP
          port: 53
        - protocol: TCP
          port: 53
```

## Customization Guide

Replace these placeholders before applying a template:

| Placeholder | Meaning |
|---|---|
| `<network>` | The Stellar network label value, such as `mainnet`, `testnet`, `futurenet`, or a custom hash-derived label |
| `<validator-namespace>` | Namespace that hosts validator pods |
| `<api-namespace>` | Namespace that hosts Horizon, Soroban RPC, or another API workload |
| `<database-namespace>` | Namespace that hosts the database or PgBouncer |
| `<source-namespace>` | Namespace where the policy is applied |
| `<trusted-namespace>` | Another namespace that is explicitly allowed |
| `<approved-cidr>` | A CIDR range that may reach the API endpoint |
| `<approved-external-cidr>` | A CIDR range that may reach a public service |
| `<history-archive-cidr>` | A CIDR range for an external archive or upstream dependency |

Recommended customization rules:

1. Start with the narrowest `podSelector` you can use.
2. Prefer `namespaceSelector` over `ipBlock` when traffic stays inside the cluster.
3. Keep DNS egress open anywhere a policy would otherwise block service discovery.
4. Add monitoring ingress only from the monitoring namespace you actually run.
5. Separate validator, API, and database policies instead of combining unrelated rules into a single object.

## Testing Guide

Use this workflow after applying a policy template:

1. Confirm the policy is present.

```bash
kubectl get networkpolicy -n <namespace>
kubectl describe networkpolicy <policy-name> -n <namespace>
```

2. Verify the labels that the policy depends on.

```bash
kubectl get namespace <namespace> --show-labels
kubectl get pods -n <namespace> --show-labels
```

3. Test an allowed connection from a debug pod.

```bash
kubectl run -it --rm netshoot --image=nicolaka/netshoot --restart=Never -n <namespace> -- \
  nc -zv <target-service> 8000
```

4. Test a blocked connection from a disallowed namespace or CIDR.

```bash
kubectl run -it --rm netshoot --image=nicolaka/netshoot --restart=Never -n <other-namespace> -- \
  nc -zv <target-service> 8000
```

5. For HTTP services, confirm the response is reachable only from the approved path.

```bash
kubectl run -it --rm curl-test --image=curlimages/curl --restart=Never -n <namespace> -- \
  curl -sv http://<service-name>:8000/
```

Expected result:

- Allowed paths should connect successfully.
- Disallowed paths should time out, reset, or fail DNS resolution, depending on the CNI implementation.

## Troubleshooting Tips

- If a policy appears to do nothing, check that the `podSelector` matches at least one pod.
- If all traffic is blocked, make sure the policy still allows DNS egress on port 53.
- If namespace-based allow rules fail, confirm that the target namespace carries the expected `stellar.org/network` label.
- If an ingress controller cannot reach the service, verify the controller namespace label and the Service selector.
- If you are using a CNI that does not enforce standard `NetworkPolicy`, the templates will apply but traffic will not be blocked.
- If a database policy blocks legitimate traffic, check whether the application connects directly or through PgBouncer and allow the correct pod labels.
