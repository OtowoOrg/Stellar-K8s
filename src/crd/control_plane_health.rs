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
//! `ControlPlaneHealth` CRD (#1494)
//!
//! Cluster-scoped singleton that makes the operator's degradation mode an
//! explicit, observable status rather than implicit behaviour. The spec
//! configures component probes; the status reports per-component health, the
//! current [`DegradationLevel`], the actions the operator currently permits
//! itself, the transition log and post-incident mode reports.
//!
//! The status is written by [`crate::degradation::monitor`].

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Desired probing and webhook behaviour for control-plane degradation.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ControlPlaneHealth",
    status = "ControlPlaneHealthStatus",
    shortname = "cph",
    printcolumn = r#"{"name":"Level","type":"string","jsonPath":".status.level"}"#,
    printcolumn = r#"{"name":"Since","type":"date","jsonPath":".status.levelSince"}"#,
    printcolumn = r#"{"name":"WebhookMode","type":"string","jsonPath":".spec.webhookMode"}"#,
    printcolumn = r#"{"name":"Age","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ControlPlaneHealthSpec {
    /// How admission webhooks behave when the webhook layer is unavailable.
    #[serde(default)]
    pub webhook_mode: WebhookMode,
    /// Seconds between probe rounds.
    #[serde(default = "default_probe_interval_seconds")]
    pub probe_interval_seconds: u32,
    /// Per-probe timeout in seconds.
    #[serde(default = "default_probe_timeout_seconds")]
    pub probe_timeout_seconds: u32,
    /// Consecutive failed probes before a component is marked unhealthy.
    #[serde(default = "default_threshold")]
    pub failure_threshold: u32,
    /// Consecutive successful probes before an unhealthy component recovers.
    #[serde(default = "default_threshold")]
    pub recovery_threshold: u32,
    #[serde(default)]
    pub dns: DnsProbeSpec,
    #[serde(default)]
    pub webhook: WebhookProbeSpec,
    #[serde(default)]
    pub scheduler: SchedulerProbeSpec,
    /// Number of post-incident reports retained in status.
    #[serde(default = "default_history")]
    pub incident_history: u32,
}

impl Default for ControlPlaneHealthSpec {
    fn default() -> Self {
        Self {
            webhook_mode: WebhookMode::default(),
            probe_interval_seconds: default_probe_interval_seconds(),
            probe_timeout_seconds: default_probe_timeout_seconds(),
            failure_threshold: default_threshold(),
            recovery_threshold: default_threshold(),
            dns: DnsProbeSpec::default(),
            webhook: WebhookProbeSpec::default(),
            scheduler: SchedulerProbeSpec::default(),
            incident_history: default_history(),
        }
    }
}

fn default_probe_interval_seconds() -> u32 {
    10
}
fn default_probe_timeout_seconds() -> u32 {
    3
}
fn default_threshold() -> u32 {
    3
}
fn default_history() -> u32 {
    10
}

/// Admission webhook behaviour under a webhook-layer outage.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum WebhookMode {
    /// Webhooks keep `failurePolicy: Fail`; an outage blocks admission.
    #[default]
    Strict,
    /// Webhooks are kept at `failurePolicy: Ignore` so an outage never blocks
    /// admission (including pod startup). Drift is corrected by the monitor.
    Permissive,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DnsProbeSpec {
    /// Name resolved through cluster DNS on every probe.
    #[serde(default = "default_dns_hostname")]
    pub hostname: String,
}

impl Default for DnsProbeSpec {
    fn default() -> Self {
        Self {
            hostname: default_dns_hostname(),
        }
    }
}

fn default_dns_hostname() -> String {
    "kubernetes.default.svc.cluster.local".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookProbeSpec {
    /// Webhook service host probed for TCP reachability.
    #[serde(default = "default_webhook_host")]
    pub host: String,
    #[serde(default = "default_webhook_port")]
    pub port: u16,
    /// Webhook configurations governed by `webhookMode`.
    #[serde(default = "default_webhook_configurations")]
    pub configurations: Vec<WebhookConfigurationRef>,
}

impl Default for WebhookProbeSpec {
    fn default() -> Self {
        Self {
            host: default_webhook_host(),
            port: default_webhook_port(),
            configurations: default_webhook_configurations(),
        }
    }
}

fn default_webhook_host() -> String {
    "stellar-webhook.stellar-webhook.svc".to_string()
}
fn default_webhook_port() -> u16 {
    443
}
fn default_webhook_configurations() -> Vec<WebhookConfigurationRef> {
    vec![WebhookConfigurationRef {
        kind: WebhookConfigurationKind::Validating,
        name: "stellar-webhook".to_string(),
    }]
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WebhookConfigurationRef {
    pub kind: WebhookConfigurationKind,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum WebhookConfigurationKind {
    Validating,
    Mutating,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerProbeSpec {
    /// Leader-election lease renewed by the active scheduler.
    #[serde(default = "default_scheduler_lease_namespace")]
    pub lease_namespace: String,
    #[serde(default = "default_scheduler_lease_name")]
    pub lease_name: String,
}

impl Default for SchedulerProbeSpec {
    fn default() -> Self {
        Self {
            lease_namespace: default_scheduler_lease_namespace(),
            lease_name: default_scheduler_lease_name(),
        }
    }
}

fn default_scheduler_lease_namespace() -> String {
    "kube-system".to_string()
}
fn default_scheduler_lease_name() -> String {
    "kube-scheduler".to_string()
}

/// Control-plane component probed independently.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "PascalCase")]
pub enum ControlPlaneComponent {
    Etcd,
    Dns,
    Webhook,
    Scheduler,
}

impl ControlPlaneComponent {
    pub const ALL: [Self; 4] = [Self::Etcd, Self::Dns, Self::Webhook, Self::Scheduler];

    /// Level the operator enters while this component is unhealthy.
    pub fn implied_level(self) -> DegradationLevel {
        match self {
            Self::Etcd => DegradationLevel::Frozen,
            Self::Dns => DegradationLevel::Degraded,
            Self::Webhook | Self::Scheduler => DegradationLevel::Reduced,
        }
    }

    /// Components whose outage makes this component's probe inconclusive.
    ///
    /// The scheduler is observed through its lease, which cannot be renewed
    /// (or read fresh) while etcd is down.
    pub fn depends_on(self) -> &'static [Self] {
        match self {
            Self::Scheduler => &[Self::Etcd],
            Self::Etcd | Self::Dns | Self::Webhook => &[],
        }
    }
}

/// Declared degradation levels, ordered by severity.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "PascalCase")]
pub enum DegradationLevel {
    /// All components healthy; no restrictions.
    #[default]
    Normal,
    /// Webhook or scheduler unavailable: no disruptive actions, because
    /// replacement pods might not be admitted or placed.
    Reduced,
    /// Cluster DNS unavailable: health signals are unreliable, so no
    /// disruptive actions; running pods keep existing connections.
    Degraded,
    /// etcd unavailable: the operator makes no writes at all and the data
    /// plane runs on its last applied configuration.
    Frozen,
}

impl DegradationLevel {
    pub fn permitted(self) -> PermittedActions {
        match self {
            Self::Normal => PermittedActions {
                writes: true,
                disruptive_actions: true,
            },
            Self::Reduced | Self::Degraded => PermittedActions {
                writes: true,
                disruptive_actions: false,
            },
            Self::Frozen => PermittedActions {
                writes: false,
                disruptive_actions: false,
            },
        }
    }

    pub fn as_index(self) -> u8 {
        self as u8
    }

    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Self::Normal,
            1 => Self::Reduced,
            2 => Self::Degraded,
            _ => Self::Frozen,
        }
    }
}

/// Operator actions allowed at the current level.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermittedActions {
    /// Any write to the Kubernetes API.
    pub writes: bool,
    /// Actions that interrupt serving pods (restarts, deletions, evictions).
    pub disruptive_actions: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ComponentState {
    #[default]
    Unknown,
    Healthy,
    Unhealthy,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    pub component: ControlPlaneComponent,
    pub state: ComponentState,
    pub implied_level: DegradationLevel,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_probe_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LevelTransition {
    pub from: DegradationLevel,
    pub to: DegradationLevel,
    pub at: String,
    pub reason: String,
}

/// Post-incident mode report covering one excursion away from `Normal`.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IncidentReport {
    pub id: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    pub peak_level: DegradationLevel,
    /// Components that were unhealthy at any point during the incident.
    pub components: Vec<ControlPlaneComponent>,
    pub transitions: Vec<LevelTransition>,
    /// Operator actions withheld because the level did not permit them.
    pub suppressed_actions: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlPlaneHealthStatus {
    #[serde(default)]
    pub level: DegradationLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level_since: Option<String>,
    #[serde(default)]
    pub permitted: PermittedActions,
    #[serde(default)]
    pub components: Vec<ComponentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_incident: Option<IncidentReport>,
    /// Most recent closed incidents, newest first.
    #[serde(default)]
    pub incidents: Vec<IncidentReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_generation: Option<i64>,
}
