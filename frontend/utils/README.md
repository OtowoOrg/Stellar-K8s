# frontend/utils

Shared utility modules for the Stellar-K8s frontend tooling.

---

## manifest_builder.ts

Generates valid Kubernetes YAML manifests for `StellarNode` custom resources directly from the topology configurator's in-memory state, without any external YAML serialisation library.

### Exported API

| Export | Signature | Description |
|---|---|---|
| `buildManifests` | `(state: TopologyState, namespace?: string) => string` | Produces concatenated YAML for every placed node in the topology, with documents separated by `---`. Each node emits a `StellarNode` CRD manifest and a matching `PodDisruptionBudget`. |
| `buildNodeManifest` | `(node: PlacedStellarNode, zones: AvailabilityZone[], workerNodes: WorkerNode[]) => string` | Builds a single `StellarNode` YAML manifest for one placed node. Includes resource requests/limits, storage config, topology spread constraints, optional pod anti-affinity, and optional validator config. |
| `buildPodDisruptionBudget` | `(node: PlacedStellarNode) => string` | Generates a `PodDisruptionBudget` YAML manifest that enforces `minAvailable` for the node's pods. |
| `escapeYaml` | `(str: string) => string` | Normalises line endings and ensures a single trailing newline for safe embedding in YAML literal block scalars (`\|`). |

### Generated manifest structure

**StellarNode** (`stellar.org/v1alpha1`):
- `metadata.labels` always includes `app.kubernetes.io/managed-by: topology-configurator`
- `spec.topologySpreadConstraints` — one entry per zone, `whenUnsatisfiable: DoNotSchedule`
- `spec.affinity.podAntiAffinity` — `Hard` (required) or `Soft` (preferred, weight 100); omitted when set to `None`
- `spec.validatorConfig` — only present for `Validator` node types; includes quorum set as a YAML block scalar when defined

**PodDisruptionBudget** (`policy/v1`):
- Named `<node-name>-pdb`
- `spec.minAvailable` taken from the node's `minAvailable` field
- `spec.selector.matchLabels.app` matches the node's DNS-safe name

### Usage

```ts
import { buildManifests } from '../utils/manifest_builder';
import { useTopology } from './configurator/src/topology_builder/topology_store';

const [state] = useTopology();
const yaml = buildManifests(state, 'stellar-production');

// Write to a file, copy to clipboard, etc.
navigator.clipboard.writeText(yaml);
```

### Design notes

- All YAML is assembled from template literals for deterministic output with no runtime dependencies.
- Node names are normalised to valid Kubernetes DNS labels (lowercase, hyphens only).
- The `namespace` parameter in `buildManifests` overrides each node's individual namespace, making it easy to target staging vs. production clusters from the same topology definition.
