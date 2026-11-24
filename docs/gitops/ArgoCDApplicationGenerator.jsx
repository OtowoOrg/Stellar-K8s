import React, { useMemo, useState } from 'react';

const REPO = 'https://github.com/agnesnaomiolim-cloud/Stellar-K8s.git';
const VERSION = 'v1';

function yamlQuote(value) {
  return JSON.stringify(value);
}

function safeName(value, fallback) {
  const normalized = value
    .toLowerCase()
    .replace(/[^a-z0-9-]/g, '-')
    .replace(/^-+|-+$/g, '');
  return (normalized || fallback).slice(0, 63).replace(/-+$/g, '') || fallback;
}

export default function ArgoCDApplicationGenerator() {
  const [namespace, setNamespace] = useState('stellar-testnet');
  const [storageClass, setStorageClass] = useState('stellar-local');
  const [nodeType, setNodeType] = useState('Validator');
  const [copied, setCopied] = useState(false);

  const result = useMemo(() => {
    const ns = safeName(namespace, 'stellar-testnet');
    const sc = safeName(storageClass, 'stellar-local');
    const slug = nodeType === 'Validator' ? 'testnet-validator' : 'soroban-rpc';
    const appName = `stellar-${slug}-${ns}`;
    const values =
      nodeType === 'Validator'
        ? `        nodeType: Validator\n        network: testnet\n        storageClass: ${yamlQuote(sc)}\n        seedSecret:\n          enabled: true\n          name: validator-seed-testnet\n          value: REPLACE_WITH_A_TESTNET_VALIDATOR_SECRET_KEY`
        : `        nodeType: SorobanRpc\n        network: testnet\n        storageClass: ${yamlQuote(sc)}\n        replicas: 2\n        soroban:\n          stellarCoreUrl: http://validator-testnet:11626`;

    return `apiVersion: argoproj.io/v1alpha1\nkind: Application\nmetadata:\n  name: ${appName}\n  namespace: argocd\n  labels:\n    stellar.org/gitops-version: ${VERSION}\n  finalizers:\n    - resources-finalizer.argocd.argoproj.io\nspec:\n  project: default\n  source:\n    repoURL: ${REPO}\n    targetRevision: main\n    path: examples/argocd/${VERSION}/node-chart\n    helm:\n      releaseName: ${appName}\n      values: |\n${values}\n  destination:\n    server: https://kubernetes.default.svc\n    namespace: ${ns}\n  syncPolicy:\n    automated:\n      prune: true\n      selfHeal: true\n    syncOptions:\n      - CreateNamespace=true\n      - ServerSideApply=true\n      - PruneLast=true\n      - PrunePropagationPolicy=foreground\n`;
  }, [namespace, storageClass, nodeType]);

  async function copy() {
    if (!navigator.clipboard) return;
    await navigator.clipboard.writeText(result);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <div className="argocd-generator" style={{ border: '1px solid #ddd', borderRadius: 8, padding: 16 }}>
      <label>
        Destination namespace{' '}
        <input value={namespace} onChange={(event) => setNamespace(event.target.value)} />
      </label>{' '}
      <label>
        StorageClass{' '}
        <input value={storageClass} onChange={(event) => setStorageClass(event.target.value)} />
      </label>{' '}
      <label>
        Node type{' '}
        <select value={nodeType} onChange={(event) => setNodeType(event.target.value)}>
          <option>Validator</option>
          <option>SorobanRpc</option>
        </select>
      </label>
      <p>
        <strong>Review before committing:</strong> namespace and StorageClass are
        cluster-specific; replace the Validator seed placeholder using an approved
        secret workflow.
      </p>
      <pre style={{ overflowX: 'auto' }}><code>{result}</code></pre>
      <button type="button" onClick={copy}>{copied ? 'Copied' : 'Copy Application YAML'}</button>
    </div>
  );
}
