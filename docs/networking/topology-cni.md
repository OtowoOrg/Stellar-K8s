# Topology and CNI Integration

> This guide is specific to Stellar Core validators and RPC nodes where peer gossip, SCP message exchange, and ledger catch-up are sensitive to microbursts, path asymmetry, and packet loss.

---

## 1) Calico Integration (IP-in-IP + BGP Peer Encapsulation)

### When to choose Calico

Choose Calico when you need:

- Familiar BGP controls with explicit route advertisements.
- Granular GlobalNetworkPolicy behavior for namespace/workload identity.
- Hybrid mode support (encapsulation between non-routable segments, native routing elsewhere).

### Reference: Calico Installation Values

```yaml
# calico-values.yaml
installation:
  calicoNetwork:
    bgp: Enabled
    ipPools:
      - cidr: 10.244.0.0/16
        encapsulation: IPIPCrossSubnet
        natOutgoing: Enabled
        nodeSelector: all()

apiServer:
  enabled: true

felixConfiguration:
  bpfEnabled: false
  logSeverityScreen: Info
  defaultEndpointToHostAction: Drop
  bpfConnectTimeLoadBalancing: Disabled
```

### BGP Peer Model for Node-to-ToR

```yaml
# calico-bgppeers.yaml
apiVersion: projectcalico.org/v3
kind: BGPPeer
metadata:
  name: tor-peer-rack-a
spec:
  nodeSelector: rack == "a"
  peerIP: 172.20.0.1
  asNumber: 64512
---
apiVersion: projectcalico.org/v3
kind: BGPPeer
metadata:
  name: tor-peer-rack-b
spec:
  nodeSelector: rack == "b"
  peerIP: 172.21.0.1
  asNumber: 64512
```

### Validation Commands

```bash
calicoctl node status
calicoctl get bgppeers -o wide
ip route show table main | grep 10.244.
```

Expected state:

- `State = up` for each ToR peer.
- Pod CIDRs announced to ToR and reflected across racks.
- No route flapping during validator pod rollouts.

---

## 2) Cilium Integration (eBPF Overlay/Direct Routing + XDP)

### When to choose Cilium

Choose Cilium when you need:

- eBPF dataplane efficiency and advanced visibility (`hubble`).
- Direct routing in L3-routable fabrics with low encapsulation overhead.
- XDP acceleration on capable NIC/kernel combinations.

### Reference: Cilium Helm Values

```yaml
# cilium-values.yaml
kubeProxyReplacement: strict
routingMode: native
autoDirectNodeRoutes: true
ipv4NativeRoutingCIDR: 10.244.0.0/16
bpf:
  masquerade: true
  lbAcceleration: native
  datapathMode: veth

bandwidthManager:
  enabled: true
  bbr: true

hubble:
  enabled: true
  relay:
    enabled: true
  ui:
    enabled: false

l2announcements:
  enabled: false
xdp:
  enabled: true
```

### Overlay Fallback for Non-Routable Segments

```yaml
# cilium-values-overlay.yaml
routingMode: tunnel
tunnelProtocol: vxlan
kubeProxyReplacement: strict
```

### Validation Commands

```bash
cilium status --verbose
cilium bpf lb list
cilium node list
cilium connectivity test
```

Expected state:

- `Cluster Pods` reachable across nodes without asymmetric RTT spikes.
- No sustained packet drops in `cilium monitor` for SCP paths.
- `kubeProxyReplacement: strict` active and healthy.

---

## 3) Stellar-Specific CNI Placement Rules

| Workload | Recommended Placement | Rationale |
|---|---|---|
| Validator StatefulSet | Dedicated node pool with low-overcommit CPU | Avoid consensus latency spikes and NUMA jitter |
| Horizon/RPC Deployments | Separate autoscaled pool behind LB | Isolate public query traffic from SCP traffic |
| Operator controller | Control-plane-adjacent system pool | Stable reconciliation and watch latency |

```yaml
# Example affinity fragment for validator pod template
affinity:
  nodeAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      nodeSelectorTerms:
        - matchExpressions:
            - key: node-role.stellar.io/validator
              operator: In
              values: ["true"]
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchLabels:
            app.kubernetes.io/component: validator
        topologyKey: kubernetes.io/hostname
```

## 4) Operational Guardrails

- Keep MTU consistent across CNI + underlay to avoid hidden fragmentation.
- Prefer direct routing for validators when L3 fabric allows it.
- Enable BFD on ToR peers if supported to reduce failure detection time.
- Pin kernel and CNI versions per environment tier (dev/stage/prod).
