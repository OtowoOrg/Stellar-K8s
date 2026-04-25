# SCP Topology Visualization Dashboard

The Stellar-K8s operator includes a real-time visualization of the Stellar Consensus Protocol (SCP) quorum graph. The dashboard shows which validators are currently in each SCP phase, which nodes are critical to consensus, and whether any nodes appear stalled.

---

## Enabling the Feature

The SCP topology endpoints are always available when the operator is running. Set the `enable_scp_topology` feature flag to `"true"` in the `stellar-operator-config` ConfigMap to signal to the operator that topology polling should be prioritized:

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-operator-config
  namespace: stellar-system
data:
  enable_scp_topology: "true"
```

### Via Helm

Set the flag in your `values.yaml`:

```yaml
featureFlags:
  enableScpTopology: "true"
```

Or pass it on the CLI:

```bash
helm upgrade stellar-operator charts/stellar-operator \
  --namespace stellar-system \
  --set featureFlags.enableScpTopology=true
```

---

## Accessing the Dashboard

The SCP topology graph is built into the operator's web dashboard. Open it in a browser:

```bash
# Port-forward the operator service
kubectl port-forward -n stellar-system svc/stellar-operator 9090:9090

# Open in browser
open http://localhost:9090
```

Click the **"🔗 SCP Topology"** tab in the header navigation.

---

## Endpoints

### REST snapshot

```
GET /api/v1/quorum/topology
```

Returns a one-shot JSON snapshot of the current topology. Useful for scripting or CI checks.

**Example:**
```bash
curl http://localhost:9090/api/v1/quorum/topology | jq .
```

### WebSocket stream

```
GET /api/v1/quorum/topology/stream
```

Upgrades to a WebSocket connection. The server pushes a fresh `QuorumTopologyResponse` JSON frame every **5 seconds** until the client disconnects.

**Example (wscat):**
```bash
wscat -c ws://localhost:9090/api/v1/quorum/topology/stream
```

---

## Response Schema

Both the REST and WebSocket endpoints return the same `QuorumTopologyResponse` structure:

```json
{
  "nodes": [
    {
      "id": "GCEZ…XUYZ",
      "full_id": "GCEZWKCA5VLDNRLN3RPRJMRZOX3Z6G5CHCGBWRXSJHEG8VORHEA3PUO",
      "phase": "EXTERNALIZE",
      "is_critical": true,
      "threshold": 3,
      "stalled": false
    }
  ],
  "edges": [
    { "source": "GCEZ…XUYZ", "target": "GABC…1234" }
  ],
  "stalled_nodes": [],
  "timestamp": "2025-04-25T12:00:00Z",
  "healthy": true
}
```

### Field reference

| Field | Type | Description |
|-------|------|-------------|
| `nodes[].id` | string | Short key (first 4 + "…" + last 4 chars) |
| `nodes[].full_id` | string | Full Ed25519 validator public key |
| `nodes[].phase` | string | Current SCP phase: `PREPARE`, `CONFIRM`, `EXTERNALIZE`, or `UNKNOWN` |
| `nodes[].is_critical` | bool | `true` if removing this node would break quorum consensus |
| `nodes[].threshold` | integer | Quorum threshold for this node's quorum set |
| `nodes[].stalled` | bool | `true` if the node's SCP phase has not advanced |
| `edges[].source` | string | Short key of the source node |
| `edges[].target` | string | Short key of the quorum member |
| `stalled_nodes` | string[] | Short keys of all stalled nodes |
| `timestamp` | string | RFC3339 timestamp of the snapshot |
| `healthy` | bool | `false` if all pod queries failed |

---

## Graph Visualization

The dashboard renders a **D3.js force-directed graph**:

| Node color | Meaning |
|------------|---------|
| Blue | Phase: PREPARE |
| Yellow | Phase: CONFIRM |
| Green | Phase: EXTERNALIZE |
| Grey | Phase: UNKNOWN (pod unreachable) |
| Red border | Critical node OR stalled node |

Edges are directed arrows showing which nodes each validator includes in its quorum set.

**Interactions:**
- **Drag** — move individual nodes to reposition the graph
- **Scroll / pinch** — zoom in and out
- **Hover** — tooltip showing the full node ID, phase, threshold, and status

---

## How Data Is Collected

The operator queries every running pod that carries the label  
`app.kubernetes.io/name=stellar-node,stellar.org/node-type=Validator`

For each pod IP it calls:
```
GET http://{pod_ip}:11626/scp?limit=1
```

This is the standard Stellar Core HTTP admin API, available on port 11626 inside the cluster. No additional configuration is required.

---

## Stall Detection

A node is considered **stalled** when:
- Its SCP phase is `UNKNOWN` (the pod is unreachable), or
- Its ballot counter is `0` and phase is `PREPARE` (the node never advanced past its initial state)

In a streaming WebSocket session, the client-side `StallTracker` also flags nodes whose `phase + ballot_counter` combination has not changed for more than 30 seconds.

---

## Troubleshooting

**Graph shows no nodes**

- Ensure at least one Validator pod is running and labelled with `stellar.org/node-type=Validator`.
- Check the operator logs for "SCP query failed for pod" messages.
- Verify port 11626 is reachable between the operator pod and validator pods (same namespace or correct NetworkPolicy).

**WebSocket status shows "error"**

- Confirm the operator is running and the port-forward is active.
- Some browsers block mixed-content WebSocket connections (ws:// from an https:// page). Use the matching protocol.

**Node shows phase "UNKNOWN"**

- The validator pod is running but its Stellar Core HTTP API is not responding on port 11626.
- Check the validator pod logs: `kubectl logs -n stellar <pod-name>`.

**Critical nodes flagged in red**

- This means removing those validators would break quorum intersection across the cluster.
- Consider adding additional validators to reduce the fragility of the quorum set.
- See `stellar_quorum_fragility_score` in the Prometheus metrics for a numeric score.

---

## Related Documentation

- [API Reference](api-reference.md) — full `StellarNode` CRD fields including `validatorConfig.quorumSet`
- [Monitoring & Observability](../README.md#monitoring--observability) — Prometheus metrics and Grafana dashboards
- [Peer Discovery](peer-discovery.md) — automatic peer discovery for validator clusters
