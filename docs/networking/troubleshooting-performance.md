# Troubleshooting and Performance Tuning

> Use this runbook when validators exhibit elevated ledger age, peer churn, or SCP timeout behavior likely rooted in dataplane or routing instability.

---

## 1) First-Response Commands

| Goal | Command | Expected Signal |
|---|---|---|
| Verify host routes | `ip route` | Pod CIDR and service routes present, no frequent route replacement |
| Check Calico BGP status | `calicoctl node status` | All peers `Established` |
| Inspect Cilium datapath | `cilium monitor --type drop` | No sustained drop stream for validator flows |
| Validate socket pressure | `ss -s` | No abnormal orphaned/overflow sockets |
| Check NIC errors | `ethtool -S eth0` | Low/zero `rx_missed_errors`, `tx_errors` |

### Example: route sanity check

```bash
ip route | egrep '10\.244\.|10\.96\.'
```

### Example: Cilium packet drops by reason

```bash
cilium monitor --type drop -v | head -n 40
```

---

## 2) Kernel and eBPF Tuning Profile

> Validate these values in staging before production rollout. Persist through your node OS configuration management layer.

```bash
# /etc/sysctl.d/99-stellar-network.conf
net.core.rmem_max = 67108864
net.core.wmem_max = 67108864
net.core.netdev_max_backlog = 500000
net.ipv4.tcp_rmem = 4096 87380 33554432
net.ipv4.tcp_wmem = 4096 65536 33554432
net.ipv4.tcp_congestion_control = bbr
net.ipv4.tcp_mtu_probing = 1
net.ipv4.ip_local_port_range = 10240 65000
net.ipv4.tcp_tw_reuse = 1
kernel.sched_migration_cost_ns = 5000000
vm.max_map_count = 1048576
```

Apply and verify:

```bash
sudo sysctl --system
sysctl net.core.netdev_max_backlog net.ipv4.tcp_congestion_control
```

---

## 3) Latency and Throughput Baselines

| Metric | Healthy Target | Warning Threshold | Critical Threshold |
|---|---|---|---|
| Validator p99 peer RTT | < 20 ms (intra-region) | 20-40 ms | > 40 ms |
| Packet drop rate (validator node) | < 0.01% | 0.01-0.1% | > 0.1% |
| Ledger age (`stellar_core_ledger_age`) | < 5 sec | 5-20 sec | > 20 sec |
| Reconciliation p95 | < 2 sec | 2-5 sec | > 5 sec |

---

## 4) Root-Cause Pivots

1. If BGP sessions flap, inspect ToR hold timers, BFD profile mismatch, and node CPU starvation.
2. If only public RPC is impacted, inspect LB target health and connection tracking pressure.
3. If validator-only traffic is impacted, verify policy updates did not block `11625/tcp` east-west paths.
4. If Cilium is in strict replacement mode, inspect kube-proxy replacement health and BPF map pressure.

## 5) Command Bundle for Incident Capture

```bash
kubectl get pods -A -o wide
kubectl get events -A --sort-by=.metadata.creationTimestamp | tail -n 200
cilium status --verbose || true
calicoctl node status || true
ip -s link
ss -antp | head -n 100
```

Store outputs with incident timestamp and node identity for postmortem correlation.
