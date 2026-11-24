# Stellar-K8s ArgoCD Sync Status & Finalizer Tracking Widget

A specialized React widget that interfaces with the ArgoCD API to monitor `StellarNode` application sync states and identify Kubernetes resources stuck in a `Terminating` state due to Finalizers.

## Background & Problem

Stellar-K8s uses Kubernetes Finalizers (`stellarnode.k8s.stellar.org/*`, `kubernetes.io/pvc-protection`, etc.) to safely drain nodes and tear down Persistent Volumes without corrupting ledger data. When node teardown or PVC cleanup encounters an issue, GitOps engines like ArgoCD get blocked in an `OutOfSync` or `Degraded` state.

This widget surfaces those locks directly to cluster operators, identifies the exact blocking Finalizer, and provides contextual remediation commands.

## Features

- **ArgoCD API Integration**: Lightweight `ArgoCdPoller` with configurable interval, abortable requests, and error backoff.
- **Deep Resource Tree Traversal**: Iteratively and safely parses nested Application resource trees (tested on 100+ resource trees without stack overflow).
- **Stellar Finalizer Detection**: Highlights specific Stellar-K8s Finalizers:
  - `stellarnode.k8s.stellar.org/pv-cleanup`
  - `stellarnode.k8s.stellar.org/peer-deregister`
  - `stellarnode.k8s.stellar.org/config-sync`
  - `stellarnode.k8s.stellar.org/network-drain`
  - `storage.kubernetes.io/pv-protection`
  - `kubernetes.io/pvc-protection`
- **Contextual Resolution Hints**: Expandable actionable guidance with pre-formatted `kubectl` commands tailored to each resource kind (`Pod`, `PVC`, `PV`, `StellarNode`).
- **Zero-Dependency Core Parser**: Pure JavaScript functions with 100% test coverage via Node test runner.
- **Mock & Live Modes**: Switch seamlessly between mock data for testing and live ArgoCD server endpoints.

## Getting Started

### Development

```bash
cd frontend/widgets/argocd
npm install
npm run dev
```

Open `http://localhost:5175` to view the widget in demo mode.

### Running Unit Tests

```bash
npm test
```

### Configuration via URL Parameters

| Parameter | Default | Description |
|---|---|---|
| `mode` | `mock` | `live` or `mock` |
| `base` | `""` | ArgoCD API base URL (e.g., `https://argocd.stellar.internal`) |
| `token` | `""` | ArgoCD API Bearer JWT token |
| `poll` | `10000` | Polling interval in milliseconds |

Example:
```
http://localhost:5175/?mode=live&base=https://argocd.example.com&token=YOUR_TOKEN&poll=5000
```
