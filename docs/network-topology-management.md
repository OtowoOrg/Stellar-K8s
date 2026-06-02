# Advanced Network Topology Management (#869)

Implements the foundation of the Advanced Network Topology Management epic: the
`StellarTopology` CRD plus operator logic for **quorum health**, **network
partition detection**, **peer-optimization recommendations**, and a what-if
**network simulation**.

## StellarTopology CRD

`StellarTopology` (`stellar.org/v1alpha1`, shortname `stopo`) declares a
validator network's peer relationships and quorum expectations.

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarTopology
metadata:
  name: pubnet-core
  namespace: stellar
spec:
  minOnlinePct: 66.0                 # quorum threshold (classic BFT 2/3)
  partitionDetectionWindowSeconds: 30
  validators:
    - { name: core-1, peers: [core-2, core-3] }
    - { name: core-2, peers: [core-1, core-3] }
    - { name: core-3, peers: [core-1, core-2] }
```

### Status

```yaml
status:
  phase: Healthy                     # Pending | Healthy | Degraded | Partitioned
  totalValidators: 3
  onlineValidators: 3
  quorumHealthPct: 100.0
  partitionDetected: false
  partitionCount: 1
  recommendations: []
```

## Controller logic

`src/controller/topology/` holds pure, unit-tested analysis:

- **Quorum health** — fraction of declared validators currently online.
- **Partition detection** — connected-components (BFS) over the subgraph
  induced by *online* validators; more than one component means a partition.
- **Recommendations** — flags validators with `< 2` peers, surfaces partitions,
  and warns when online validators fall below the quorum threshold.
- **Network simulation** — `simulate_failures` re-runs the analysis as if a set
  of validators were offline, so operators can predict whether removing nodes
  would partition the network *before* it happens (e.g. removing a bridge node
  in a line topology splits the network).

Connections are treated as undirected for reachability; peer references to
names that are not declared validators are ignored.

## Scope and follow-up

This slice delivers the CRD, quorum health, partition detection,
recommendations, and failure simulation. The remaining epic capabilities build
on this foundation and are tracked as follow-up work:

- SCP message streaming to Kafka (existing `src/controller/quorum/` modules).
- Real-time topology visualization dashboard.
- Historical SCP data querying.
- Automated peer reconfiguration from recommendations.
