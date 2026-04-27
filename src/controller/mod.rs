//! Controller module for StellarNode reconciliation
//!
//! This module contains the main controller loop, reconciliation logic,
//! and resource management for Stellar nodes.
//!
//! # Overview
//!
//! The controller implements the Kubernetes Operator pattern, continuously
//! watching StellarNode resources and reconciling their desired state with
//! the actual cluster state. It handles:
//!
//! - **Reconciliation**: Applying desired state changes to Deployments, StatefulSets, Services, etc.
//! - **Health Monitoring**: Checking node health and sync status
//! - **Lifecycle Management**: Finalizers for clean resource cleanup
//! - **Leader Election**: Ensuring only one operator instance reconciles at a time
//! - **Remediation**: Automatic recovery from common failure modes
//! - **Archive Management**: History archive integrity and pruning
//! - **Disaster Recovery**: Backup and restore automation
//! - **Service Mesh Integration**: Istio and Linkerd support
//! - **CVE Patching**: Automatic security updates
//! - **Blue/Green Deployments**: Zero-downtime RPC node updates
//! - **Metrics**: Prometheus metrics for observability
//!
//! # Key Types
//!
//! - [`reconciler::ControllerState`] - Shared state for the reconciliation loop
//! - [`reconciler::run_controller`] - Main entry point for the controller
//! - [`health::HealthCheckResult`] - Node health status
//! - [`archive_health::ArchiveHealthResult`] - Archive integrity status
//! - [`remediation::RemediationLevel`] - Severity of remediation actions
//! - [`blue_green::BlueGreenStatus`] - Blue/Green deployment status
//!
//! # Reconciliation Flow
//!
//! 1. Watch for StellarNode resource changes
//! 2. Acquire leader lease (if leader election enabled)
//! 3. Validate node specification
//! 4. Create/update Kubernetes resources (Deployments, Services, PVCs, etc.)
//! 5. Monitor health and sync status
//! 6. Apply remediation if needed
//! 7. Update node status with conditions
//! 8. Requeue for periodic reconciliation
//!
//! # Finalizers
//!
//! The controller uses Kubernetes finalizers to ensure clean cleanup:
//! - Removes PVCs if retention policy is `Delete`
//! - Cleans up associated resources (Services, ConfigMaps, etc.)
//! - Removes finalizer only after successful cleanup

pub mod benchmark;
pub mod blue_green;
pub mod canary;
pub mod cross_cloud_failover;
pub mod feature_flags;
pub mod gas_autoscaling;
pub mod horizon_scaler;
pub mod jurisdiction;
pub mod label_propagation;
pub mod maintenance;
pub mod network_isolation;
pub mod predictive_scaling;
pub mod pss;
pub mod resource_meta;

pub(crate) mod archive_health;
pub mod archive_prune;
pub mod zk_archive_verifier;
pub mod audit;
pub mod audit_log;
pub mod audit_sink;
pub mod audit_worker;
pub mod background_jobs;
pub mod captive_core;
pub mod chaos_engineering;
pub mod conditions;
pub mod cost;
pub mod cross_cluster;
pub mod cve;
pub(crate) mod cve_reconciler;
pub mod cve_scanner;
#[cfg(test)]
pub(crate) mod cve_test;
pub mod db_pool;
pub mod diff;
pub mod dr;
pub mod dr_drill;
#[cfg(test)]
mod dr_test;
pub(crate) mod finalizers;
pub(crate) mod forensic_snapshot;
pub(crate) mod health;
#[cfg(test)]
mod health_test;
pub mod kms_secret;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod mtls;
pub mod mtls_rotation;
pub mod oci_snapshot;
pub mod operator_config;
pub mod peer_discovery;
#[cfg(test)]
mod peer_discovery_test;
pub mod pruning_reconciler;
pub mod pruning_worker;
pub mod quorum;
pub mod read_pool;
pub(crate) mod reconciler;
#[cfg(test)]
mod reconciler_test;
pub(crate) mod remediation;
#[cfg(test)]
mod remediation_test;
pub(crate) mod resources;
#[cfg(test)]
mod resources_test;
pub mod service_mesh;
mod snapshot;
pub mod snapshot_worker;
pub mod storage_migration;
pub(crate) mod sync_scale;
pub(crate) mod sync_state_monitor;
pub mod traffic;
#[cfg(test)]
mod traffic_test;
pub mod vpa;
pub(crate) mod vsl;
pub mod webhook_delivery;

pub use archive_health::{
    calculate_backoff, check_archive_integrity, check_history_archive_health, ArchiveHealthResult,
    ArchiveIntegrityResult, ARCHIVE_LAG_THRESHOLD,
};
pub use audit_log::{AdminAction, AuditEntry, AuditLog};
pub use background_jobs::{JobKind, JobRecord, JobRegistry, JobState, MAX_JOBS};
pub use benchmark::run_benchmark_controller;
pub use blue_green::{
    cleanup_blue_deployment, create_green_deployment, rollback_to_blue, run_smoke_tests,
    switch_traffic_to_green, wait_for_green_ready, BlueGreenConfig, BlueGreenStatus,
};
pub use cross_cloud_failover::reconcile_cross_cloud_failover;
pub use cross_cluster::{check_peer_latency, ensure_cross_cluster_services, PeerLatencyStatus};
pub use cve_reconciler::reconcile_cve_patches;
pub use cve_scanner::{
    list_vulnerable_pods, register_cve_metrics, spawn_background_scanner, CveScannerConfig,
    PodScanSummary,
};
pub use db_pool::{
    create_pool, DbPoolConfig, DEFAULT_CONNECTION_TIMEOUT_SECS, DEFAULT_MAX_CONNECTIONS,
};
pub use feature_flags::{
    watch_feature_flags, FeatureFlags, SharedFeatureFlags, FEATURE_FLAGS_CONFIGMAP,
};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use health::{check_node_health, HealthCheckResult};
pub use jurisdiction::{
    build_jurisdiction_node_affinity, compliance_report, merge_jurisdiction_tolerations,
    ComplianceReportEntry,
};
pub use network_isolation::{
    check_network_safety, network_label_value, same_network_namespace_selector,
    NetworkSafetyViolation, NAMESPACE_NETWORK_LABEL, NODE_NETWORK_LABEL,
};
pub use operator_config::{hardcoded_defaults, OperatorConfig};
pub use peer_discovery::{
    get_peers_from_config_map, trigger_peer_config_reload, PeerDiscoveryConfig,
    PeerDiscoveryManager, PeerInfo,
};
pub use pruning_reconciler::{reconcile_pruning, update_pruning_status};
pub use pss::{
    ensure_namespace_pss_labels, restricted_container_security_context,
    restricted_pod_security_context, validate_pss_compliance, PssViolation,
};
#[cfg(feature = "reconciler-fuzz")]
pub use reconciler::reconcile_for_fuzz;
pub use reconciler::{run_controller, BatchSummaryReport, ControllerState};
pub use remediation::{can_remediate, check_stale_node, RemediationLevel, StaleCheckResult};
pub use service_mesh::{
    delete_service_mesh_resources, ensure_destination_rule, ensure_peer_authentication,
    ensure_request_authentication, ensure_virtual_service,
};
pub use snapshot_worker::run_snapshot_worker;
pub use webhook_delivery::{
    DeliveryRecord, WebhookDeliveryService, WebhookEndpoint, WebhookEvent, WebhookEventType,
};
