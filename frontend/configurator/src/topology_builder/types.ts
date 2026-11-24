/**
 * Core type definitions for the Stellar-K8s Topology Configurator.
 *
 * These types model the domain objects used throughout the drag-and-drop
 * topology builder UI: availability zones, worker nodes, placed Stellar
 * nodes, validation results, and drag-and-drop payloads.
 */

// ---------------------------------------------------------------------------
// Primitive / enum-like union types
// ---------------------------------------------------------------------------

/** The three supported Stellar node types managed by the operator. */
export type NodeType = 'Validator' | 'Horizon' | 'SorobanRpc';

/** The Stellar networks a node can connect to. */
export type StellarNetwork = 'mainnet' | 'testnet' | 'futurenet';

// ---------------------------------------------------------------------------
// Resource & storage configuration
// ---------------------------------------------------------------------------

/**
 * Kubernetes-style CPU and memory resource requirements.
 * Values follow standard Kubernetes resource quantity notation,
 * e.g. "500m" for CPU and "1Gi" for memory.
 */
export interface ResourceRequirements {
  /** CPU request/limit, e.g. "500m", "2", "4000m". */
  cpu: string;
  /** Memory request/limit, e.g. "512Mi", "2Gi", "8Gi". */
  memory: string;
}

/**
 * Persistent storage configuration for a Stellar node.
 */
export interface StorageConfig {
  /** Kubernetes StorageClass name, e.g. "standard", "gp3", "local-path". */
  storageClass: string;
  /** Requested volume size, e.g. "100Gi", "500Gi". */
  size: string;
  /**
   * Storage backend mode.
   * - `PersistentVolume`: cloud-backed PVC (EBS, GCP PD, etc.)
   * - `Local`: low-latency NVMe local storage
   */
  mode: 'PersistentVolume' | 'Local';
  /**
   * Volume retention policy when a StellarNode is deleted.
   * - `Retain`: PVC/PV is kept for manual recovery.
   * - `Delete`: PVC/PV is automatically removed.
   */
  retentionPolicy: 'Retain' | 'Delete';
}

// ---------------------------------------------------------------------------
// Node-specific configuration
// ---------------------------------------------------------------------------

/**
 * Configuration specific to Stellar Validator nodes.
 */
export interface ValidatorConfig {
  /**
   * Name of the Kubernetes Secret that holds the validator seed key.
   * The secret must be pre-created in the same namespace.
   */
  seedSecretRef: string;
  /**
   * Optional inline quorum set definition (TOML fragment or JSON string).
   * When omitted the operator uses the network's default quorum set.
   */
  quorumSet?: string;
  /**
   * Optional list of history archive URLs this validator publishes to.
   * Example: ["https://history.stellar.org/prd/core-live/core_live_001"]
   */
  historyArchiveUrls?: string[];
  /** Whether the validator should publish a history archive. */
  enableHistoryArchive: boolean;
}

// ---------------------------------------------------------------------------
// Cluster topology objects
// ---------------------------------------------------------------------------

/**
 * A Kubernetes taint applied to a worker node, mirroring the core API type.
 */
export interface NodeTaint {
  /** Taint key, e.g. "dedicated". */
  key: string;
  /** Taint value (optional). */
  value?: string;
  /**
   * Taint effect controlling pod scheduling behaviour.
   * Maps directly to Kubernetes taint effect values.
   */
  effect: 'NoSchedule' | 'PreferNoSchedule' | 'NoExecute';
}

/**
 * Represents a physical or virtual Kubernetes worker node that can host
 * Stellar pods. Used to populate availability zones in the topology canvas.
 */
export interface WorkerNodeConfig {
  /** Unique identifier for this worker node (typically the Kubernetes node name). */
  id: string;
  /** Human-readable display name shown in the UI. */
  name: string;
  /** Kubernetes node labels, e.g. `{ "node.kubernetes.io/instance-type": "m5.xlarge" }`. */
  labels: Record<string, string>;
  /** Optional taints applied to this node that affect pod scheduling. */
  taints?: NodeTaint[];
}

/**
 * An availability zone (AZ) grouping of worker nodes. In cloud environments
 * this corresponds to a provider AZ (e.g. "us-east-1a"). In on-prem setups
 * it can represent a rack or failure domain.
 */
export interface AvailabilityZone {
  /** Unique identifier for this zone, used as a foreign key in PlacedStellarNode. */
  id: string;
  /** Human-readable display label, e.g. "us-east-1a". */
  name: string;
  /** Cloud region this zone belongs to, e.g. "us-east-1". */
  region: string;
  /** IDs of WorkerNodeConfig objects assigned to this zone. */
  workerNodeIds: string[];
}

/**
 * A Stellar node instance that has been placed onto a specific availability
 * zone in the topology canvas. This is the primary entity the configurator
 * manages and ultimately serialises to a StellarNode CRD manifest.
 */
export interface PlacedStellarNode {
  /** Unique identifier for this placed node (UUID generated client-side). */
  id: string;
  /** The Stellar node type this instance represents. */
  nodeType: NodeType;
  /** The Stellar network this node connects to. */
  network: StellarNetwork;
  /** Kubernetes resource name (must be DNS-label compliant). */
  name: string;
  /** Kubernetes namespace where the StellarNode CRD will be created. */
  namespace: string;
  /** Stellar Core / Horizon / Soroban image version tag, e.g. "v21.0.0". */
  version: string;
  /** Desired replica count for Deployments (ignored for StatefulSet Validators). */
  replicas: number;
  /** ID of the AvailabilityZone this node is pinned to. */
  availabilityZoneId: string;
  /** CPU and memory resource requests/limits. */
  resources: ResourceRequirements;
  /** Persistent storage configuration. */
  storage: StorageConfig;
  /**
   * Validator-specific configuration. Required when nodeType is 'Validator';
   * undefined for Horizon and SorobanRpc nodes.
   */
  validatorConfig?: ValidatorConfig;
  /**
   * Maximum number of pods that can be unavailable during a voluntary
   * disruption (used to generate PodDisruptionBudget). */
  maxUnavailable: number;
  /**
   * Minimum number of pods that must be available during a voluntary
   * disruption (used to generate PodDisruptionBudget). */
  minAvailable: number;
  /**
   * Pod anti-affinity strategy to spread replicas across nodes/zones.
   * - `Hard`: requiredDuringSchedulingIgnoredDuringExecution
   * - `Soft`: preferredDuringSchedulingIgnoredDuringExecution
   * - `None`: no anti-affinity rule applied
   */
  podAntiAffinity: 'Hard' | 'Soft' | 'None';
}

// ---------------------------------------------------------------------------
// Top-level topology state
// ---------------------------------------------------------------------------

/**
 * The complete state managed by the topology configurator UI.
 * This is the single source of truth passed through React context.
 */
export interface TopologyState {
  /** All availability zones visible on the canvas. */
  zones: AvailabilityZone[];
  /** All worker nodes available for zone assignment. */
  workerNodes: WorkerNodeConfig[];
  /** All Stellar node instances currently placed on the canvas. */
  placedNodes: PlacedStellarNode[];
  /** ID of the currently selected zone, or null if none is selected. */
  selectedZoneId: string | null;
  /**
   * The NodeType currently being dragged from the palette, or null when no
   * drag is in progress.
   */
  draggedNodeType: NodeType | null;
  /**
   * True when the topology has unsaved changes relative to the last
   * saved/exported state.
   */
  isDirty: boolean;
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/**
 * A blocking validation error that must be resolved before the topology
 * can be exported as Kubernetes manifests.
 */
export interface ValidationError {
  /** Machine-readable error code, e.g. "VALIDATOR_NO_SEED_SECRET". */
  code: string;
  /** Human-readable description shown in the validation panel. */
  message: string;
  /** IDs of the zones involved in this error (may be empty for global errors). */
  zoneIds: string[];
}

/**
 * A non-blocking validation warning. The topology can still be exported
 * but the user should review these before deploying.
 */
export interface ValidationWarning {
  /** Machine-readable warning code, e.g. "SINGLE_ZONE_VALIDATOR". */
  code: string;
  /** Human-readable description shown in the validation panel. */
  message: string;
  /** IDs of the zones involved in this warning (may be empty for global warnings). */
  zoneIds: string[];
}

/**
 * Aggregate result returned by the topology validation engine.
 */
export interface ValidationResult {
  /** True only when there are zero errors (warnings are permitted). */
  valid: boolean;
  /** List of blocking errors that prevent manifest export. */
  errors: ValidationError[];
  /** List of non-blocking warnings the user should review. */
  warnings: ValidationWarning[];
}

// ---------------------------------------------------------------------------
// Drag-and-drop
// ---------------------------------------------------------------------------

/**
 * Data transferred during a drag-and-drop operation on the topology canvas.
 *
 * Two drag sources exist:
 * - `node-type`: dragging a new node type from the palette onto a zone.
 * - `placed-node`: dragging an already-placed node between zones.
 */
export interface DragPayload {
  /**
   * Discriminator indicating the drag source.
   * - `'node-type'`: originates from the node-type palette.
   * - `'placed-node'`: originates from an existing PlacedStellarNode card.
   */
  type: 'node-type' | 'placed-node';
  /** The Stellar node type being dragged (always populated). */
  nodeType: NodeType;
  /**
   * ID of the PlacedStellarNode being moved when `type === 'placed-node'`;
   * null when dragging a new node type from the palette.
   */
  placedNodeId: string | null;
}

// ---------------------------------------------------------------------------
// Default constants
// ---------------------------------------------------------------------------

/**
 * Sensible baseline resource requests suitable for a Testnet node.
 * Override per-node in the configurator panel.
 */
export const DEFAULT_RESOURCES: ResourceRequirements = {
  cpu: '500m',
  memory: '1Gi',
};

/**
 * Default storage configuration using a standard PersistentVolume with
 * Retain policy to protect against accidental data loss.
 */
export const DEFAULT_STORAGE: StorageConfig = {
  storageClass: 'standard',
  size: '100Gi',
  mode: 'PersistentVolume',
  retentionPolicy: 'Retain',
};
