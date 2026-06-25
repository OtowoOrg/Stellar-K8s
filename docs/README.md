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

## CRD & API Reference

- [StellarNode API Reference](api-reference.md) — all CRD fields, types, defaults, examples
- [API Index](api/index.md)
- [Error Codes](errors.md)

## Configuration

- [Resource Limits](resource-limits.md)
- [Storage & PVC Auto-Expansion](pvc-auto-expansion.md)
- [Proactive Disk Scaling](proactive-disk-scaling.md)
- [Sync-State Scaling](sync-state-scaling.md)
- [Feature Flags](../README.md#️-runtime-feature-flags)
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
- [Fuzzing](fuzzing.md)
- [Docker Compose → Kubernetes Migration](docker-compose-to-kubernetes-migration.md)

## Reference

- [Glossary](glossary.md)
- [FAQ](faq.md)
- [CHANGELOG](../CHANGELOG.md)
