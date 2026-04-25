# Networking Troubleshooting Guide

This guide covers the most common networking failures when running Stellar nodes on Kubernetes: `Connection Refused`, `No Route to Host`, SCP handshake failures, and related issues. Work through the sections that match your symptoms.

---

## Quick Reference: Stellar Port Map

| Port  | Protocol | Purpose                                      |
|-------|----------|----------------------------------------------|
| 11625 | TCP      | Stellar Core P2P (peer-to-peer SCP traffic)  |
| 11626 | TCP      | Stellar Core HTTP admin / Horizon ingest URL |
| 8000  | TCP      | Horizon REST API                             |
| 9100  | TCP      | Prometheus metrics (if enabled)              |

---

## 1. Diagnosing "Connection Refused"

`Connection Refused` means the TCP connection reached the target host but nothing was listening on that port.

### 1.1 Is the pod running?

```bash
kubectl get pods -n <namespace> -l app.kubernetes.io/name=stellar-node
```

If the pod is in `CrashLoopBackOff` or `Error`, fix the pod first:

```bash
kubectl describe pod <pod-name> -n <namespace>
kubectl logs <pod-name> -n <namespace> --previous
```

### 1.2 Is the Service pointing at the right pods?

```bash
# Check endpoints — if empty, the selector doesn't match any pods
kubectl get endpoints <service-name> -n <namespace>

# Compare the Service selector with pod labels
kubectl get svc <service-name> -n <namespace> -o jsonpath='{.spec.selector}'
kubectl get pods -n <namespace> --show-labels
```

### 1.3 Is the container actually listening?

```bash
kubectl exec -n <namespace> <pod-name> -- ss -tlnp
# or
kubectl exec -n <namespace> <pod-name> -- netstat -tlnp 2>/dev/null || \
  kubectl exec -n <namespace> <pod-name> -- cat /proc/net/tcp
```

### 1.4 Test from inside the cluster

```bash
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -- \
  nc -zv <service-name>.<namespace>.svc.cluster.local 11625
```

---

## 2. Diagnosing "No Route to Host"

`No Route to Host` (EHOSTUNREACH / ENETUNREACH) means the packet never reached the destination — typically a NetworkPolicy, firewall, or CNI misconfiguration.

### 2.1 Check NetworkPolicies

```bash
# List all NetworkPolicies in the namespace
kubectl get networkpolicies -n <namespace>

# Describe each one to see ingress/egress rules
kubectl describe networkpolicy <name> -n <namespace>
```

A common mistake is creating a default-deny policy without an explicit allow for P2P traffic:

```yaml
# Allow inbound P2P from other validators
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-stellar-p2p
  namespace: <namespace>
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: stellar-validator
  policyTypes:
    - Ingress
    - Egress
  ingress:
    - ports:
        - port: 11625
          protocol: TCP
  egress:
    - ports:
        - port: 11625
          protocol: TCP
    - ports:
        - port: 53       # DNS
          protocol: UDP
    - ports:
        - port: 443      # HTTPS (for history archives)
          protocol: TCP
```

### 2.2 Temporarily disable NetworkPolicies for testing

```bash
# Delete the policy temporarily (restore it after testing!)
kubectl delete networkpolicy <name> -n <namespace>

# Re-test connectivity
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -- \
  nc -zv <target-ip> 11625
```

### 2.3 Check node-level firewall rules

On cloud providers, security groups / firewall rules are applied outside Kubernetes:

```bash
# AWS: check security group rules for the node
# GCP: check VPC firewall rules
# Azure: check NSG rules

# From a debug pod, test if the port is reachable at the node level
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -- \
  traceroute -T -p 11625 <target-pod-ip>
```

---

## 3. SCP Handshake Failures

SCP (Stellar Consensus Protocol) handshakes fail when validators can reach each other's TCP port but the application-level handshake is rejected.

### 3.1 Check Stellar Core logs for handshake errors

```bash
kubectl logs -n <namespace> <validator-pod> | grep -i "handshake\|peer\|scp\|flood\|drop"
```

Common log patterns and their causes:

| Log message | Cause |
|---|---|
| `Dropping peer: wrong network` | `NETWORK_PASSPHRASE` mismatch between peers |
| `Dropping peer: version too old` | Peer is running an incompatible Stellar Core version |
| `Dropping peer: flood gate` | Too many connections; peer is rate-limiting |
| `Failed to authenticate peer` | mTLS certificate mismatch or expired cert |
| `Connection reset by peer` | Firewall or NetworkPolicy dropping mid-handshake |

### 3.2 Verify network passphrase

All validators in a quorum set must use the same `NETWORK_PASSPHRASE`. Check the generated ConfigMap:

```bash
kubectl get configmap <node-name>-config -n <namespace> -o jsonpath='{.data.stellar-core\.cfg}' \
  | grep NETWORK_PASSPHRASE
```

### 3.3 Check peer list

```bash
# View the operator-managed peers ConfigMap
kubectl get configmap stellar-peers -n stellar-system -o jsonpath='{.data.peers\.json}' | jq

# Query Stellar Core's live peer list
kubectl exec -n <namespace> <validator-pod> -- \
  curl -s http://localhost:11626/peers | jq '.authenticated_peers'
```

### 3.4 Test P2P reachability between two validators

```bash
# From validator-1's pod, try to reach validator-2 on port 11625
kubectl exec -n <namespace> <validator-1-pod> -- \
  nc -zv <validator-2-service-ip> 11625
```

---

## 4. Ingress vs LoadBalancer

### 4.1 When to use each

| Scenario | Recommended approach |
|---|---|
| Horizon / Soroban RPC (HTTP/HTTPS) | Ingress (NGINX, Traefik) |
| Validator P2P (TCP port 11625) | LoadBalancer Service or MetalLB |
| Internal cluster traffic only | ClusterIP Service |

**Validators must not use Ingress** — Ingress controllers only handle HTTP/HTTPS (L7). Stellar Core P2P is raw TCP (L4). Use a `LoadBalancer` Service or MetalLB instead.

### 4.2 Diagnose Ingress issues

```bash
# Check Ingress resource
kubectl describe ingress <name> -n <namespace>

# Check ingress controller pods
kubectl get pods -n ingress-nginx
kubectl logs -n ingress-nginx -l app.kubernetes.io/name=ingress-nginx --tail=50

# Verify the backend Service and endpoints
kubectl get endpoints <backend-service> -n <namespace>

# Test from inside the cluster (bypasses external DNS)
kubectl run -it --rm curl-test --image=curlimages/curl --restart=Never -- \
  curl -v http://<ingress-service-ip>/
```

### 4.3 Diagnose LoadBalancer issues

```bash
# Check if external IP is assigned
kubectl get svc <service-name> -n <namespace>
# STATUS: <pending> means the cloud provider hasn't assigned an IP yet

# For MetalLB: check the address pool
kubectl get ipaddresspool -A
kubectl describe ipaddresspool <pool-name> -n metallb-system

# Check MetalLB speaker logs
kubectl logs -n metallb-system -l component=speaker --tail=50

# Verify the LoadBalancer IP is reachable from outside
curl -v telnet://<external-ip>:11625
```

### 4.4 Common Ingress annotation mistakes

```yaml
# WRONG: using Ingress for TCP (won't work)
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: validator-p2p   # ❌ Ingress cannot route raw TCP
spec:
  rules:
    - host: validator.example.com

# CORRECT: use a LoadBalancer Service for TCP
apiVersion: v1
kind: Service
metadata:
  name: validator-p2p
spec:
  type: LoadBalancer
  selector:
    app.kubernetes.io/name: stellar-node
  ports:
    - port: 11625
      targetPort: 11625
      protocol: TCP
```

---

## 5. CNI-Specific Issues

### 5.1 Calico

```bash
# Check Calico node status
kubectl get pods -n kube-system -l k8s-app=calico-node
kubectl exec -n kube-system <calico-node-pod> -- calicoctl node status

# Check GlobalNetworkPolicy (cluster-wide deny rules)
kubectl get globalnetworkpolicies.crd.projectcalico.org

# Trace a specific flow (requires calicoctl)
kubectl exec -n kube-system <calico-node-pod> -- \
  calicoctl node diags
```

Common Calico issue: a `GlobalNetworkPolicy` with `default-deny` blocks P2P before namespace-level policies apply. Add an explicit allow:

```yaml
apiVersion: projectcalico.org/v3
kind: GlobalNetworkPolicy
metadata:
  name: allow-stellar-p2p
spec:
  selector: app.kubernetes.io/name == 'stellar-node'
  types:
    - Ingress
    - Egress
  ingress:
    - action: Allow
      protocol: TCP
      destination:
        ports: [11625]
  egress:
    - action: Allow
      protocol: TCP
      destination:
        ports: [11625, 11626, 443]
    - action: Allow
      protocol: UDP
      destination:
        ports: [53]
```

### 5.2 Cilium

```bash
# Check Cilium agent status
kubectl get pods -n kube-system -l k8s-app=cilium
kubectl exec -n kube-system <cilium-pod> -- cilium status

# Monitor live traffic for a pod
kubectl exec -n kube-system <cilium-pod> -- \
  cilium monitor --type drop --from-pod <namespace>/<pod-name>

# Check CiliumNetworkPolicy
kubectl get ciliumnetworkpolicies -A
```

Cilium-specific: if you use `CiliumNetworkPolicy` instead of standard `NetworkPolicy`, ensure you have explicit egress rules for DNS (port 53) and history archive access (port 443).

### 5.3 Flannel / Weave

Flannel and Weave don't enforce NetworkPolicies natively. If you're using them and seeing drops, the issue is likely at the node firewall (iptables) or cloud security group level, not the CNI.

```bash
# Check iptables rules on the node (requires node access)
iptables -L -n -v | grep -E "11625|11626|DROP|REJECT"

# Check if kube-proxy is healthy
kubectl get pods -n kube-system -l k8s-app=kube-proxy
kubectl logs -n kube-system -l k8s-app=kube-proxy --tail=20
```

---

## 6. Stellar P2P Firewalling

### 6.1 Required outbound connections from validators

Validators need outbound access to:

| Destination | Port | Purpose |
|---|---|---|
| Other validators (cluster-internal) | 11625 TCP | SCP consensus |
| Other validators (external) | 11625 TCP | SCP consensus with external peers |
| History archive servers | 443 TCP | Ledger history sync |
| Kubernetes DNS | 53 UDP/TCP | Service name resolution |

### 6.2 Required inbound connections to validators

| Source | Port | Purpose |
|---|---|---|
| Other validators | 11625 TCP | Inbound P2P connections |
| Horizon pods | 11626 TCP | Horizon ingest (internal) |
| Operator pod | 11626 TCP | Health checks and config reload |

### 6.3 Horizon-specific requirements

Horizon needs:
- Outbound to Stellar Core on port 11626 (ingest URL)
- Inbound on port 8000 from Ingress controller or LoadBalancer
- Outbound to PostgreSQL on port 5432

### 6.4 Verify external P2P reachability

If your validator needs to peer with external validators (outside the cluster):

```bash
# From outside the cluster, test if port 11625 is reachable
nc -zv <external-ip-of-validator> 11625

# From inside the cluster, test outbound to a known Stellar validator
kubectl run -it --rm netdebug --image=nicolaka/netshoot --restart=Never -- \
  nc -zv stellar1.example.com 11625
```

---

## 7. DNS Resolution Issues

### 7.1 Test DNS from inside a pod

```bash
kubectl run -it --rm dnstest --image=busybox --restart=Never -- \
  nslookup <service-name>.<namespace>.svc.cluster.local

# Test external DNS
kubectl run -it --rm dnstest --image=busybox --restart=Never -- \
  nslookup history.stellar.org
```

### 7.2 Check CoreDNS

```bash
kubectl get pods -n kube-system -l k8s-app=kube-dns
kubectl logs -n kube-system -l k8s-app=kube-dns --tail=30

# Check CoreDNS ConfigMap for custom forwarders
kubectl get configmap coredns -n kube-system -o yaml
```

### 7.3 Common DNS failure: NetworkPolicy blocking port 53

If you have a default-deny egress policy, DNS will silently fail. Always include:

```yaml
egress:
  - ports:
      - port: 53
        protocol: UDP
      - port: 53
        protocol: TCP
```

---

## 8. mTLS / Certificate Issues

See [docs/mtls-guide.md](../mtls-guide.md) for full details. Quick checks:

```bash
# Check if the TLS secret exists and has the right keys
kubectl get secret <node-name>-client-cert -n <namespace> -o jsonpath='{.data}' | jq 'keys'
# Expected: ["ca.crt", "tls.crt", "tls.key"]

# Check certificate expiry
kubectl get secret <node-name>-client-cert -n <namespace> \
  -o jsonpath='{.data.tls\.crt}' | base64 -d | openssl x509 -noout -dates

# If using cert-manager, check Certificate status
kubectl get certificate -n <namespace>
kubectl describe certificate <node-name>-mtls-cert -n <namespace>
```

---

## 9. Useful One-Liners

```bash
# List all Services and their types in a namespace
kubectl get svc -n <namespace> -o wide

# Show all pods with their IPs
kubectl get pods -n <namespace> -o wide

# Watch events in real time (great for spotting network errors)
kubectl get events -n <namespace> --sort-by='.lastTimestamp' -w

# Check if a specific pod can reach the Kubernetes API
kubectl exec -n <namespace> <pod> -- \
  curl -sk https://kubernetes.default.svc.cluster.local/healthz

# Dump all NetworkPolicies across all namespaces
kubectl get networkpolicies -A -o yaml

# Check iptables NAT rules for a Service (run on the node)
iptables -t nat -L -n -v | grep <service-cluster-ip>
```

---

## 10. Escalation Checklist

Before opening a support issue, collect:

- [ ] `kubectl get stellarnodes -A -o yaml`
- [ ] `kubectl get pods -A -l app.kubernetes.io/name=stellar-node -o wide`
- [ ] `kubectl get svc -A -l app.kubernetes.io/name=stellar-node`
- [ ] `kubectl get networkpolicies -A -o yaml`
- [ ] `kubectl get events -n <namespace> --sort-by='.lastTimestamp'`
- [ ] Operator logs: `kubectl logs deployment/stellar-operator -n stellar-system --tail=200`
- [ ] Validator pod logs: `kubectl logs <validator-pod> -n <namespace> --tail=200`
- [ ] Output of `scripts/debug-network.sh` (see below)

---

## Related Documentation

- [Ingress Configuration Guide](../ingress-guide.md)
- [Peer Discovery](../peer-discovery.md)
- [mTLS Guide](../mtls-guide.md)
- [MetalLB BGP Anycast](../metallb-bgp-anycast.md)
- [NetworkPolicy examples](../gatekeeper-policies.md)
- [Diagnostic script](../../scripts/debug-network.sh)
