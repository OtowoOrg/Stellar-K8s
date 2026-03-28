# Failure Mode and Effects Analysis (FMEA) — StellarNode

Structured view of how a managed Stellar node can fail under Kubernetes and network stress, aligned with Chaos Mesh scenarios in [`tests/chaos/`](../tests/chaos/).

| ID | Failure mode | Cause / driver | Local effect | System effect | Detection | Mitigation (operator / ops) |
|----|----------------|----------------|-------------|-------------|-----------|------------------------------|
| F-01 | Operator pod killed | PodChaos, node loss | Reconcile pauses | Drift until new leader | Pod restarts, logs | kube-controller-manager; PDB if configured |
| F-02 | API server partition | NetworkChaos to apiserver | List/watch fail | No progress | API errors in logs | Automatic reconnect; widen timeouts |
| F-03 | API latency | NetworkChaos delay | Slow applies | Sync lag | Controller metrics | Tune client QPS; scale API |
| F-04 | Validator network split | NetworkChaos partition | Peers drop / stuck | Possible halt / catch-up | Core logs, Ready=false | Heal partition; optional quorum tooling |
| F-05 | Stellar Core OOM / crash | Resource limits, disk full | Pod CrashLoop | Not Ready | K8s events | Raise limits; PVC expansion; forensic snapshot |
| F-06 | Bad seed / Vault outage | Secret missing, injector down | Init fails | Pod pending | Pod status | Fix Vault / ESO / local Secret |
| F-07 | PVC loss | AZ failure, bad SC | Data loss risk | Full resync from archives | Volume events | RetentionPolicy; DR config |
| F-08 | Hardware node failure | Node loss, bad host | Multiple validator pods lost | Network stall if quorum slice lost | NodeReady=false | SCP-aware anti-affinity (placement.scpAwareAntiAffinity) |

## SCP-Aware Placement and Network Liveness

The Stellar Consensus Protocol (SCP) relies on quorum sets and slices. If a significant portion of a validator's quorum slice becomes unavailable, the validator (and potentially the entire network) can stall.

By default, Kubernetes might schedule multiple validator pods on the same physical node if resources are available. If that node fails, all pods on it are lost simultaneously. If these pods belong to the same quorum slice, the network's liveness is at risk.

### Impact of SCP-Aware Anti-Affinity

Enabling `placement.scpAwareAntiAffinity: true` in the `StellarNode` spec instructs the operator to:
1. Parse the validator's `quorumSet` configuration.
2. Identify all peers listed in that quorum set.
3. Inject `podAntiAffinity` rules into the validator's pod template.

These rules strongly discourage (using `preferredDuringScheduling` with weight 100) placing the validator on the same `kubernetes.io/hostname` as any of its quorum set peers.

**Benefit:** This significantly reduces the probability of a single hardware failure causing a correlated outage of an entire quorum slice, thereby preserving network liveness.

**Trade-off:** Scheduling may become more difficult in small clusters with limited nodes. If Kubernetes cannot find a node that satisfies the anti-affinity rules, it will still schedule the pod (due to the "preferred" nature of the rule), but it will prioritize spreading them out when possible.

Runbooks:

1. **Partition** — [`tests/chaos/run-partition-verify.sh`](../tests/chaos/run-partition-verify.sh)
2. **Forensic** — [`docs/forensic-snapshot.md`](forensic-snapshot.md)
3. **Vault** — [`docs/vault-stellar-tutorial.md`](vault-stellar-tutorial.md)
