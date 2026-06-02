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
pub mod dr_policy;
pub mod federation;
pub mod multi_region;
pub mod read_replica;
pub mod schema_utils;
pub mod secret_policy;
pub mod seed_secret;
pub mod service_mesh;
pub mod stellar_autoscaler;
pub mod stellar_benchmark;
pub mod stellar_federation;
pub mod stellar_observability;
pub mod stellar_performance;
pub mod stellar_topology;
pub mod stellar_upgrade;
mod stellar_node;
pub mod tenant;
pub mod traffic_policy;
pub mod types;

#[cfg(test)]
mod tests;

pub use cnpg::*;
pub use dr_policy::{
    ComplianceStatus, DisasterRecoveryPolicy, DisasterRecoveryPolicySpec,
    DisasterRecoveryPolicyStatus,
};
pub use federation::{
    ClusterRegistry, ClusterRegistrySpec, ConflictResolutionStrategy, FederatedCluster,
    FederatedPlacement, FederatedStellarNode, FederatedStellarNodeSpec,
};
pub use multi_region::{
    ClusterConfig, ClusterHealthStatus, FailoverPolicy, MultiRegionConfig, MultiRegionHealthCheck,
    MultiRegionSpec, MultiRegionStatus, SecretSyncConfig,
};
pub use read_replica::{ReadReplicaConfig, ReadReplicaStrategy};
pub use secret_policy::{
    AwsKmsConfig, AzureKeyVaultConfig, GcpKmsConfig, KmsProvider, RotationPolicy,
    SecretAuditConfig, SecretPolicy, SecretPolicyCondition, SecretPolicyPhase, SecretPolicySpec,
    SecretPolicyStatus, SecretPolicySyncConfig, SyncConflictResolution,
};
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
pub use stellar_federation::{
    FederationCluster, ReplicationConfig, ReplicationMode, RoutingStrategy,
    StellarFederation, StellarFederationSpec, StellarFederationStatus, TrafficRoutingPolicy,
};
pub use stellar_autoscaler::{
    CanaryStrategy, CostAwareConfig, MetricType, PredictionModel, PredictiveScalingConfig,
    ScalingPolicy, ScalingStrategy, StellarAutoscaler, StellarAutoscalerSpec,
    StellarAutoscalerStatus, StellarMetric,
};
pub use stellar_performance::{
    BudgetResult, PerformanceBudgets, PerformancePhase, PerformanceSample, RegressionPolicy,
    StellarPerformance, StellarPerformanceSpec, StellarPerformanceStatus,
};
pub use stellar_topology::{
    StellarTopology, StellarTopologySpec, StellarTopologyStatus, TopologyPhase, TopologyValidator,
};
pub use stellar_upgrade::{
    CanaryStrategy as UpgradeCanaryStrategy, HealthValidation, RollbackPolicy, StellarUpgrade,
    StellarUpgradeSpec, StellarUpgradeStatus, UpgradePhase,
};
pub use stellar_observability::{
    AnomalyDetectionConfig, AnomalyModel, AnomalySensitivity, AlertingConfig, AlertRule,
    LoggingBackend, LoggingConfig, StellarObservability, StellarObservabilitySpec,
    StellarObservabilityStatus, TracingBackend, TracingConfig,
};
pub use stellar_node::{
    BGPStatus, SnapshotBootstrapStatus, SpecValidationError, StellarNode, StellarNodeSpec,
    StellarNodeStatus,
};
pub use traffic_policy::{
    AdaptiveRateLimitPolicy, CircuitBreakerPolicy, LeakyBucketPolicy, PriorityRule, QosClassPolicy,
    TokenBucketPolicy, TrafficPolicy, TrafficPolicySpec, TrafficPolicyStatus, TrafficPriorityClass,
};
pub use types::*;
