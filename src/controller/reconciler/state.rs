//! Controller shared state.

use super::prelude::*;

/// Shared state for the controller
///
/// Holds the Kubernetes client and any other shared resources needed by the reconciler.
/// This state is passed to reconcile functions and is used to interact with the Kubernetes API.
pub struct ControllerState {
    /// Kubernetes client for API interactions
    pub client: Client,
    pub enable_mtls: bool,
    pub operator_namespace: String,
    /// Restrict the operator to only watch and manage StellarNode resources in this namespace.
    /// If None, the operator watches all namespaces.
    pub watch_namespace: Option<String>,
    pub mtls_config: Option<crate::MtlsConfig>,
    pub dry_run: bool,
    /// Requeue interval in seconds for retriable reconciliation errors.
    pub retry_budget_retriable_secs: u64,
    /// Requeue interval in seconds for non-retriable reconciliation errors.
    pub retry_budget_nonretriable_secs: u64,
    /// Maximum HTTP retry attempts for SCP and quorum queries.
    pub retry_budget_max_attempts: u32,
    pub is_leader: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Identifies this operator when publishing Kubernetes Events via [`Recorder`].
    pub event_reporter: Reporter,
    /// Operator-level config loaded from the Helm-rendered ConfigMap (defaultResources).
    pub operator_config: std::sync::Arc<OperatorConfig>,
    /// Counter for generating unique reconcile IDs
    pub reconcile_id_counter: std::sync::atomic::AtomicU64,
    /// Timestamp of the last successful reconcile
    pub last_reconcile_success: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Handle to reload the tracing filter
    pub log_reload_handle: Handle<EnvFilter, Registry>,
    /// Optional expiration time for a temporary log level change
    pub log_level_expires_at:
        std::sync::Arc<tokio::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>>,
    /// Timestamp of the last event received from the K8s watch stream
    pub last_event_received: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Background job registry for the monitoring dashboard.
    pub job_registry: std::sync::Arc<crate::controller::background_jobs::JobRegistry>,
    /// In-memory audit log for admin activity.
    pub audit_log: std::sync::Arc<crate::controller::audit_log::AuditLog>,
    /// Unified audit recorder (in-memory log + optional sink).
    pub audit_recorder: std::sync::Arc<crate::controller::audit_recorder::AuditRecorder>,
    /// ML-based anomaly detector for operator behavior.
    pub anomaly_detector: std::sync::Arc<crate::controller::anomaly_detection::AnomalyDetector>,
    /// Plugin registry for custom reconciliation hooks and sidecar injectors.
    pub plugin_registry: std::sync::Arc<crate::plugin_sdk::PluginRegistry>,
    /// Log analytics engine for pattern detection and anomaly reporting.
    pub analytics_engine: std::sync::Arc<crate::logging::analytics::AnalyticsEngine>,
    /// Optional OIDC configuration for JWT-based authentication on the REST API.
    /// When `Some`, the OIDC middleware is active; when `None`, the operator falls
    /// back to Kubernetes RBAC token validation.
    #[cfg(feature = "rest-api")]
    pub oidc_config: Option<crate::rest_api::OidcConfig>,
    /// Thread-safe cache of Stellar metrics (TPS, queue length, etc.) shared between
    /// the background [`HorizonMetricsCollector`] and the custom metrics API handlers.
    ///
    /// Handlers read from this store to serve `custom.metrics.k8s.io/v1beta2` requests.
    /// The collector writes to it on each scrape cycle.
    #[cfg(feature = "rest-api")]
    pub metrics_store: std::sync::Arc<crate::rest_api::metrics_store::StellarMetricsStore>,
}

impl ControllerState {
    /// Generate a unique reconcile ID
    pub fn next_reconcile_id(&self) -> u64 {
        self.reconcile_id_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}
