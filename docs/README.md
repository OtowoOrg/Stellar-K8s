# Stellar-K8s Documentation

Production-grade Stellar infrastructure on Kubernetes. This directory contains all operator documentation.

---

## Getting Started

- [Prerequisites](getting-started/prerequisites.md)
- [Installation](getting-started/installation.md)
- [Quick Start](getting-started/quick-start.md)
- [WSL2 Installation](installation-wsl2.md)

## Deployment Guides

- [Validator Node](deployment-guides/validator.md)
- [Horizon API](deployment-guides/horizon.md)
- [Soroban RPC](deployment-guides/soroban-rpc.md)
- [OLM / OpenShift](deploy-olm.md)
- [Deployment Patterns](deployment-patterns/index.md)
- [Horizon / RPC Farm](deployment-patterns/horizon-rpc-farm.md)
- [Multi-Region and DR](deployment-patterns/multi-region-dr.md)
- [GitOps, Scaling and Upgrades](deployment-patterns/gitops-scaling-upgrades.md)

## CRD & API Reference

- [StellarNode API Reference](api-reference.md) — all CRD fields, types, defaults, examples
- [API Index](api/index.md)
- [Error Codes](errors.md)

## Configuration

- [Resource Limits](resource-limits.md)
- [Storage & PVC Auto-Expansion](pvc-auto-expansion.md)
- [Proactive Disk Scaling](proactive-disk-scaling.md)
- [Sync-State Scaling](sync-state-scaling.md)
- [Feature Flags](../README.md#runtime-feature-flags)
- [Ingress Guide](ingress-guide.md)
- [Network Policies](network-isolation.md)
- [Network Policy Templates](network-policy-templates.md)

## Operations

- [Health Checks](health-checks.md)
- [Peer Discovery](peer-discovery.md)
- [Archive Pruning](archive-pruning.md)
- [Volume Snapshots](volume-snapshots.md)
- [Diff Utility](diff-utility.md)
- [Upgrade Workflow](upgrade-workflow.md)
- [Disaster Recovery & Failover](dr-failover.md)
- [Cross-Cloud Failover](cross-cloud-failover.md)
- [Pod Disruption Budgets](pod-disruption-budget.md)
- [Backup Verification](backup-verification.md)
- [Operations Runbook](operations/index.md)
- [Incident Response](operations/incident-response.md)

## Observability

- [Observability Overview](observability/index.md)
- [Metric Reference](observability/metric-reference.md)
- [Metric Naming Conventions](observability/metric-naming-conventions.md)
- [Metrics and Dashboards](observability/metrics-and-dashboards.md)
- [Telemetry Architecture](observability/telemetry-architecture.md)
- [Unified Observability Contract](observability/unified-contract.md)
- [OpenTelemetry Tracing](observability/tracing.md)
- [Metrics Guide](metrics/STELLAR_METRICS_GUIDE.md)
- [Grafana Dashboard Guide](monitoring/GRAFANA_DASHBOARD_GUIDE.md)
- [SCP Analytics Pipeline](scp-analytics-pipeline.md)
- [SCP Topology Dashboard](scp-topology-dashboard.md)
- [Byzantine Monitoring](byzantine-monitoring.md)
- [Canary Deployments](canary-deployments.md)

## Security

- [Credentials and Secrets (Central Reference)](security/credentials-and-secrets.md)
- [Secret Management Guide](secret-management-guide.md)
- [Secret Rotation](secret-rotation.md)
- [Secret Management (KMS)](secret-management-kms.md)
- [Vault Tutorial](vault-stellar-tutorial.md)
- [Production Security Hardening](production-security-hardening.md)
- [mTLS Guide](mtls-guide.md)
- [Gatekeeper Policies](gatekeeper-policies.md)
- [Pod Security Standards](security/pss.md)
- [Image Pinning](image-pinning.md)
- [ZK Archive Verification](zk-archive-verification.md)

## CLI & Plugins

- [kubectl-stellar Plugin](kubectl-plugin.md)
- [Interactive Mode Guide](kubectl-plugin/INTERACTIVE_MODE_GUIDE.md)
- [CLI Commands Reference](cli-commands-reference.md)
- [Shell Completions](../README.md#shell-completions)

## Architecture & Design

- [Architecture Overview](architecture.md)
- [ADR Index](adr/README.md)
- [Formal Verification](FORMAL_VERIFICATION.md)
- [FMEA](fmea-stellarnode.md)
- [Service Mesh](service-mesh.md)
- [Multi-Cluster](multi-cluster.md)
- [Network Topology Management](network-topology-management.md)

## Performance & Benchmarking

- [Benchmarking Guide](benchmarking.md)
- [Multi-Cluster Benchmark Compare](benchmark-compare.md)
- [Performance Tuning](performance-tuning.md)
- [Scalability](scalability.md)
- [Resource Optimization](resource-optimization.md)

## Contributing & Development

- [Contributing Guide](../CONTRIBUTING.md)
- [Development Guide](../DEVELOPMENT.md)
- [Repository Conventions](../CONVENTIONS.md)
- [Development Setup](contributing/development-setup.md)
- [Developer Onboarding](developer-onboarding/index.md)
- [Development Reference](development.md)
- [Build Targets](development/build-targets.md)
- [Regenerating Charts & Bundle Manifests](development/regeneration-guide.md)
- [Fuzzing](fuzzing.md)
- [Docker Compose → Kubernetes Migration](docker-compose-to-kubernetes-migration.md)
- [First-Class Issue Types](first-class-issues.md)

## Reference

- [Glossary](glossary.md)
- [FAQ](faq.md)
- [CHANGELOG](../CHANGELOG.md)
- [Third-Party Licenses](../THIRD_PARTY_LICENSES.md)

---
## Navigation

The canonical documentation index is maintained in [`mkdocs.yml`](../mkdocs.yml) — the single navigation source for the rendered MkDocs site. Use that file to find all documentation pages organized by topic.

For a quick reference, key topic areas include:
- Getting Started, Deployment Guides, Configuration, Tutorials
- Operations & Observability, Troubleshooting
- API Reference, Security, Networking
- Development, Contributing
- PITCH Storyboards ([`pitch-storyboards.md`](./pitch-storyboards.md))
- Change Notes ([`change-notes.md`](./change-notes.md))

## Documentation Maintenance

**Owner:** Any contributor — this README is maintained alongside `mkdocs.yml`.

### Adding a new doc

1. Create your `.md` file under the appropriate `docs/` subdirectory.
2. Add a corresponding entry to `mkdocs.yml` under the relevant section.

### Updating an existing doc

- Keep links relative (e.g. `../README.md`, `./api-reference.md`).
- Run `make link-check` to catch broken links before opening a PR.

### Removing a doc

1. Delete the file.
2. Remove its entry from `mkdocs.yml`.
3. Search for cross-references: `grep -r "filename.md" docs/` and fix or remove them.

### Generating auto-derived docs

Some files are generated — do not edit them by hand:

| File | How to regenerate |
|------|------------------|
| `docs/api-reference.md` | `make generate-api-docs` |
| `completions/` | `make completions` |

Run `make health` to verify formatting, linting, tests, and docs drift in one command.
