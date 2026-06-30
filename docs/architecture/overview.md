# Architecture Overview

The Stellar-K8s architecture is centered on a Kubernetes-native operator that manages Stellar Core clusters through `StellarNode` custom resources.

## Core Components

- `stellar-operator` deployment: control plane, reconciliation loop, REST API, metrics, and health probes.
- `StellarNode` CRD: defines cluster topology, network settings, storage, and runtimes.
- Admission webhook: validates and mutates incoming `StellarNode` resources.
- Managed workloads: Stellar Core pods, optional sidecars, and service mesh proxies.
- Monitoring stack: Prometheus scrapes operator, webhook, and workload metrics.

## Integration Points

- Kubernetes API Server: stores CRDs and orchestrates pod lifecycle.
- External systems: CI/CD, dashboards, automation tools, and observability platforms.
- Optional service mesh: provides mTLS and traffic policy enforcement for Stellar workloads.

## Design Principles

- Declarative infrastructure through CRDs.
- Strong validation with admission webhooks.
- Observability through Prometheus-compatible metrics.
- Stable upgrades through versioned API and documentation.
- Separation of control plane (`stellar-operator`) and data plane (managed Stellar workloads).

## Architecture Goals

- Make Stellar Core easier to run on Kubernetes.
- Provide clear, version-controlled architecture documentation.
- Enable operations teams with runbooks, monitoring guidance, and incident response procedures.
