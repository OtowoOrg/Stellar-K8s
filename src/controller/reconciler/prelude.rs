//! Shared imports for reconciler submodules.

use futures::future::BoxFuture;
use futures::FutureExt;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::policy::v1::PodDisruptionBudget;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Service};
use kube::{
    api::{Api, Patch, PatchParams},
    client::Client,
    runtime::{
        controller::{Action, Controller},
        events::{Event as K8sRecorderEvent, EventType, Recorder, Reporter},
        watcher::Config,
    },
    Resource, ResourceExt,
};
use tracing::{debug, error, info, info_span, instrument, warn};
use tracing_subscriber::{reload::Handle, EnvFilter, Registry};

use crate::crd::{
    Condition, DisasterRecoveryStatus, NodeType, SpecValidationError, StellarNode,
    StellarNodeStatus,
};
use crate::error::{Error, Result};
#[cfg(feature = "metrics")]
use crate::infra;
use crate::plugin_sdk::{HookResult, ReconcileContext};

use super::archive_health::{
    calculate_backoff, check_archive_integrity, check_archive_integrity_random,
    check_history_archive_health, ArchiveHealthResult, ArchiveIntegrityCheckResult,
    ARCHIVE_LAG_THRESHOLD,
};
use super::audit_worker::AuditWorker;
use super::conditions;
use super::cross_cloud_failover;
use super::cve_reconciler;
use super::disk_scaler;
use super::dr;
use super::dr_drill;
use super::finalizers::STELLAR_NODE_FINALIZER;
use super::health;
use super::kms_secret;
use super::label_propagation::LabelPropagator;
use super::maintenance;
#[cfg(feature = "metrics")]
use super::metrics;
use super::mtls;
use super::oci_snapshot;
use super::operator_config::{hardcoded_defaults, OperatorConfig};
use super::peer_discovery;
use super::pss;
use super::remediation;
use super::resources;
use super::secret_watcher;
use super::service_mesh;
use super::spot_drain;
use super::sync_scale;
use super::sync_state_monitor;
use super::vpa as vpa_controller;
use super::vsl;
use chrono::Utc;
