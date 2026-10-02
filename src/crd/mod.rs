// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
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

#[cfg(test)]
mod blue_green_schema_test;
#[cfg(test)]
mod secret_rotation_schema_test;
pub mod capacity_forecast;
pub mod contract_deployment;
mod cnpg;
pub mod control_plane_health;
pub mod dr_policy;
pub mod federation;
pub mod internal_api_schema;
pub mod job_retention;
pub mod maintenance_plan;
pub mod multi_region;
pub mod progressive_delivery;
pub mod read_replica;
pub mod schema_utils;
pub mod secret_policy;
pub mod seed_secret;
pub mod service_mesh;
pub mod stellar_asset_monitor;
pub mod stellar_autoscaler;
pub mod stellar_benchmark;
pub mod stellar_federation;
pub mod stellar_network_policy;
mod stellar_node;
pub mod stellar_observability;
pub mod stellar_performance;
pub mod stellar_topology;
pub mod stellar_upgrade;
pub mod tenant;
pub mod traffic_policy;
pub mod traffic_shift_plan;
pub mod types;
pub use tenant::{
    TenantBillingSpec, TenantCondition, TenantNetworkIsolation, TenantQuotaHard, TenantSpec,
    TenantSpecCrd, TenantStatus, TenantUsageCrd, TenantUsageSpec, TenantUsageStatus,
};

// New Epic CRDs (Wave 5)
pub mod compliance_report;
pub mod incident;
pub mod multisig_operation;
pub mod service_ownership;
pub mod stellar_aiops;
pub mod stellar_database;
pub mod stellar_disaster_recovery;
pub mod stellar_gitops;
pub mod stellar_policy_bundle;
pub mod stellar_registry;
pub mod stellar_security;
pub mod validator_score;

pub mod namespace_teardown;

// Issue #1577 â€” Webhook Event Delivery for Ledger Close Notifications
pub mod ledger_close_webhook;

#[cfg(test)]
mod tests;

pub use capacity_forecast::{
    BacktestReport, CapacityDimension, CapacityForecastSummary, CapacityRecommendationReport,
    CapacityRecommendationReportSpec, CapacityRecommendationReportStatus, ForecastInterval,
    ForecastModelKind, IncidentBacktest, RecommendationPhase, RecommendationPriority,
    ScalingRecommendation, SeriesBacktest, TimeToExhaustion,
};
pub use contract_deployment::{
    ContractDeploymentPhase, ContractInstance, ContractInstanceSpec, ContractInstanceStatus,
    ContractStorageEntry, ContractWASM, ContractWASMSpec, ContractWASMStatus, StorageDurability,
};
pub use cnpg::*;
pub use control_plane_health::{
    ComponentState, ComponentStatus, ControlPlaneComponent, ControlPlaneHealth,
    ControlPlaneHealthSpec, ControlPlaneHealthStatus, DegradationLevel, IncidentReport,
    LevelTransition, PermittedActions, WebhookMode,
};
pub use dr_policy::{
    ComplianceStatus, DisasterRecoveryPolicy, DisasterRecoveryPolicySpec,
    DisasterRecoveryPolicyStatus,
};
pub use federation::{
    ClusterRegistry, ClusterRegistrySpec, ConflictResolutionStrategy, FederatedCluster,
    FederatedPlacement, FederatedStellarNode, FederatedStellarNodeSpec,
};
pub use internal_api_schema::{
    ConsumerDeploymentPolicy, InternalApiSchema, InternalApiSchemaSpec, InternalApiSchemaStatus,
    SchemaDeploymentPhase,
};
pub use job_retention::{
    JobRetentionPolicy, JobRetentionPolicySpec, JobRetentionPolicyStatus, ReclaimedByClass,
};
pub use maintenance_plan::{
    AbortPolicy, DrainConfig, MaintenanceIntent, MaintenancePhase, MaintenancePlan,
    MaintenancePlanSpec, MaintenancePlanStatus, MaintenanceTarget, PdbConfig, PrewarmConfig,
    SloVerification, StallRecovery,
};
pub use multi_region::{
    ClusterConfig, ClusterHealthStatus, FailoverPolicy, MultiRegionConfig, MultiRegionHealthCheck,
    MultiRegionSpec, MultiRegionStatus, SecretSyncConfig,
};
pub use progressive_delivery::{
    GateResult, ProgressiveDelivery, ProgressiveDeliverySpec, ProgressiveDeliveryStatus,
    PromotionPhase, SloGate, TrafficSplit, WeightProgression,
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
pub use stellar_asset_monitor::{AssetWatch, StellarAssetMonitor, StellarAssetMonitorSpec};
pub use stellar_autoscaler::{
    CostAwareConfig, MetricType, PredictionModel, PredictiveScalingConfig, ScalingPolicy,
    ScalingStrategy, StellarAutoscaler, StellarAutoscalerSpec, StellarAutoscalerStatus,
    StellarMetric,
};
pub use stellar_benchmark::{
    BenchmarkConfig, BenchmarkMetrics, BenchmarkPhase, BenchmarkReport, BenchmarkReportSpec,
    BenchmarkReportStatus, BenchmarkResourceRequirements, BenchmarkSummary,
    EnvVar as BenchmarkEnvVar, PodResult, ResultStorage, StellarBenchmark, StellarBenchmarkSpec,
    StellarBenchmarkStatus, Toleration as BenchmarkToleration,
};
pub use stellar_federation::{
    FederationCluster, ReplicationConfig, ReplicationMode, RoutingStrategy, StellarFederation,
    StellarFederationSpec, StellarFederationStatus, TrafficRoutingPolicy,
};
pub use stellar_network_policy::{
    AllowedDestination, Condition as NetworkPolicyCondition, DNSRule, EgressRule, GRPCRule,
    HTTPRule, HeaderMatch, IPBlock, IngressRule, L7Rule, LabelSelector, LabelSelectorRequirement,
    MetadataMatch, NetworkPolicyPeer, NetworkPolicyPort, SegmentSelector, StellarNetworkPolicy,
    StellarNetworkPolicySpec, StellarNetworkPolicyStatus, StellarNetworkSegment,
    StellarNetworkSegmentSpec, StellarNetworkSegmentStatus, StellarWorkloadProfile,
    StellarWorkloadProfileSpec, TLSRule, WorkloadIdentity,
};
pub use stellar_node::{
    BGPStatus, ObservedStorageStatus, SnapshotBootstrapStatus, SpecValidationError, StellarNode,
    StellarNodeSpec, StellarNodeStatus,
};
pub use stellar_observability::{
    AlertRule, AlertingConfig, AnomalyDetectionConfig, AnomalyModel, AnomalySensitivity,
    LoggingBackend, LoggingConfig, StellarObservability, StellarObservabilitySpec,
    StellarObservabilityStatus, TracingBackend, TracingConfig,
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
pub use traffic_policy::{
    AdaptiveRateLimitPolicy, CircuitBreakerPolicy, LeakyBucketPolicy, PriorityRule, QosClassPolicy,
    TokenBucketPolicy, TrafficPolicy, TrafficPolicySpec, TrafficPolicyStatus, TrafficPriorityClass,
};
pub use traffic_shift_plan::{
    AppliedGate, FailbackPolicy, FailoverTargets, FailoverTrigger, GateDecision, HealthGateSpec,
    RegionHealthEvidence, RegionTarget, RoutingRecordSpec, RpoEvidence, RtoMeasurement,
    ShiftDirection, ShiftPolicy, ShiftStepStatus, StepOutcome, TrafficShiftPhase, TrafficShiftPlan,
    TrafficShiftPlanSpec, TrafficShiftPlanStatus, TrafficWeights,
};
pub use types::*;

// Epic CRD exports (Wave 5)
pub use stellar_aiops::{
    AnomalyDetectionConfig as AIOpsAnomalyDetectionConfig, AutomatedRemediationConfig,
    CapacityPlanningConfig, ChatOpsConfig, OperationalStatus, PredictiveMaintenanceConfig,
    RootCauseAnalysisConfig, SlackIntegration, StellarAIOps, StellarAIOpsSpec, StellarAIOpsStatus,
    TeamsIntegration,
};
pub use stellar_database::{
    AutoTuningConfig as DbAutoTuningConfig, ConnectionPoolingConfig as DbConnectionPoolingConfig,
    DatabasePhase, FailoverConfig as DbFailoverConfig,
    QueryOptimizationConfig as DbQueryOptimizationConfig, ReadReplicaConfig as DbReadReplicaConfig,
    StellarDatabase, StellarDatabaseSpec, StellarDatabaseStatus,
};
pub use stellar_disaster_recovery::{
    BackupDestination, DrillStatus, EncryptionConfig, RestorePhase, RetentionPolicy, StellarBackup,
    StellarBackupSpec, StellarBackupStatus, StellarDRDrill, StellarDRDrillSpec,
    StellarDRDrillStatus, StellarRestore, StellarRestoreSpec, StellarRestoreStatus,
};
pub use stellar_gitops::{
    ArgoCDConfig, ArgoCDSyncPolicy, FluxCDConfig, GitOpsProvider, ProgressiveDeliveryConfig,
    StellarGitOpsConfig, StellarGitOpsConfigSpec, StellarGitOpsConfigStatus, SyncStatus,
};
pub use stellar_policy_bundle::{
    CelPolicySpec, PolicyTrustRootRef, StellarPolicyBundle, StellarPolicyBundleSpec,
    StellarPolicyBundleStatus,
};
pub use stellar_registry::{
    AdmissionPolicy, AutoPatchConfig, ComplianceReport as RegistryComplianceReport,
    GarbageCollectionConfig, MirrorStatus, RegistryMirror, RegistryPhase, RegistryProxyConfig,
    ScannerBackend, ScanningConfig, SigningConfig, StellarRegistry, StellarRegistrySpec,
    StellarRegistryStatus, VulnerabilitySummary,
};
pub use stellar_security::{
    AutomatedScanningConfig, ComplianceFramework, ComplianceStatus as SecurityComplianceStatus,
    NetworkPoliciesConfig, PodSecurityLevel, PodSecurityStandardsConfig, RBACConfig,
    SecretManagementConfig, SecretProvider, SecurityMonitoringConfig, StellarSecurityPolicy,
    StellarSecurityPolicySpec, StellarSecurityPolicyStatus,
};

// Issue #1577 — Ledger-Close Webhook CRD exports
pub mod namespace_teardown;

// Issue #1577 â€” Ledger-Close Webhook CRD exports
pub use ledger_close_webhook::{
    DeliveryLogEntry, DeliveryPhase, LedgerCloseEventType, LedgerClosePayload,
    LedgerCloseWebhook, LedgerCloseWebhookSpec, LedgerCloseWebhookStatus,
};
pub use compliance_report::{
    ComplianceCondition, ComplianceReport, ComplianceReportFormat, ComplianceReportPhase,
    ComplianceReportSpec, ComplianceReportStatus, GeneratedArtifactRef, HsmKmsVerificationSpec,
    KeyCustodyAttestation, ReportDestinationConfig, TxProcessingEvidence, ValidatorUptimeEvidence,
};
pub use incident::{
    AlertChannelConfig, AlertChannelType, AlertDispatchResult, Incident, IncidentPhase,
    IncidentSeverity as NetworkIncidentSeverity, IncidentSpec,
    IncidentStatus as NetworkIncidentStatus, IncidentTimelineEntry, IncidentType, PartitionDetails,
    QuorumAdjustmentRecommendation,
};
pub use ledger_close_webhook::{
    DeliveryLogEntry, DeliveryPhase, LedgerCloseEventType, LedgerClosePayload, LedgerCloseWebhook,
    LedgerCloseWebhookSpec, LedgerCloseWebhookStatus,
};
pub use namespace_teardown::{
    ArchiveSpec, NamespaceTeardown, NamespaceTeardownSpec, NamespaceTeardownStatus,
    TeardownCondition, TeardownPhase, TeardownStepStatus,
};
pub use multisig_operation::{
    AdminOperationType, CollectedSignature, MultiSigAuditEntry, MultiSigCondition,
    MultiSigOperation, MultiSigOperationSpec, MultiSigOperationStatus, MultiSigPhase,
    SignerEndpointSpec, SubmissionResult,
};
pub use validator_score::{
    ComponentScore, HourlyScoreSample, LeaderboardEntry, PerformanceGrade, ScoringWeights,
    ValidatorLeaderboard, ValidatorLeaderboardSpec, ValidatorLeaderboardStatus, ValidatorScore,
    ValidatorScoreSpec, ValidatorScoreStatus,
};



