# Stellar-K8s Glossary

A comprehensive guide to common terms and concepts used throughout the Stellar-K8s project.

## Core Stellar Concepts

### Consensus Protocol
The mechanism by which Stellar nodes agree on the state of the ledger. Stellar uses the Stellar Consensus Protocol (SCP).

### Horizon
The REST API server for the Stellar network. It provides access to historical data, account information, and transaction submission. See [Horizon documentation](../examples/horizon.yaml).

### Ledger
A record of all accounts, balances, and transactions on the Stellar network. Each ledger is identified by a sequence number and contains a cryptographic hash of the previous ledger.

### Quorum Set
A set of validators that a node trusts to validate transactions. A node reaches consensus when its quorum set agrees on the ledger state. See [Quorum Optimization](./quorum-optimization.md).

### SCP (Stellar Consensus Protocol)
The Byzantine Fault Tolerant consensus algorithm used by Stellar. It allows nodes to reach agreement even when some nodes are faulty or malicious. See [SCP Analytics](./scp-analytics-pipeline.md).

### Stellar Core
The reference implementation of the Stellar protocol. It validates transactions, maintains the ledger, and participates in consensus.

### Soroban
Stellar's smart contract platform. Enables developers to write and deploy WebAssembly-based smart contracts on the Stellar network. See [Soroban RPC](../examples/soroban-rpc.yaml).

### Validator
A Stellar node that participates in consensus and helps validate transactions. Validators maintain a full copy of the ledger and vote on transaction validity.

## Kubernetes & Operator Concepts

### CRD (Custom Resource Definition)
A Kubernetes extension that defines custom resources. Stellar-K8s uses the `StellarNode` CRD to represent Stellar infrastructure. See [API Reference](./api-reference.md).

### Controller
The core reconciliation loop that watches for changes to Kubernetes resources and drives the cluster state to match the desired specification. See [Reconciler](../src/controller/reconciler.rs).

### Finalizer
A Kubernetes mechanism that prevents resource deletion until cleanup tasks are complete. Stellar-K8s uses finalizers to ensure PVCs and other resources are properly cleaned up. See [kube-rs Finalizers ADR](./adr/README.md).

### Operator
A Kubernetes application that extends the platform with domain-specific knowledge. The Stellar-K8s operator automates deployment and management of Stellar infrastructure.

### Reconciliation
The process of comparing desired state (defined in manifests) with actual state (in the cluster) and making changes to converge them. Runs continuously in the controller loop.

### StatefulSet
A Kubernetes workload for managing stateful applications. Stellar-K8s uses StatefulSets for validators that require persistent storage and stable network identities.

### Deployment
A Kubernetes workload for managing stateless applications. Stellar-K8s uses Deployments for Horizon and Soroban RPC nodes.

### PVC (Persistent Volume Claim)
A request for persistent storage in Kubernetes. Stellar nodes use PVCs to store ledger data and history archives.

## Stellar-K8s Specific Concepts

### Archive Pruning
The process of removing old history archive data to manage storage costs. See [Archive Pruning](./archive-pruning.md).

### Disk Scaling
Automatic expansion of storage volumes as the Stellar ledger grows. Prevents "Disk Full" outages without manual intervention. See [Proactive Disk Scaling](./proactive-disk-scaling.md).

### Health Check
Automated monitoring of node sync status. Stellar-K8s marks nodes as Ready only when fully synced with the network.

### History Archive
A complete record of all ledger states stored externally (typically S3). Used for node recovery and historical analysis.

### Ledger Sync
The process of a node downloading and validating all historical ledgers to catch up with the network. A node is "synced" when it has processed all ledgers up to the current network state.

### Reconciler
The main controller logic that handles resource creation, updates, and deletion. Implements the reconciliation loop for `StellarNode` resources.

### StellarNode
The custom Kubernetes resource that represents a Stellar node (Validator, Horizon, or Soroban RPC). Defined by the `StellarNode` CRD.

### Webhook
A Kubernetes admission controller that validates or mutates resources before they are stored. Stellar-K8s uses webhooks for custom validation policies. See [WASM Webhook](./wasm-webhook.md).

## Deployment & Operations

### Blue-Green Deployment
A deployment strategy where two identical environments (blue and green) run in parallel. Traffic switches from one to the other during updates. See [Blue-Green Deployments](./blue-green.md).

### Canary Deployment
A deployment strategy where new versions are rolled out to a small subset of nodes first, then gradually to all nodes if healthy. See [Canary Deployments](./canary-deployments.md).

### Disaster Recovery (DR)
Processes and tools for recovering from failures. Includes backup/restore procedures and automated failover. See [Cross-Cloud Failover](./cross-cloud-failover.md).

### Feature Flag
A runtime configuration that enables or disables functionality without redeploying. Stellar-K8s supports feature flags via ConfigMap. See [Runtime Feature Flags](../README.md#-runtime-feature-flags).

### GitOps
A deployment methodology where infrastructure is defined in Git and automatically applied to clusters. Stellar-K8s is compatible with ArgoCD and Flux.

### High Availability (HA)
A system design that minimizes downtime through redundancy and automatic failover. Stellar-K8s supports HA configurations with multiple replicas.

### Ledger Ingestion Lag
The delay between when a transaction is committed to the network and when a node processes it. Lower lag indicates better node performance.

### Metrics
Quantitative measurements of system behavior (e.g., CPU usage, transaction throughput). Stellar-K8s exports Prometheus metrics for monitoring. See [Monitoring & Observability](../README.md#-monitoring--observability).

### mTLS (Mutual TLS)
Encryption and authentication where both client and server verify each other's certificates. Stellar-K8s supports mTLS for secure inter-node communication.

### Pod Disruption Budget (PDB)
A Kubernetes policy that limits the number of pods that can be disrupted during maintenance. Protects Stellar nodes during cluster upgrades. See [Pod Disruption Budget](./pod-disruption-budget.md).

### Prometheus
An open-source monitoring and alerting system. Stellar-K8s exports metrics in Prometheus format for integration with monitoring stacks.

### Quorum Slice
A subset of a node's quorum set that is sufficient for that node to reach consensus. Different from the full quorum set.

### Reconciliation Loop
The continuous process where the controller watches for resource changes and applies them to the cluster. Runs indefinitely during operator execution.

### Replica
A copy of a pod or service. Multiple replicas provide redundancy and load distribution.

### Resource Limits
Kubernetes constraints on CPU and memory usage. Prevent pods from consuming excessive resources and impacting other workloads.

### Sidecar
An auxiliary container that runs alongside the main application container in a pod. Used for logging, monitoring, or configuration injection.

### Sync State
The current synchronization status of a node (e.g., "syncing", "synced", "catching up"). Stellar-K8s monitors and reports sync state.

### Topology
The network structure of Stellar validators and their trust relationships. See [SCP Topology Dashboard](./scp-topology-dashboard.md).

### Vertical Pod Autoscaler (VPA)
A Kubernetes tool that automatically adjusts CPU and memory requests based on actual usage. Stellar-K8s integrates with VPA for resource optimization.

## Development & Testing

### Fuzzing
Automated testing with random or malformed inputs to find edge cases and prevent panics. Stellar-K8s uses proptest for reconciler fuzzing. See [Fuzzing](./fuzzing.md).

### Integration Test
A test that verifies multiple components work together correctly. Stellar-K8s includes integration tests for end-to-end scenarios.

### Unit Test
A test that verifies a single function or module in isolation. Stellar-K8s includes comprehensive unit tests for all major components.

### E2E (End-to-End) Test
A test that verifies the entire system works from user perspective. Includes deployment, configuration, and operational scenarios.

## Security Concepts

### CVE (Common Vulnerabilities and Exposures)
A standardized identifier for known security vulnerabilities. Stellar-K8s includes CVE scanning in CI/CD. See [Security Scanning](../README.md#-runtime-feature-flags).

### Pod Security Standards (PSS)
Kubernetes policies that enforce security best practices (e.g., no root containers, read-only filesystems). See [Pod Security Standards](./security/pss.md).

### RBAC (Role-Based Access Control)
A Kubernetes authorization mechanism that controls who can perform which actions on resources. Stellar-K8s includes RBAC configuration in Helm charts.

### Secret
A Kubernetes object for storing sensitive data (passwords, keys, tokens). Stellar-K8s uses secrets for validator seeds and API credentials.

### Secret Rotation
The process of periodically updating secrets to reduce exposure risk. See [Secret Rotation](./secret-rotation.md).

---

## Cross-References

- **Deployment Examples**: See [examples/](../examples/) directory for sample manifests
- **API Reference**: See [api-reference.md](./api-reference.md) for CRD field documentation
- **Architecture Decisions**: See [adr/](./adr/) for design rationale
- **Troubleshooting**: See [troubleshooting/](./troubleshooting/) for common issues
