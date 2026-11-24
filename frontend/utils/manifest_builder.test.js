import { describe, it } from 'node:test';
import assert from 'node:assert';
import {
  buildTopologySpreadConstraints,
  buildStellarNodeManifestObject,
  generateTopologyManifestYaml,
} from './manifest_builder.js';
import { validateTopologyQuorum } from '../configurator/topology_builder/quorum_validator.js';

describe('Manifest Builder Utilities', () => {
  it('should build valid Kubernetes topologySpreadConstraints array', () => {
    const settings = {
      maxSkew: 1,
      topologyKey: 'topology.kubernetes.io/zone',
      whenUnsatisfiable: 'DoNotSchedule',
    };

    const constraints = buildTopologySpreadConstraints(settings);
    assert.strictEqual(constraints.length, 1);
    assert.strictEqual(constraints[0].maxSkew, 1);
    assert.strictEqual(constraints[0].topologyKey, 'topology.kubernetes.io/zone');
    assert.strictEqual(constraints[0].whenUnsatisfiable, 'DoNotSchedule');
    assert.deepStrictEqual(constraints[0].labelSelector.matchLabels, {
      'app.kubernetes.io/name': 'stellarnode',
    });
  });

  it('should build StellarNode Custom Resource manifest object', () => {
    const node = {
      id: 'node-1',
      name: 'validator-mainnet-a',
      nodeType: 'Validator',
      zone: 'us-east-1a',
      network: 'mainnet',
      version: 'v21.0.0',
    };
    const settings = {
      maxSkew: 1,
      topologyKey: 'topology.kubernetes.io/zone',
      whenUnsatisfiable: 'DoNotSchedule',
    };

    const manifestObj = buildStellarNodeManifestObject(node, settings, 'stellar');

    assert.strictEqual(manifestObj.apiVersion, 'stellar.org/v1alpha1');
    assert.strictEqual(manifestObj.kind, 'StellarNode');
    assert.strictEqual(manifestObj.metadata.name, 'validator-mainnet-a');
    assert.strictEqual(manifestObj.metadata.namespace, 'stellar');
    assert.strictEqual(manifestObj.metadata.labels['topology.kubernetes.io/zone'], 'us-east-1a');
    assert.strictEqual(manifestObj.spec.nodeType, 'Validator');
    assert.strictEqual(manifestObj.spec.network, 'mainnet');
    assert.strictEqual(manifestObj.spec.replicas, 1);
    assert.strictEqual(manifestObj.spec.podAntiAffinity, 'Hard');
    assert.strictEqual(manifestObj.spec.topologySpreadConstraints[0].maxSkew, 1);
  });

  it('should generate valid multi-document YAML for multi-zone node topology', () => {
    const state = {
      zones: [
        { id: 'zone-a', name: 'zone-a' },
        { id: 'zone-b', name: 'zone-b' },
        { id: 'zone-c', name: 'zone-c' },
      ],
      nodes: [
        { id: '1', name: 'val-a', nodeType: 'Validator', zone: 'zone-a' },
        { id: '2', name: 'val-b', nodeType: 'Validator', zone: 'zone-b' },
        { id: '3', name: 'val-c', nodeType: 'Validator', zone: 'zone-c' },
      ],
      spreadSettings: {
        maxSkew: 1,
        topologyKey: 'topology.kubernetes.io/zone',
        whenUnsatisfiable: 'DoNotSchedule',
      },
      namespace: 'stellar',
    };

    const yamlOutput = generateTopologyManifestYaml(state);

    assert.ok(yamlOutput.includes('apiVersion: stellar.org/v1alpha1'));
    assert.ok(yamlOutput.includes('kind: StellarNode'));
    assert.ok(yamlOutput.includes('name: val-a'));
    assert.ok(yamlOutput.includes('name: val-b'));
    assert.ok(yamlOutput.includes('name: val-c'));
    assert.ok(yamlOutput.includes('topologySpreadConstraints:'));
    assert.ok(yamlOutput.includes('topology.kubernetes.io/zone'));
    assert.ok(yamlOutput.includes('---')); // Multi-document separator
  });
});

describe('Real-Time Quorum Redundancy Validation', () => {
  const zones = [
    { id: 'zone-a', name: 'Zone A' },
    { id: 'zone-b', name: 'Zone B' },
    { id: 'zone-c', name: 'Zone C' },
  ];

  it('should validate a balanced 3-zone validator layout as quorum safe', () => {
    const nodes = [
      { id: '1', name: 'val-a', nodeType: 'Validator', zone: 'zone-a' },
      { id: '2', name: 'val-b', nodeType: 'Validator', zone: 'zone-b' },
      { id: '3', name: 'val-c', nodeType: 'Validator', zone: 'zone-c' },
    ];

    const result = validateTopologyQuorum(zones, nodes, { maxSkew: 1 });

    assert.strictEqual(result.isValid, true);
    assert.strictEqual(result.quorumSafe, true);
    assert.strictEqual(result.activeZonesCount, 3);
    assert.strictEqual(result.totalValidators, 3);
    assert.strictEqual(result.skew, 0);
    assert.strictEqual(result.errors.length, 0);
  });

  it('should flag layout spanning less than 3 zones as a quorum risk', () => {
    const nodes = [
      { id: '1', name: 'val-a', nodeType: 'Validator', zone: 'zone-a' },
      { id: '2', name: 'val-b', nodeType: 'Validator', zone: 'zone-b' },
    ];

    const result = validateTopologyQuorum(zones, nodes, { maxSkew: 1 });

    assert.strictEqual(result.isValid, false);
    assert.strictEqual(result.quorumSafe, false);
    assert.strictEqual(result.activeZonesCount, 2);
    assert.ok(result.errors.some(e => e.includes('spans only 2 zone(s)')));
  });

  it('should flag majority concentration hazard when 1 zone has >= 50% of validators', () => {
    const nodes = [
      { id: '1', name: 'val-a1', nodeType: 'Validator', zone: 'zone-a' },
      { id: '2', name: 'val-a2', nodeType: 'Validator', zone: 'zone-a' },
      { id: '3', name: 'val-b', nodeType: 'Validator', zone: 'zone-b' },
      { id: '4', name: 'val-c', nodeType: 'Validator', zone: 'zone-c' },
    ];

    const result = validateTopologyQuorum(zones, nodes, { maxSkew: 1 });

    assert.strictEqual(result.isValid, false);
    assert.strictEqual(result.quorumSafe, false);
    assert.ok(result.errors.some(e => e.includes("Zone 'zone-a' contains 2/4 (50%) of validator nodes")));
  });

  it('should calculate topology skew correctly across zones', () => {
    const nodes = [
      { id: '1', name: 'val-a1', nodeType: 'Validator', zone: 'zone-a' },
      { id: '2', name: 'val-a2', nodeType: 'Validator', zone: 'zone-a' },
      { id: '3', name: 'val-a3', nodeType: 'Validator', zone: 'zone-a' },
      { id: '4', name: 'val-b', nodeType: 'Validator', zone: 'zone-b' },
      { id: '5', name: 'val-c', nodeType: 'Validator', zone: 'zone-c' },
    ];

    const result = validateTopologyQuorum(zones, nodes, { maxSkew: 1 });

    assert.strictEqual(result.skew, 2); // 3 nodes in zone-a vs 1 in zone-b/c
    assert.ok(result.warnings.some(w => w.includes('Node distribution skew is 2')));
  });
});
