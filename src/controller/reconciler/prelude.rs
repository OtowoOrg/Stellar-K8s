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

use crate::controller::archive_health::{
    calculate_backoff, check_archive_integrity, check_archive_integrity_random,
    check_history_archive_health, ArchiveHealthResult, ArchiveIntegrityCheckResult,
    ARCHIVE_LAG_THRESHOLD,
};
use crate::controller::audit_worker::AuditWorker;
use crate::controller::conditions;
use crate::controller::cross_cloud_failover;
use crate::controller::cve_reconciler;
use crate::controller::disk_scaler;
use crate::controller::dr;
use crate::controller::dr_drill;
use crate::controller::finalizers::STELLAR_NODE_FINALIZER;
use crate::controller::health;
use crate::controller::kms_secret;
use crate::controller::label_propagation::LabelPropagator;
use crate::controller::maintenance;
#[cfg(feature = "metrics")]
use crate::controller::metrics;
use crate::controller::mtls;
use crate::controller::oci_snapshot;
use crate::controller::operator_config::{hardcoded_defaults, OperatorConfig};
use crate::controller::peer_discovery;
use crate::controller::pss;
use crate::controller::remediation;
use crate::controller::resources;
use crate::controller::secret_watcher;
use crate::controller::service_mesh;
use crate::controller::spot_drain;
use crate::controller::sync_scale;
use crate::controller::sync_state_monitor;
use crate::controller::vpa as vpa_controller;
use crate::controller::vsl;
use chrono::Utc;
