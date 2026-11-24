/**
 * Vitest unit tests for buildManifests and buildNodeManifest
 * from /workspaces/Stellar-K8s/frontend/utils/manifest_builder.ts
 *
 * Tests build TopologyState objects manually and verify the YAML output
 * against specific string patterns.
 */

import { describe, it, expect } from 'vitest';
import {
  buildManifests,
  buildNodeManifest,
  buildPodDisruptionBudget,
} from '../../../../utils/manifest_builder';
import type {
  TopologyState,
  AvailabilityZone,
  PlacedStellarNode,
  WorkerNodeConfig,
} from '../types';
import { DEFAULT_RESOURCES, DEFAULT_STORAGE } from '../types';

// ---------------------------------------------------------------------------
// Test-data helpers
// ---------------------------------------------------------------------------

function makeZone(id: string, name: string, workerNodeIds: string[] = []): AvailabilityZone {
  return { id, name, region: 'us-east-1', workerNodeIds };
}

function makeWorkerNode(id: string): WorkerNodeConfig {
  return { id, name: id, labels: {} };
}

function makeValidator(
  id: string,
  zoneId: string,
  overrides: Partial<PlacedStellarNode> = {},
): PlacedStellarNode {
  return {
    id,
    nodeType: 'Validator',
    network: 'testnet',
    name: `validator-${id}`,
    namespace: 'stellar',
    version: 'v21.0.0',
    replicas: 1,
    availabilityZoneId: zoneId,
    resources: { ...DEFAULT_RESOURCES },
    storage: { ...DEFAULT_STORAGE },
    maxUnavailable: 1,
    minAvailable: 1,
    podAntiAffinity: 'Soft',
    validatorConfig: {
      seedSecretRef: `secret-${id}`,
      enableHistoryArchive: true,
      quorumSet: '[[QUORUM_SET]]\nTHRESHOLD_PERCENT=67',
      historyArchiveUrls: [
        'https://history.stellar.org/prd/core-live/core_live_001',
        'https://history.stellar.org/prd/core-live/core_live_002',
      ],
    },
    ...overrides,
  };
}

function makeHorizon(
  id: string,
  zoneId: string,
  overrides: Partial<PlacedStellarNode> = {},
): PlacedStellarNode {
  return {
    id,
    nodeType: 'Horizon',
    network: 'mainnet',
    name: `horizon-${id}`,
    namespace: 'stellar',
    version: 'v2.31.0',
    replicas: 2,
    availabilityZoneId: zoneId,
    resources: { cpu: '1', memory: '2Gi' },
    storage: { ...DEFAULT_STORAGE },
    maxUnavailable: 1,
    minAvailable: 1,
    podAntiAffinity: 'Hard',
    ...overrides,
  };
}

/**
 * Builds a complete 3-zone TopologyState used across most tests.
 */
function makeThreeZoneState(): TopologyState {
  const zones = [
    makeZone('zone-a', 'us-east-1a', ['worker-a']),
    makeZone('zone-b', 'us-east-1b', ['worker-b']),
    makeZone('zone-c', 'us-east-1c', ['worker-c']),
  ];
  const workerNodes = [
    makeWorkerNode('worker-a'),
    makeWorkerNode('worker-b'),
    makeWorkerNode('worker-c'),
  ];
  const placedNodes: PlacedStellarNode[] = [
    makeValidator('v1', 'zone-a'),
    makeHorizon('h1', 'zone-a'),
    makeValidator('v2', 'zone-b'),
    makeHorizon('h2', 'zone-b'),
    makeValidator('v3', 'zone-c'),
  ];
  return {
    zones,
    workerNodes,
    placedNodes,
    selectedZoneId: null,
    draggedNodeType: null,
    isDirty: false,
  };
}

// ---------------------------------------------------------------------------
// describe: buildManifests — basic structure
// ---------------------------------------------------------------------------

describe('buildManifests — basic YAML structure', () => {
  it('contains "apiVersion: stellar.org/v1alpha1" for all node manifests', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    expect(yaml).toContain('apiVersion: stellar.org/v1alpha1');
  });

  it('contains "kind: StellarNode" for node manifests', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    expect(yaml).toContain('kind: StellarNode');
  });

  it('returns empty string when there are no placed nodes', () => {
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).toBe('');
  });

  it('contains "kind: PodDisruptionBudget" in the output', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    expect(yaml).toContain('kind: PodDisruptionBudget');
  });

  it('generates one PodDisruptionBudget per placed node', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    const pdbCount = (yaml.match(/kind: PodDisruptionBudget/g) ?? []).length;
    expect(pdbCount).toBe(state.placedNodes.length);
  });
});

// ---------------------------------------------------------------------------
// describe: buildManifests — topology spread constraints
// ---------------------------------------------------------------------------

describe('buildManifests — topologySpreadConstraints', () => {
  it('includes topologySpreadConstraints in the manifest', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    expect(yaml).toContain('topologySpreadConstraints');
  });

  it('includes the zone topology key', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state);
    expect(yaml).toContain('topology.kubernetes.io/zone');
  });
});

// ---------------------------------------------------------------------------
// describe: buildManifests — podAntiAffinity
// ---------------------------------------------------------------------------

describe('buildManifests — podAntiAffinity', () => {
  it('Hard podAntiAffinity produces requiredDuringSchedulingIgnoredDuringExecution', () => {
    const node = makeHorizon('h1', 'zone-a', { podAntiAffinity: 'Hard' });
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [node],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).toContain('requiredDuringSchedulingIgnoredDuringExecution');
  });

  it('Soft podAntiAffinity produces preferredDuringSchedulingIgnoredDuringExecution', () => {
    const node = makeValidator('v1', 'zone-a', { podAntiAffinity: 'Soft' });
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [node],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).toContain('preferredDuringSchedulingIgnoredDuringExecution');
  });

  it('None podAntiAffinity produces no affinity block', () => {
    const node = makeValidator('v1', 'zone-a', { podAntiAffinity: 'None' });
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [node],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).not.toContain('podAntiAffinity:');
  });
});

// ---------------------------------------------------------------------------
// describe: buildManifests — validatorConfig
// ---------------------------------------------------------------------------

describe('buildManifests — validatorConfig', () => {
  it('validatorConfig block appears only for Validator nodeType', () => {
    // Build state with 1 Validator and 1 Horizon in same zone
    const zone = makeZone('zone-a', 'us-east-1a', ['worker-a']);
    const validator = makeValidator('v1', 'zone-a');
    const horizon = makeHorizon('h1', 'zone-a');

    const state: TopologyState = {
      zones: [zone],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [validator, horizon],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };

    const validatorYaml = buildNodeManifest(validator, state.zones, state.workerNodes);
    const horizonYaml = buildNodeManifest(horizon, state.zones, state.workerNodes);

    expect(validatorYaml).toContain('validatorConfig:');
    expect(horizonYaml).not.toContain('validatorConfig:');
  });

  it('includes seedSecretRef in the validatorConfig block', () => {
    const validator = makeValidator('v1', 'zone-a', {
      validatorConfig: {
        seedSecretRef: 'my-validator-seed',
        enableHistoryArchive: true,
        quorumSet: '[[QS]]',
      },
    });
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    expect(yaml).toContain('seedSecretRef: my-validator-seed');
  });

  it('renders quorumSet as a YAML literal block scalar', () => {
    const validator = makeValidator('v1', 'zone-a', {
      validatorConfig: {
        seedSecretRef: 'my-secret',
        enableHistoryArchive: true,
        quorumSet: '[[QUORUM_SET]]\nTHRESHOLD_PERCENT=67',
      },
    });
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    // Block scalar indicator
    expect(yaml).toContain('quorumSet: |');
    // The content should be present (possibly indented)
    expect(yaml).toContain('[[QUORUM_SET]]');
    expect(yaml).toContain('THRESHOLD_PERCENT=67');
  });

  it('renders historyArchiveUrls as YAML list items', () => {
    const validator = makeValidator('v1', 'zone-a', {
      validatorConfig: {
        seedSecretRef: 'my-secret',
        enableHistoryArchive: true,
        historyArchiveUrls: [
          'https://history.stellar.org/prd/core-live/core_live_001',
          'https://history.stellar.org/prd/core-live/core_live_002',
        ],
      },
    });
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    expect(yaml).toContain('historyArchiveUrls:');
    expect(yaml).toContain('- https://history.stellar.org/prd/core-live/core_live_001');
    expect(yaml).toContain('- https://history.stellar.org/prd/core-live/core_live_002');
  });
});

// ---------------------------------------------------------------------------
// describe: buildNodeManifest — labels and metadata
// ---------------------------------------------------------------------------

describe('buildNodeManifest — labels and metadata', () => {
  it('includes app.kubernetes.io/managed-by: topology-configurator', () => {
    const validator = makeValidator('v1', 'zone-a');
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    expect(yaml).toContain('app.kubernetes.io/managed-by: topology-configurator');
  });

  it('includes the node name in the manifest metadata', () => {
    const validator = makeValidator('v1', 'zone-a');
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    expect(yaml).toContain('name: validator-v1');
  });

  it('includes the stellar.org/node-type label', () => {
    const validator = makeValidator('v1', 'zone-a');
    const zones = [makeZone('zone-a', 'us-east-1a', ['worker-a'])];
    const yaml = buildNodeManifest(validator, zones, [makeWorkerNode('worker-a')]);
    expect(yaml).toContain('stellar.org/node-type: Validator');
  });
});

// ---------------------------------------------------------------------------
// describe: buildManifests — namespace handling
// ---------------------------------------------------------------------------

describe('buildManifests — namespace', () => {
  it('uses node namespace when no override is provided', () => {
    const node = makeValidator('v1', 'zone-a', { namespace: 'my-stellar-ns' });
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [node],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).toContain('namespace: my-stellar-ns');
  });

  it('defaults namespace to "stellar" when node namespace is stellar', () => {
    // The default namespace in our helper is 'stellar'
    const node = makeValidator('v1', 'zone-a');
    const state: TopologyState = {
      zones: [makeZone('zone-a', 'us-east-1a', ['worker-a'])],
      workerNodes: [makeWorkerNode('worker-a')],
      placedNodes: [node],
      selectedZoneId: null,
      draggedNodeType: null,
      isDirty: false,
    };
    const yaml = buildManifests(state);
    expect(yaml).toContain('namespace: stellar');
  });

  it('applies namespace override to all manifests when provided', () => {
    const state = makeThreeZoneState();
    const yaml = buildManifests(state, 'stellar-staging');
    // Every namespace reference should use the override
    expect(yaml).toContain('namespace: stellar-staging');
    // Original namespace 'stellar' should not appear in metadata
    const lines = yaml.split('\n').filter((l) => l.trim().startsWith('namespace:'));
    for (const line of lines) {
      expect(line).toContain('stellar-staging');
    }
  });
});

// ---------------------------------------------------------------------------
// describe: buildPodDisruptionBudget
// ---------------------------------------------------------------------------

describe('buildPodDisruptionBudget', () => {
  it('generates a PodDisruptionBudget manifest with the correct structure', () => {
    const node = makeValidator('v1', 'zone-a');
    const yaml = buildPodDisruptionBudget(node);
    expect(yaml).toContain('kind: PodDisruptionBudget');
    expect(yaml).toContain('apiVersion: policy/v1');
    expect(yaml).toContain('minAvailable: 1');
    expect(yaml).toContain('app.kubernetes.io/managed-by: topology-configurator');
  });

  it('PDB name is derived from the node name with -pdb suffix', () => {
    const node = makeValidator('v1', 'zone-a');
    const yaml = buildPodDisruptionBudget(node);
    expect(yaml).toContain('name: validator-v1-pdb');
  });
});
