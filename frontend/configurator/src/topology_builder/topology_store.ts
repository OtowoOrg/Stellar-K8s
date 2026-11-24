/**
 * Topology Store — React context + reducer based state management for the
 * Stellar-K8s topology configurator.
 *
 * Usage:
 *   // Wrap your app (or subtree) with the provider:
 *   <TopologyProvider>
 *     <App />
 *   </TopologyProvider>
 *
 *   // Consume state and dispatch inside any child component:
 *   const [state, dispatch] = useTopology();
 *   dispatch({ type: 'ADD_ZONE', payload: { id: 'z1', name: 'us-west-2a', region: 'us-west-2' } });
 */

import React, { createContext, useContext, useReducer } from 'react';
import type {
  TopologyState,
  AvailabilityZone,
  WorkerNodeConfig,
  PlacedStellarNode,
  NodeType,
  ResourceRequirements,
  StorageConfig,
  ValidatorConfig,
} from './types';
import { DEFAULT_RESOURCES, DEFAULT_STORAGE } from './types';

// ---------------------------------------------------------------------------
// Action union type
// ---------------------------------------------------------------------------

/** Payload for ADD_ZONE */
export interface AddZonePayload {
  /** Unique zone identifier. */
  id: string;
  /** Human-readable display label, e.g. "us-east-1a". */
  name: string;
  /** Cloud region this zone belongs to, e.g. "us-east-1". */
  region: string;
}

/** Payload for REMOVE_ZONE */
export interface RemoveZonePayload {
  /** ID of the zone to remove. Also removes all PlacedStellarNodes in that zone. */
  zoneId: string;
}

/** Payload for ADD_WORKER_NODE */
export interface AddWorkerNodePayload {
  /** The full worker node configuration to add to the store. */
  workerNode: WorkerNodeConfig;
}

/** Payload for REMOVE_WORKER_NODE */
export interface RemoveWorkerNodePayload {
  /** ID of the worker node to remove. Also unassigns it from any zone. */
  nodeId: string;
}

/** Payload for ASSIGN_WORKER_TO_ZONE */
export interface AssignWorkerToZonePayload {
  /** ID of the worker node to assign. */
  nodeId: string;
  /** ID of the zone to assign the worker node to. */
  zoneId: string;
}

/** Payload for UNASSIGN_WORKER_FROM_ZONE */
export interface UnassignWorkerFromZonePayload {
  /** ID of the worker node to unassign. */
  nodeId: string;
  /** ID of the zone to unassign the worker node from. */
  zoneId: string;
}

/** Payload for PLACE_NODE */
export interface PlaceNodePayload {
  /** ID of the availability zone where the node will be placed. */
  zoneId: string;
  /** The Stellar node type (Validator, Horizon, SorobanRpc). */
  nodeType: NodeType;
  /** Kubernetes resource name (DNS-label compliant). */
  name: string;
  /** Stellar network to connect to. */
  network: PlacedStellarNode['network'];
  /** Kubernetes namespace for the StellarNode CRD. */
  namespace: string;
  /** Stellar Core / Horizon / Soroban image version tag, e.g. "v21.0.0". */
  version: string;
  /** Optional CPU/memory resource requests; defaults to DEFAULT_RESOURCES. */
  resources?: ResourceRequirements;
  /** Optional persistent storage config; defaults to DEFAULT_STORAGE. */
  storage?: StorageConfig;
  /** Validator-specific config — required when nodeType is 'Validator'. */
  validatorConfig?: ValidatorConfig;
  /** Desired replica count; defaults to 1. */
  replicas?: number;
  /** Pod anti-affinity strategy; defaults to 'Soft'. */
  podAntiAffinity?: PlacedStellarNode['podAntiAffinity'];
}

/** Payload for REMOVE_PLACED_NODE */
export interface RemovePlacedNodePayload {
  /** ID of the PlacedStellarNode to remove from the canvas. */
  nodeId: string;
}

/** Payload for UPDATE_PLACED_NODE */
export interface UpdatePlacedNodePayload {
  /** ID of the PlacedStellarNode to update. */
  nodeId: string;
  /** Partial fields to merge into the existing PlacedStellarNode record. */
  updates: Partial<Omit<PlacedStellarNode, 'id'>>;
}

/** Payload for SET_SELECTED_ZONE */
export interface SetSelectedZonePayload {
  /** ID of the zone to select, or null to deselect. */
  zoneId: string | null;
}

/** Payload for SET_DRAGGED_NODE_TYPE */
export interface SetDraggedNodeTypePayload {
  /** The NodeType currently being dragged, or null when drag ends. */
  nodeType: NodeType | null;
}

/**
 * Discriminated union of all actions that can be dispatched to the topology
 * reducer. Each action carries a `type` discriminator and an optional
 * `payload` with the data required to perform the state transition.
 */
export type TopologyAction =
  | { type: 'ADD_ZONE'; payload: AddZonePayload }
  | { type: 'REMOVE_ZONE'; payload: RemoveZonePayload }
  | { type: 'ADD_WORKER_NODE'; payload: AddWorkerNodePayload }
  | { type: 'REMOVE_WORKER_NODE'; payload: RemoveWorkerNodePayload }
  | { type: 'ASSIGN_WORKER_TO_ZONE'; payload: AssignWorkerToZonePayload }
  | { type: 'UNASSIGN_WORKER_FROM_ZONE'; payload: UnassignWorkerFromZonePayload }
  | { type: 'PLACE_NODE'; payload: PlaceNodePayload }
  | { type: 'REMOVE_PLACED_NODE'; payload: RemovePlacedNodePayload }
  | { type: 'UPDATE_PLACED_NODE'; payload: UpdatePlacedNodePayload }
  | { type: 'SET_SELECTED_ZONE'; payload: SetSelectedZonePayload }
  | { type: 'SET_DRAGGED_NODE_TYPE'; payload: SetDraggedNodeTypePayload }
  | { type: 'RESET' };

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Generates a lightweight pseudo-UUID for client-side entity IDs. */
function generateId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 9)}`;
}

// ---------------------------------------------------------------------------
// Reducer
// ---------------------------------------------------------------------------

/**
 * Pure reducer that computes the next TopologyState from the current state
 * and a dispatched action. All state transitions are immutable.
 *
 * @param state  - Current topology state.
 * @param action - Action describing the desired state change.
 * @returns      - Next topology state.
 */
export function topologyReducer(
  state: TopologyState,
  action: TopologyAction,
): TopologyState {
  switch (action.type) {
    // -----------------------------------------------------------------------
    case 'ADD_ZONE': {
      const { id, name, region } = action.payload;
      // Prevent duplicate zone IDs
      if (state.zones.some((z) => z.id === id)) {
        return state;
      }
      const newZone: AvailabilityZone = { id, name, region, workerNodeIds: [] };
      return {
        ...state,
        zones: [...state.zones, newZone],
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'REMOVE_ZONE': {
      const { zoneId } = action.payload;
      return {
        ...state,
        zones: state.zones.filter((z) => z.id !== zoneId),
        // Remove all placed nodes that were in this zone
        placedNodes: state.placedNodes.filter(
          (n) => n.availabilityZoneId !== zoneId,
        ),
        // Deselect if the removed zone was selected
        selectedZoneId:
          state.selectedZoneId === zoneId ? null : state.selectedZoneId,
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'ADD_WORKER_NODE': {
      const { workerNode } = action.payload;
      if (state.workerNodes.some((w) => w.id === workerNode.id)) {
        return state;
      }
      return {
        ...state,
        workerNodes: [...state.workerNodes, workerNode],
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'REMOVE_WORKER_NODE': {
      const { nodeId } = action.payload;
      return {
        ...state,
        workerNodes: state.workerNodes.filter((w) => w.id !== nodeId),
        // Remove the worker from every zone that references it
        zones: state.zones.map((z) => ({
          ...z,
          workerNodeIds: z.workerNodeIds.filter((id) => id !== nodeId),
        })),
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'ASSIGN_WORKER_TO_ZONE': {
      const { nodeId, zoneId } = action.payload;
      return {
        ...state,
        zones: state.zones.map((z) => {
          if (z.id !== zoneId) return z;
          // Avoid duplicates
          if (z.workerNodeIds.includes(nodeId)) return z;
          return { ...z, workerNodeIds: [...z.workerNodeIds, nodeId] };
        }),
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'UNASSIGN_WORKER_FROM_ZONE': {
      const { nodeId, zoneId } = action.payload;
      return {
        ...state,
        zones: state.zones.map((z) => {
          if (z.id !== zoneId) return z;
          return {
            ...z,
            workerNodeIds: z.workerNodeIds.filter((id) => id !== nodeId),
          };
        }),
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'PLACE_NODE': {
      const {
        zoneId,
        nodeType,
        name,
        network,
        namespace,
        version,
        resources,
        storage,
        validatorConfig,
        replicas,
        podAntiAffinity,
      } = action.payload;

      const newNode: PlacedStellarNode = {
        id: generateId(),
        nodeType,
        network,
        name,
        namespace,
        version,
        replicas: replicas ?? 1,
        availabilityZoneId: zoneId,
        resources: resources ?? { ...DEFAULT_RESOURCES },
        storage: storage ?? { ...DEFAULT_STORAGE },
        validatorConfig,
        maxUnavailable: 1,
        minAvailable: 1,
        podAntiAffinity: podAntiAffinity ?? 'Soft',
      };

      return {
        ...state,
        placedNodes: [...state.placedNodes, newNode],
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'REMOVE_PLACED_NODE': {
      const { nodeId } = action.payload;
      return {
        ...state,
        placedNodes: state.placedNodes.filter((n) => n.id !== nodeId),
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'UPDATE_PLACED_NODE': {
      const { nodeId, updates } = action.payload;
      return {
        ...state,
        placedNodes: state.placedNodes.map((n) =>
          n.id === nodeId ? { ...n, ...updates } : n,
        ),
        isDirty: true,
      };
    }

    // -----------------------------------------------------------------------
    case 'SET_SELECTED_ZONE': {
      return {
        ...state,
        selectedZoneId: action.payload.zoneId,
      };
    }

    // -----------------------------------------------------------------------
    case 'SET_DRAGGED_NODE_TYPE': {
      return {
        ...state,
        draggedNodeType: action.payload.nodeType,
      };
    }

    // -----------------------------------------------------------------------
    case 'RESET': {
      return createInitialState();
    }

    // -----------------------------------------------------------------------
    default: {
      // Exhaustiveness check — TypeScript will error if a case is missed.
      const _exhaustive: never = action;
      return _exhaustive;
    }
  }
}

// ---------------------------------------------------------------------------
// Initial state factory
// ---------------------------------------------------------------------------

/**
 * Creates the default 3-zone topology used when the configurator first loads
 * (or after a RESET action).
 *
 * Layout:
 *   - 3 availability zones: us-east-1a, us-east-1b, us-east-1c
 *   - 6 worker nodes, 2 pre-assigned per zone
 *   - No placed Stellar nodes (user starts with a clean canvas)
 */
export function createInitialState(): TopologyState {
  const region = 'us-east-1';

  // Build the 3 availability zones
  const zones: AvailabilityZone[] = [
    { id: 'zone-us-east-1a', name: 'us-east-1a', region, workerNodeIds: ['worker-1a-1', 'worker-1a-2'] },
    { id: 'zone-us-east-1b', name: 'us-east-1b', region, workerNodeIds: ['worker-1b-1', 'worker-1b-2'] },
    { id: 'zone-us-east-1c', name: 'us-east-1c', region, workerNodeIds: ['worker-1c-1', 'worker-1c-2'] },
  ];

  // 6 worker nodes — 2 per zone
  const workerNodes: WorkerNodeConfig[] = [
    {
      id: 'worker-1a-1',
      name: 'node-us-east-1a-1',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1a', 'topology.kubernetes.io/region': region },
    },
    {
      id: 'worker-1a-2',
      name: 'node-us-east-1a-2',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1a', 'topology.kubernetes.io/region': region },
    },
    {
      id: 'worker-1b-1',
      name: 'node-us-east-1b-1',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1b', 'topology.kubernetes.io/region': region },
    },
    {
      id: 'worker-1b-2',
      name: 'node-us-east-1b-2',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1b', 'topology.kubernetes.io/region': region },
    },
    {
      id: 'worker-1c-1',
      name: 'node-us-east-1c-1',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1c', 'topology.kubernetes.io/region': region },
    },
    {
      id: 'worker-1c-2',
      name: 'node-us-east-1c-2',
      labels: { 'topology.kubernetes.io/zone': 'us-east-1c', 'topology.kubernetes.io/region': region },
    },
  ];

  return {
    zones,
    workerNodes,
    placedNodes: [],
    selectedZoneId: null,
    draggedNodeType: null,
    isDirty: false,
  };
}

// ---------------------------------------------------------------------------
// React context
// ---------------------------------------------------------------------------

/**
 * The shape of the value provided by TopologyContext.
 * Consumers receive both the current state and a dispatch function.
 */
type TopologyContextValue = [TopologyState, React.Dispatch<TopologyAction>];

/**
 * React context that distributes topology state and dispatch throughout the
 * component tree. Use the `useTopology` hook instead of consuming this
 * context directly.
 */
export const TopologyContext = createContext<TopologyContextValue | undefined>(
  undefined,
);

// ---------------------------------------------------------------------------
// Provider component
// ---------------------------------------------------------------------------

/** Props accepted by TopologyProvider. */
export interface TopologyProviderProps {
  /** The React subtree that will have access to topology state. */
  children: React.ReactNode;
  /**
   * Optional initial state override. When omitted the provider uses
   * `createInitialState()` to produce a fresh default topology.
   */
  initialState?: TopologyState;
}

/**
 * Context provider that wires up the topology reducer and makes state
 * available to all descendant components via `useTopology`.
 *
 * @example
 * ```tsx
 * <TopologyProvider>
 *   <TopologyCanvas />
 * </TopologyProvider>
 * ```
 */
export function TopologyProvider({
  children,
  initialState,
}: TopologyProviderProps): React.JSX.Element {
  const [state, dispatch] = useReducer(
    topologyReducer,
    initialState ?? createInitialState(),
  );

  return React.createElement(
    TopologyContext.Provider,
    { value: [state, dispatch] },
    children,
  );
}

// ---------------------------------------------------------------------------
// Consumer hook
// ---------------------------------------------------------------------------

/**
 * Returns the current topology state and dispatch function.
 *
 * Must be called inside a component that is a descendant of
 * `TopologyProvider`; throws otherwise.
 *
 * @returns `[state, dispatch]` tuple — identical to what `useReducer` returns.
 *
 * @example
 * ```tsx
 * function ZoneList() {
 *   const [state, dispatch] = useTopology();
 *   return <ul>{state.zones.map(z => <li key={z.id}>{z.name}</li>)}</ul>;
 * }
 * ```
 */
export function useTopology(): TopologyContextValue {
  const ctx = useContext(TopologyContext);
  if (ctx === undefined) {
    throw new Error(
      'useTopology must be called inside a <TopologyProvider>. ' +
        'Make sure your component tree is wrapped with <TopologyProvider>.',
    );
  }
  return ctx;
}
