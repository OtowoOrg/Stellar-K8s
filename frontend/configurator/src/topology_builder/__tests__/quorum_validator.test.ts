/**
 * Vitest unit tests for validateTopology (quorum_validator.ts).
 *
 * All TopologyState objects are built manually — no store or provider is used.
 * Tests cover every error and warning rule documented in quorum_validator.ts.
 */

import { describe, it, expect } from 'vitest';
import { validateTopology } from '../quorum_validator';
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

function makeWorkerNode(id: string, zone: string): WorkerNodeConfig {
  return {
    id,
    name: id,
    labels: { 'topology.kubernetes.io/zone': zone },
  };
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
    },
    ...overrides,
  };
}

function makeHorizon(id: string, zoneId: string): PlacedStellarNode {
  return {
    id,
    nodeType: 'Horizon',
    network: 'testnet',
    name: `horizon-${id}`,
    namespace: 'stellar',
    version: 'v2.31.0',
    replicas: 2,
    availabilityZoneId: zoneId,
    resources: { ...DEFAULT_RESOURCES },
    storage: { ...DEFAULT_STORAGE },
    maxUnavailable: 1,
    minAvailable: 1,
    podAntiAffinity: 'Soft',
  };
}

/**
 * Builds a minimal TopologyState with 3 zones and assigned worker nodes.
 * Each zone has one worker node pre-assigned.
 */
function makeBaseState(placedNodes: PlacedStellarNode[] = []): TopologyState {
  const zones = [
    makeZone('zone-a', 'us-east-1a', ['worker-a']),
    makeZone('zone-b', 'us-east-1b', ['worker-b']),
    makeZone('zone-c', 'us-east-1c', ['worker-c']),
  ];
  const workerNodes = [
    makeWorkerNode('worker-a', 'us-east-1a'),
    makeWorkerNode('worker-b', 'us-east-1b'),
    makeWorkerNode('worker-c', 'us-east-1c'),
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
// describe: no validators
// ---------------------------------------------------------------------------

describe('validateTopology — no validators', () => {
  it('returns valid with no errors when there are no placed nodes', () => {
    const state = makeBaseState([]);
    const result = validateTopology(state);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.warnings).toHaveLength(0);
  });

  it('returns valid with no errors when only Horizon nodes are placed', () => {
    const state = makeBaseState([
      makeHorizon('h1', 'zone-a'),
      makeHorizon('h2', 'zone-b'),
    ]);
    const result = validateTopology(state);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// describe: INSUFFICIENT_ZONES error
// ---------------------------------------------------------------------------

describe('validateTopology — INSUFFICIENT_ZONES', () => {
  it('returns INSUFFICIENT_ZONES error when validators are in only 2 zones', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-b'),
    ]);
    const result = validateTopology(state);
    expect(result.valid).toBe(false);
    const err = result.errors.find((e) => e.code === 'INSUFFICIENT_ZONES');
    expect(err).toBeDefined();
    expect(err!.message).toMatch(/at least 3/i);
  });

  it('returns INSUFFICIENT_ZONES error when validators are in only 1 zone', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-a'),
      makeValidator('v3', 'zone-a'),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'INSUFFICIENT_ZONES');
    expect(err).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// describe: QUORUM_BELOW_THRESHOLD error
// ---------------------------------------------------------------------------

describe('validateTopology — QUORUM_BELOW_THRESHOLD', () => {
  it('returns QUORUM_BELOW_THRESHOLD when only 1 validator replica total', () => {
    // Place 1 validator in zone-a with replicas=1 — total is 1, below threshold
    // Also set replicas=1 (default) — total replicas = 1
    const v = makeValidator('v1', 'zone-a', { replicas: 1 });
    // To isolate this rule we put validators in multiple zones to avoid
    // INSUFFICIENT_ZONES and SINGLE_ZONE_VALIDATORS firing exclusively
    const stateWith1Total: TopologyState = {
      ...makeBaseState([v]),
    };
    const result = validateTopology(stateWith1Total);
    const err = result.errors.find((e) => e.code === 'QUORUM_BELOW_THRESHOLD');
    expect(err).toBeDefined();
    expect(err!.message).toMatch(/1/);
  });

  it('does not return QUORUM_BELOW_THRESHOLD when total replicas are 3 or more', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', { replicas: 1 }),
      makeValidator('v2', 'zone-b', { replicas: 1 }),
      makeValidator('v3', 'zone-c', { replicas: 1 }),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'QUORUM_BELOW_THRESHOLD');
    expect(err).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: SINGLE_ZONE_VALIDATORS error
// ---------------------------------------------------------------------------

describe('validateTopology — SINGLE_ZONE_VALIDATORS', () => {
  it('returns SINGLE_ZONE_VALIDATORS error when all validators are in 1 zone', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', { replicas: 3 }),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'SINGLE_ZONE_VALIDATORS');
    expect(err).toBeDefined();
    expect(err!.zoneIds).toContain('zone-a');
  });

  it('does not return SINGLE_ZONE_VALIDATORS when validators span multiple zones', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-c'),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'SINGLE_ZONE_VALIDATORS');
    expect(err).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: valid 3-zone distribution
// ---------------------------------------------------------------------------

describe('validateTopology — valid 3-zone distribution', () => {
  it('returns no errors for 3 zones each with 1 validator and proper quorum config', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-c'),
    ]);
    const result = validateTopology(state);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// describe: ZONE_MISSING_VALIDATOR error
// ---------------------------------------------------------------------------

describe('validateTopology — ZONE_MISSING_VALIDATOR', () => {
  it('returns ZONE_MISSING_VALIDATOR when 3 zones but only 2 have validators', () => {
    // zone-c has workers assigned but no validators
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-b'),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'ZONE_MISSING_VALIDATOR');
    expect(err).toBeDefined();
    expect(err!.zoneIds).toContain('zone-c');
  });

  it('does not return ZONE_MISSING_VALIDATOR when all zones with workers have validators', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'),
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-c'),
    ]);
    const result = validateTopology(state);
    const err = result.errors.find((e) => e.code === 'ZONE_MISSING_VALIDATOR');
    expect(err).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: UNEVEN_DISTRIBUTION warning
// ---------------------------------------------------------------------------

describe('validateTopology — UNEVEN_DISTRIBUTION', () => {
  it('returns UNEVEN_DISTRIBUTION warning when zone1=5, zone2=1, zone3=1 validators', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', { replicas: 5 }),
      makeValidator('v2', 'zone-b', { replicas: 1 }),
      makeValidator('v3', 'zone-c', { replicas: 1 }),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'UNEVEN_DISTRIBUTION');
    expect(warn).toBeDefined();
    expect(warn!.message).toMatch(/4/); // max - min = 4
  });

  it('does not return UNEVEN_DISTRIBUTION when distribution difference is <= 2', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', { replicas: 3 }),
      makeValidator('v2', 'zone-b', { replicas: 2 }),
      makeValidator('v3', 'zone-c', { replicas: 2 }),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'UNEVEN_DISTRIBUTION');
    expect(warn).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: NO_HISTORY_ARCHIVE warning
// ---------------------------------------------------------------------------

describe('validateTopology — NO_HISTORY_ARCHIVE', () => {
  it('returns NO_HISTORY_ARCHIVE warning when no validator has enableHistoryArchive=true', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', {
        validatorConfig: {
          seedSecretRef: 'secret-v1',
          enableHistoryArchive: false,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
      makeValidator('v2', 'zone-b', {
        validatorConfig: {
          seedSecretRef: 'secret-v2',
          enableHistoryArchive: false,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
      makeValidator('v3', 'zone-c', {
        validatorConfig: {
          seedSecretRef: 'secret-v3',
          enableHistoryArchive: false,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'NO_HISTORY_ARCHIVE');
    expect(warn).toBeDefined();
    expect(warn!.message).toMatch(/enableHistoryArchive/);
  });

  it('does not return NO_HISTORY_ARCHIVE when at least one validator has enableHistoryArchive=true', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'), // makeValidator sets enableHistoryArchive: true by default
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-c'),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'NO_HISTORY_ARCHIVE');
    expect(warn).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: MISSING_QUORUM_SET warning
// ---------------------------------------------------------------------------

describe('validateTopology — MISSING_QUORUM_SET', () => {
  it('returns MISSING_QUORUM_SET warning when no validator has a quorumSet configured', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', {
        validatorConfig: {
          seedSecretRef: 'secret-v1',
          enableHistoryArchive: true,
          // quorumSet intentionally omitted
        },
      }),
      makeValidator('v2', 'zone-b', {
        validatorConfig: {
          seedSecretRef: 'secret-v2',
          enableHistoryArchive: true,
        },
      }),
      makeValidator('v3', 'zone-c', {
        validatorConfig: {
          seedSecretRef: 'secret-v3',
          enableHistoryArchive: true,
        },
      }),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'MISSING_QUORUM_SET');
    expect(warn).toBeDefined();
    expect(warn!.message).toMatch(/quorumSet/);
  });

  it('does not return MISSING_QUORUM_SET when at least one validator has a quorumSet', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a'), // makeValidator includes quorumSet by default
      makeValidator('v2', 'zone-b'),
      makeValidator('v3', 'zone-c'),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'MISSING_QUORUM_SET');
    expect(warn).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// describe: SEED_SECRET_MISSING warning
// ---------------------------------------------------------------------------

describe('validateTopology — SEED_SECRET_MISSING', () => {
  it('returns SEED_SECRET_MISSING warning when no validator has seedSecretRef', () => {
    const state = makeBaseState([
      makeValidator('v1', 'zone-a', {
        validatorConfig: {
          seedSecretRef: '',
          enableHistoryArchive: true,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
      makeValidator('v2', 'zone-b', {
        validatorConfig: {
          seedSecretRef: '',
          enableHistoryArchive: true,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
      makeValidator('v3', 'zone-c', {
        validatorConfig: {
          seedSecretRef: '',
          enableHistoryArchive: true,
          quorumSet: '[[QUORUM_SET]]',
        },
      }),
    ]);
    const result = validateTopology(state);
    const warn = result.warnings.find((w) => w.code === 'SEED_SECRET_MISSING');
    expect(warn).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// describe: ValidationResult shape
// ---------------------------------------------------------------------------

describe('validateTopology — result structure', () => {
  it('ValidationResult always has valid, errors, and warnings fields', () => {
    const state = makeBaseState([]);
    const result = validateTopology(state);
    expect(result).toHaveProperty('valid');
    expect(result).toHaveProperty('errors');
    expect(result).toHaveProperty('warnings');
    expect(Array.isArray(result.errors)).toBe(true);
    expect(Array.isArray(result.warnings)).toBe(true);
  });

  it('valid is false when there is at least one error', () => {
    // Only 1 validator → QUORUM_BELOW_THRESHOLD + INSUFFICIENT_ZONES + SINGLE_ZONE
    const state = makeBaseState([makeValidator('v1', 'zone-a')]);
    const result = validateTopology(state);
    expect(result.valid).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
  });

  it('valid is true when there are warnings but no errors', () => {
    // 3 zones, each with a validator, but none have history archive → warning only
    const noArchiveState = makeBaseState([
      makeValidator('v1', 'zone-a', {
        validatorConfig: {
          seedSecretRef: 'sec1',
          enableHistoryArchive: false,
          quorumSet: '[[QS]]',
        },
      }),
      makeValidator('v2', 'zone-b', {
        validatorConfig: {
          seedSecretRef: 'sec2',
          enableHistoryArchive: false,
          quorumSet: '[[QS]]',
        },
      }),
      makeValidator('v3', 'zone-c', {
        validatorConfig: {
          seedSecretRef: 'sec3',
          enableHistoryArchive: false,
          quorumSet: '[[QS]]',
        },
      }),
    ]);
    const result = validateTopology(noArchiveState);
    expect(result.valid).toBe(true);
    expect(result.errors).toHaveLength(0);
    expect(result.warnings.some((w) => w.code === 'NO_HISTORY_ARCHIVE')).toBe(true);
  });
});
