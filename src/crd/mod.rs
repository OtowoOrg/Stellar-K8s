//! Custom Resource Definitions for Stellar-K8s
//!
//! This module defines the Kubernetes CRDs for managing Stellar infrastructure.
//!
//! # Overview
//!
//! The primary CRD is [`StellarNode`], which represents a managed Stellar infrastructure node.
//! It supports three node types:
//! - **Validator**: Full Stellar Core validator participating in consensus
//! - **Horizon**: REST API server for querying the Stellar ledger
//! - **SorobanRpc**: Smart contract RPC node for Soroban interactions
//!
//! # Key Types
//!
//! - [`StellarNode`] - The main CRD resource
//! - [`StellarNodeSpec`] - Specification for desired node state
//! - [`StellarNodeStatus`] - Current status and conditions
//! - [`types`] - Shared configuration types (NodeType, StellarNetwork, etc.)
//! - [`ServiceMeshConfig`] - Istio/Linkerd integration
//! - [`ReadReplicaConfig`] - Read-only replica configuration
//! - [`seed_secret`] - Validator seed secret management
//!
//! # Validation
//!
//! All CRD specifications are validated through:
//! - **Schema validation**: Enforced by Kubernetes API server
//! - **Semantic validation**: Custom validation logic in [`StellarNodeSpec::validate`]
//! - **Webhook validation**: Optional WASM-based custom validators
//!
//! # Example: Creating a Validator
//!
//! ```yaml
//! apiVersion: stellar.org/v1alpha1
//! kind: StellarNode
//! metadata:
//!   name: my-validator
//!   namespace: stellar
//! spec:
//!   nodeType: Validator
//!   network: Testnet
//!   version: "v21.0.0"
//!   storage:
//!     storageClass: "standard"
//!     size: "100Gi"
//!   validatorConfig:
//!     seedSecretRef: "my-validator-seed"
//!     enableHistoryArchive: true
//! ```

mod cnpg;
pub mod read_replica;
pub mod schema_utils;
pub mod seed_secret;
pub mod service_mesh;
pub mod multi_region;
pub mod federation;
pub mod stellar_benchmark;
mod stellar_node;
pub mod types;

#[cfg(test)]
mod tests;

pub use cnpg::*;
pub use read_replica::{ReadReplicaConfig, ReadReplicaStrategy};
pub use service_mesh::{
    CircuitBreakerConfig, IstioMeshConfig, LinkerdMeshConfig, MtlsMode, RetryConfig,
    ServiceMeshConfig,
};
pub use stellar_benchmark::{
    BenchmarkConfig, BenchmarkMetrics, BenchmarkPhase, BenchmarkReport, BenchmarkReportSpec,
    BenchmarkReportStatus, BenchmarkResourceRequirements, BenchmarkSummary,
    EnvVar as BenchmarkEnvVar, PodResult, ResultStorage, StellarBenchmark, StellarBenchmarkSpec,
    StellarBenchmarkStatus, Toleration as BenchmarkToleration,
};
    ClusterConfig, ClusterHealthStatus, FailoverPolicy, MultiRegionConfig, MultiRegionHealthCheck,
    MultiRegionSpec, MultiRegionStatus, SecretSyncConfig,
};
pub use federation::{
    ClusterRegistry, ClusterRegistrySpec, ConflictResolutionStrategy, FederatedCluster,
    FederatedPlacement, FederatedStellarNode, FederatedStellarNodeSpec,
};
pub use stellar_node::{
    BGPStatus, SnapshotBootstrapStatus, SpecValidationError, StellarNode, StellarNodeSpec,
    StellarNodeStatus,
};
pub use types::*;
