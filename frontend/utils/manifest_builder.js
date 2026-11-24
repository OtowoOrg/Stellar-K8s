/**
 * JavaScript implementation of Kubernetes Manifest Builder for StellarNode Topology Spread Constraints.
 * Supports direct execution in Node.js ESM environment.
 */

export function buildTopologySpreadConstraints(settings) {
  const matchLabelKey = (settings && settings.matchLabelKey) || 'app.kubernetes.io/name';
  const matchLabelValue = (settings && settings.matchLabelValue) || 'stellarnode';

  return [
    {
      maxSkew: (settings && settings.maxSkew) ?? 1,
      topologyKey: (settings && settings.topologyKey) || 'topology.kubernetes.io/zone',
      whenUnsatisfiable: (settings && settings.whenUnsatisfiable) || 'DoNotSchedule',
      labelSelector: {
        matchLabels: {
          [matchLabelKey]: matchLabelValue,
        },
      },
    },
  ];
}

export function buildStellarNodeManifestObject(node, spreadSettings, namespace = 'stellar') {
  const safeName = (node.name || `node-${node.id}`)
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '-');

  return {
    apiVersion: 'stellar.org/v1alpha1',
    kind: 'StellarNode',
    metadata: {
      name: safeName,
      namespace: namespace || 'stellar',
      labels: {
        'app.kubernetes.io/name': 'stellarnode',
        'app.kubernetes.io/component': (node.nodeType || 'validator').toLowerCase(),
        'topology.kubernetes.io/zone': node.zone,
      },
    },
    spec: {
      nodeType: node.nodeType || 'Validator',
      network: node.network || 'mainnet',
      version: node.version || 'v21.0.0',
      replicas: 1,
      podAntiAffinity: 'Hard',
      topologySpreadConstraints: buildTopologySpreadConstraints(spreadSettings),
      storage: {
        mode: 'PersistentVolume',
        size: node.nodeType === 'Validator' ? '500Gi' : '100Gi',
        storageClass: 'ssd-premium',
      },
    },
  };
}

export function dumpYaml(obj, indentLevel = 0) {
  const spaces = ' '.repeat(indentLevel);

  if (obj === null || obj === undefined) {
    return 'null';
  }

  if (typeof obj === 'boolean' || typeof obj === 'number') {
    return String(obj);
  }

  if (typeof obj === 'string') {
    if (obj.includes('\n')) {
      return `|\n` + obj.split('\n').map(line => spaces + '  ' + line).join('\n');
    }
    if (/[:#\[\]\{\},&\*!|\>'"%@`]/.test(obj) || obj.trim() !== obj) {
      return `"${obj.replace(/"/g, '\\"')}"`;
    }
    return obj;
  }

  if (Array.isArray(obj)) {
    if (obj.length === 0) return '[]';
    return obj
      .map(item => {
        if (typeof item === 'object' && item !== null) {
          const itemYaml = dumpYaml(item, indentLevel + 2);
          const lines = itemYaml.split('\n');
          const firstLine = `${spaces}- ${lines[0].trimStart()}`;
          const remainingLines = lines.slice(1);
          return [firstLine, ...remainingLines].join('\n');
        } else {
          return `${spaces}- ${dumpYaml(item, 0)}`;
        }
      })
      .join('\n');
  }

  if (typeof obj === 'object') {
    const keys = Object.keys(obj);
    if (keys.length === 0) return '{}';
    return keys
      .map(key => {
        const val = obj[key];
        if (typeof val === 'object' && val !== null) {
          if (Array.isArray(val) && val.length === 0) {
            return `${spaces}${key}: []`;
          }
          if (!Array.isArray(val) && Object.keys(val).length === 0) {
            return `${spaces}${key}: {}`;
          }
          return `${spaces}${key}:\n${dumpYaml(val, indentLevel + 2)}`;
        }
        return `${spaces}${key}: ${dumpYaml(val, 0)}`;
      })
      .join('\n');
  }

  return String(obj);
}

export function generateTopologyManifestYaml(state) {
  if (!state.nodes || state.nodes.length === 0) {
    return '# No nodes placed in topology configuration.\n';
  }

  const namespace = state.namespace || 'stellar';
  const manifestObjects = state.nodes.map(node =>
    buildStellarNodeManifestObject(node, state.spreadSettings, namespace)
  );

  if (manifestObjects.length === 1) {
    return dumpYaml(manifestObjects[0]) + '\n';
  }

  return manifestObjects.map(obj => dumpYaml(obj)).join('\n---\n') + '\n';
}
