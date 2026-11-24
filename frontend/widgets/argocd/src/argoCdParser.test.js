/**
 * argoCdParser.test.js
 *
 * Unit tests for the ArgoCD resource-tree parser.
 * Run with: node --test src/argoCdParser.test.js
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  categorize,
  extractStellarFinalizers,
  isTerminating,
  buildResolutionHint,
  flattenResourceTree,
  parseAppState,
  STELLAR_FINALIZERS,
} from './argoCdParser.js';

// ── Helpers ──────────────────────────────────────────────────────────────────

/**
 * Build a minimal ArgoResource fixture.
 * @param {Partial<import('./argoCdParser.js').ArgoResource>} overrides
 */
function makeResource(overrides = {}) {
  return {
    kind: 'Pod',
    name: 'test-pod',
    namespace: 'stellar-system',
    syncStatus: 'Synced',
    finalizers: [],
    deletionTimestamp: null,
    ...overrides,
  };
}

/**
 * Build a minimal ArgoCD Application response fixture.
 * @param {object} opts
 * @param {string} [opts.name]
 * @param {string} [opts.syncStatus]
 * @param {string} [opts.healthStatus]
 * @param {Array}  [opts.resources]
 */
function makeApp({ name = 'stellar-node-app', syncStatus = 'Synced', healthStatus = 'Healthy', resources = [] } = {}) {
  return {
    metadata: { name },
    status: {
      sync: { status: syncStatus },
      health: { status: healthStatus },
      resources,
    },
  };
}

// ── categorize() ─────────────────────────────────────────────────────────────

describe('categorize', () => {
  it('returns Pod for Pod kind', () => {
    assert.equal(categorize('Pod'), 'Pod');
  });

  it('returns PVC for PersistentVolumeClaim', () => {
    assert.equal(categorize('PersistentVolumeClaim'), 'PVC');
  });

  it('returns PV for PersistentVolume', () => {
    assert.equal(categorize('PersistentVolume'), 'PV');
  });

  it('returns StellarNode for StellarNode', () => {
    assert.equal(categorize('StellarNode'), 'StellarNode');
  });

  it('returns Unknown for any other kind', () => {
    assert.equal(categorize('ConfigMap'), 'Unknown');
    assert.equal(categorize('Service'), 'Unknown');
    assert.equal(categorize(''), 'Unknown');
  });
});

// ── extractStellarFinalizers() ────────────────────────────────────────────────

describe('extractStellarFinalizers', () => {
  it('returns only Stellar-K8s finalizers from a mixed list', () => {
    const input = [
      'stellarnode.k8s.stellar.org/pv-cleanup',
      'foregroundDeletion',
      'kubernetes.io/pvc-protection',
      'some-other-controller/finalizer',
    ];
    const result = extractStellarFinalizers(input);
    assert.deepEqual(result, [
      'stellarnode.k8s.stellar.org/pv-cleanup',
      'kubernetes.io/pvc-protection',
    ]);
  });

  it('returns empty array when no Stellar finalizers present', () => {
    assert.deepEqual(extractStellarFinalizers(['foregroundDeletion']), []);
  });

  it('returns all known Stellar finalizers if all present', () => {
    const result = extractStellarFinalizers(STELLAR_FINALIZERS);
    assert.deepEqual(result, STELLAR_FINALIZERS);
  });

  it('returns empty array for empty input', () => {
    assert.deepEqual(extractStellarFinalizers([]), []);
  });

  it('returns empty array for null/undefined input', () => {
    assert.deepEqual(extractStellarFinalizers(null), []);
    assert.deepEqual(extractStellarFinalizers(undefined), []);
  });
});

// ── isTerminating() ───────────────────────────────────────────────────────────

describe('isTerminating', () => {
  it('returns true when deletionTimestamp is set and finalizers exist', () => {
    const resource = makeResource({
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['kubernetes.io/pvc-protection'],
    });
    assert.equal(isTerminating(resource), true);
  });

  it('returns false when deletionTimestamp is set but finalizers list is empty', () => {
    const resource = makeResource({
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: [],
    });
    assert.equal(isTerminating(resource), false);
  });

  it('returns false when finalizers exist but deletionTimestamp is null', () => {
    const resource = makeResource({
      deletionTimestamp: null,
      finalizers: ['stellarnode.k8s.stellar.org/pv-cleanup'],
    });
    assert.equal(isTerminating(resource), false);
  });

  it('returns false when finalizers are absent', () => {
    const resource = makeResource({ deletionTimestamp: '2026-08-31T01:00:00Z' });
    delete resource.finalizers;
    assert.equal(isTerminating(resource), false);
  });

  it('returns false for a clean resource', () => {
    assert.equal(isTerminating(makeResource()), false);
  });
});

// ── buildResolutionHint() ─────────────────────────────────────────────────────

describe('buildResolutionHint', () => {
  it('gives PVC-specific advice for PersistentVolumeClaim', () => {
    const resource = makeResource({
      kind: 'PersistentVolumeClaim',
      name: 'data-pvc',
      namespace: 'stellar-system',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['kubernetes.io/pvc-protection'],
    });
    const hint = buildResolutionHint(resource, ['kubernetes.io/pvc-protection']);
    assert.match(hint, /kubectl patch pvc data-pvc -n stellar-system/);
    assert.match(hint, /pvc-protection/);
  });

  it('gives PV-specific advice for PersistentVolume', () => {
    const resource = makeResource({
      kind: 'PersistentVolume',
      name: 'pv-data',
      namespace: '',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['kubernetes.io/pv-protection'],
    });
    const hint = buildResolutionHint(resource, []);
    assert.match(hint, /kubectl patch pv pv-data/);
  });

  it('gives Pod-specific advice for Pod', () => {
    const resource = makeResource({
      kind: 'Pod',
      name: 'validator-0',
      namespace: 'stellar-system',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['stellarnode.k8s.stellar.org/peer-deregister'],
    });
    const hint = buildResolutionHint(resource, ['stellarnode.k8s.stellar.org/peer-deregister']);
    assert.match(hint, /kubectl delete pod validator-0/);
    assert.match(hint, /--force/);
  });

  it('gives StellarNode-specific advice mentioning Draining phase', () => {
    const resource = makeResource({
      kind: 'StellarNode',
      name: 'sn-mainnet-0',
      namespace: 'stellar-system',
      phase: 'Draining',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['stellarnode.k8s.stellar.org/network-drain'],
    });
    const hint = buildResolutionHint(resource, ['stellarnode.k8s.stellar.org/network-drain']);
    assert.match(hint, /Draining/);
    assert.match(hint, /network-drain/);
  });

  it('gives StellarNode-specific advice mentioning Deregistering phase', () => {
    const resource = makeResource({
      kind: 'StellarNode',
      name: 'sn-testnet-1',
      namespace: 'stellar-testnet',
      phase: 'Deregistering',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['stellarnode.k8s.stellar.org/peer-deregister'],
    });
    const hint = buildResolutionHint(resource, ['stellarnode.k8s.stellar.org/peer-deregister']);
    assert.match(hint, /deregistering/i);
  });

  it('gives a generic hint for unknown resource kinds', () => {
    const resource = makeResource({
      kind: 'ConfigMap',
      name: 'cfg',
      namespace: 'default',
      deletionTimestamp: '2026-08-31T01:00:00Z',
      finalizers: ['custom/finalizer'],
    });
    const hint = buildResolutionHint(resource, []);
    assert.match(hint, /kubectl patch configmap cfg -n default/);
  });
});

// ── flattenResourceTree() ─────────────────────────────────────────────────────

describe('flattenResourceTree', () => {
  it('returns empty array for empty input', () => {
    assert.deepEqual(flattenResourceTree([]), []);
  });

  it('returns empty array for null input', () => {
    assert.deepEqual(flattenResourceTree(null), []);
  });

  it('flattens a nested tree of resources', () => {
    const tree = [
      {
        kind: 'StellarNode',
        name: 'sn-0',
        namespace: 'stellar-system',
        children: [
          {
            kind: 'Pod',
            name: 'pod-0',
            namespace: 'stellar-system',
            children: [
              { kind: 'PersistentVolumeClaim', name: 'pvc-0', namespace: 'stellar-system' },
            ],
          },
        ],
      },
    ];
    const flat = flattenResourceTree(tree);
    assert.equal(flat.length, 3);
    const kinds = flat.map((r) => r.kind).sort();
    assert.deepEqual(kinds, ['PersistentVolumeClaim', 'Pod', 'StellarNode']);
  });

  it('handles nodes with no children field', () => {
    const tree = [
      { kind: 'Pod', name: 'a', namespace: 'ns' },
      { kind: 'Pod', name: 'b', namespace: 'ns' },
    ];
    const flat = flattenResourceTree(tree);
    assert.equal(flat.length, 2);
  });

  it('handles deeply nested trees efficiently', () => {
    // Build a chain 500 deep
    let node = { kind: 'Pod', name: 'leaf', namespace: 'ns' };
    for (let i = 0; i < 500; i++) {
      node = { kind: 'Pod', name: `pod-${i}`, namespace: 'ns', children: [node] };
    }
    const flat = flattenResourceTree([node]);
    assert.equal(flat.length, 501);
  });
});

// ── parseAppState() ───────────────────────────────────────────────────────────

describe('parseAppState', () => {
  it('parses a healthy synced application with no resources', () => {
    const result = parseAppState(makeApp());
    assert.equal(result.appName, 'stellar-node-app');
    assert.equal(result.syncStatus, 'Synced');
    assert.equal(result.healthStatus, 'Healthy');
    assert.equal(result.isStuck, false);
    assert.deepEqual(result.terminatingResources, []);
    assert.equal(result.totalResources, 0);
  });

  it('detects a terminating PVC in the resource list', () => {
    const resources = [
      makeResource({
        kind: 'PersistentVolumeClaim',
        name: 'data-pvc',
        namespace: 'stellar-system',
        syncStatus: 'OutOfSync',
        deletionTimestamp: '2026-08-31T01:00:00Z',
        finalizers: ['kubernetes.io/pvc-protection'],
      }),
    ];
    const result = parseAppState(makeApp({ syncStatus: 'OutOfSync', resources }));
    assert.equal(result.isStuck, true);
    assert.equal(result.terminatingResources.length, 1);
    const stuck = result.terminatingResources[0];
    assert.equal(stuck.kind, 'PersistentVolumeClaim');
    assert.equal(stuck.resourceCategory, 'PVC');
    assert.deepEqual(stuck.stellarFinalizers, ['kubernetes.io/pvc-protection']);
    assert.match(stuck.resolutionHint, /kubectl patch pvc data-pvc/);
  });

  it('detects a terminating StellarNode with multiple Stellar finalizers', () => {
    const resources = [
      makeResource({
        kind: 'StellarNode',
        name: 'sn-mainnet-0',
        namespace: 'stellar-system',
        phase: 'Draining',
        syncStatus: 'OutOfSync',
        deletionTimestamp: '2026-08-31T01:00:00Z',
        finalizers: [
          'stellarnode.k8s.stellar.org/network-drain',
          'stellarnode.k8s.stellar.org/pv-cleanup',
        ],
      }),
    ];
    const result = parseAppState(makeApp({ syncStatus: 'OutOfSync', healthStatus: 'Degraded', resources }));
    assert.equal(result.isStuck, true);
    const stuck = result.terminatingResources[0];
    assert.equal(stuck.stellarFinalizers.length, 2);
    assert.match(stuck.resolutionHint, /Draining/);
  });

  it('correctly reports synced vs out-of-sync counts', () => {
    const resources = [
      makeResource({ name: 'pod-1', syncStatus: 'Synced' }),
      makeResource({ name: 'pod-2', syncStatus: 'Synced' }),
      makeResource({ name: 'pod-3', syncStatus: 'OutOfSync' }),
    ];
    const result = parseAppState(makeApp({ resources }));
    assert.equal(result.totalResources, 3);
    assert.equal(result.syncedCount, 2);
    assert.equal(result.outOfSyncCount, 1);
  });

  it('handles a resource-tree API response (nodes at top level)', () => {
    const response = {
      name: 'stellar-mainnet',
      syncStatus: 'OutOfSync',
      healthStatus: 'Degraded',
      nodes: [
        makeResource({
          kind: 'Pod',
          name: 'validator-0',
          namespace: 'stellar-system',
          deletionTimestamp: '2026-08-31T01:00:00Z',
          finalizers: ['stellarnode.k8s.stellar.org/peer-deregister'],
        }),
      ],
    };
    const result = parseAppState(response);
    assert.equal(result.appName, 'stellar-mainnet');
    assert.equal(result.isStuck, true);
    assert.equal(result.terminatingResources[0].kind, 'Pod');
  });

  it('returns Unknown status for malformed response', () => {
    const result = parseAppState({});
    assert.equal(result.appName, 'unknown');
    assert.equal(result.syncStatus, 'Unknown');
    assert.equal(result.healthStatus, 'Unknown');
    assert.equal(result.isStuck, false);
  });

  it('does not include resources that are deleting but have no finalizers', () => {
    const resources = [
      makeResource({
        name: 'clean-pod',
        deletionTimestamp: '2026-08-31T01:00:00Z',
        finalizers: [],
      }),
    ];
    const result = parseAppState(makeApp({ resources }));
    assert.equal(result.isStuck, false);
    assert.equal(result.terminatingResources.length, 0);
  });

  it('handles large trees (100+ resources) without stack overflow', () => {
    // Simulate 120 resources as a flat list in status.resources
    const resources = Array.from({ length: 120 }, (_, i) =>
      makeResource({ name: `pod-${i}`, syncStatus: i % 3 === 0 ? 'OutOfSync' : 'Synced' }),
    );
    const result = parseAppState(makeApp({ resources }));
    assert.equal(result.totalResources, 120);
  });

  it('handles large nested trees (100+ resources) without stack overflow', () => {
    // Build tree: 1 StellarNode → 10 Pods → each with 11 PVCs = 121 leaf nodes
    const pods = Array.from({ length: 10 }, (_, p) => ({
      kind: 'Pod',
      name: `pod-${p}`,
      namespace: 'stellar-system',
      syncStatus: 'Synced',
      children: Array.from({ length: 11 }, (__, c) =>
        makeResource({
          kind: 'PersistentVolumeClaim',
          name: `pvc-${p}-${c}`,
          syncStatus: 'Synced',
        }),
      ),
    }));
    const resources = [
      { kind: 'StellarNode', name: 'sn-0', namespace: 'stellar-system', children: pods },
    ];
    const result = parseAppState(makeApp({ resources }));
    // 1 StellarNode + 10 Pods + 110 PVCs = 121
    assert.equal(result.totalResources, 121);
  });
});
