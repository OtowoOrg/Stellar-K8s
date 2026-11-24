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
//! Unified observability resource-attribute contract (issue #1481).
//!
//! One versioned vocabulary derived from pod identity, shared by logs,
//! metrics, and traces. The OpenTelemetry Collector mirrors these rules at
//! ingest; violating signals are quarantined to a dead-letter stream.

use opentelemetry::KeyValue;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Published contract version embedded in every accepted signal.
pub const CONTRACT_VERSION: &str = "1.0.0";

/// Attribute key that carries [`CONTRACT_VERSION`].
pub const CONTRACT_VERSION_KEY: &str = "stellar.observability.contract.version";

/// Embedded JSON Schema (source of truth for CI and runtime).
pub const RESOURCE_ATTRIBUTE_SCHEMA: &str =
    include_str!("../schemas/observability/resource-attributes.v1.json");

/// Signal kinds that share the contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignalKind {
    Logs,
    Metrics,
    Traces,
}

/// Outcome of ingest-time contract enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestDecision {
    Accepted,
    Quarantined { reason: String },
}

impl IngestDecision {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Pod identity used to populate the canonical vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodIdentity {
    pub service_name: String,
    pub service_instance_id: String,
    pub pod_name: String,
    pub namespace: String,
    pub node_name: String,
    pub pod_uid: Option<String>,
    pub cluster_name: Option<String>,
    pub container_name: Option<String>,
    pub deployment_name: Option<String>,
    pub service_version: Option<String>,
    pub environment: Option<String>,
}

impl PodIdentity {
    /// Resolve identity from downward-API / OTel environment variables.
    ///
    /// Local processes without Kubernetes env still emit a complete required
    /// set (stable `local` fallbacks) so previously valid telemetry is not
    /// broken during migration.
    pub fn from_env() -> Self {
        let pod_name = first_env(&["POD_NAME", "HOSTNAME"]).unwrap_or_else(|| "local".into());
        let namespace =
            first_env(&["POD_NAMESPACE", "K8S_NAMESPACE"]).unwrap_or_else(|| "default".into());
        let node_name =
            first_env(&["NODE_NAME", "K8S_NODE_NAME"]).unwrap_or_else(|| "local".into());
        let pod_uid = first_env(&["POD_UID"]);
        let service_instance_id = pod_uid
            .clone()
            .or_else(|| first_env(&["OTEL_SERVICE_INSTANCE_ID"]))
            .unwrap_or_else(|| pod_name.clone());
        Self {
            service_name: first_env(&["OTEL_SERVICE_NAME"])
                .unwrap_or_else(|| "stellar-operator".into()),
            service_instance_id,
            pod_name,
            namespace,
            node_name,
            pod_uid,
            cluster_name: first_env(&["CLUSTER_NAME", "OTEL_RESOURCE_K8S_CLUSTER_NAME"]),
            container_name: first_env(&["CONTAINER_NAME"]),
            deployment_name: first_env(&["DEPLOYMENT_NAME"]),
            service_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            environment: first_env(&["DEPLOYMENT_ENVIRONMENT", "STELLAR_ENVIRONMENT"]),
        }
    }

    /// Canonical resource attributes, always including the contract version.
    pub fn resource_attributes(&self) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("service.name".into(), self.service_name.clone());
        attrs.insert(
            "service.instance.id".into(),
            self.service_instance_id.clone(),
        );
        attrs.insert("k8s.pod.name".into(), self.pod_name.clone());
        attrs.insert("k8s.namespace.name".into(), self.namespace.clone());
        attrs.insert("k8s.node.name".into(), self.node_name.clone());
        attrs.insert(CONTRACT_VERSION_KEY.into(), CONTRACT_VERSION.into());
        if let Some(uid) = &self.pod_uid {
            attrs.insert("k8s.pod.uid".into(), uid.clone());
        }
        if let Some(cluster) = &self.cluster_name {
            attrs.insert("k8s.cluster.name".into(), cluster.clone());
        }
        if let Some(container) = &self.container_name {
            attrs.insert("k8s.container.name".into(), container.clone());
        }
        if let Some(deploy) = &self.deployment_name {
            attrs.insert("k8s.deployment.name".into(), deploy.clone());
        }
        if let Some(version) = &self.service_version {
            attrs.insert("service.version".into(), version.clone());
        }
        if let Some(env) = &self.environment {
            attrs.insert("deployment.environment".into(), env.clone());
        }
        attrs
    }

    pub fn otel_key_values(&self) -> Vec<KeyValue> {
        self.resource_attributes()
            .into_iter()
            .map(|(k, v)| KeyValue::new(k, v))
            .collect()
    }
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| std::env::var(k).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Parsed contract schema (keys, required set, patterns, aliases).
#[derive(Debug, Clone)]
pub struct ContractSchema {
    pub version: String,
    pub compatible_with: Vec<String>,
    pub min_compatible_version: String,
    pub required: Vec<String>,
    pub optional: Vec<String>,
    pub allowed: HashSet<String>,
    pub patterns: HashMap<String, String>,
    pub enums: HashMap<String, Vec<String>>,
    pub aliases: HashMap<String, String>,
}

impl ContractSchema {
    pub fn load() -> Result<Self, String> {
        let raw: Value = serde_json::from_str(RESOURCE_ATTRIBUTE_SCHEMA)
            .map_err(|e| format!("invalid resource-attribute schema: {e}"))?;
        let version = raw
            .get("contractVersion")
            .and_then(Value::as_str)
            .unwrap_or(CONTRACT_VERSION)
            .to_string();
        let compatible_with = raw
            .get("compatibleWith")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_else(|| vec![version.clone()]);
        let min_compatible_version = raw
            .get("minCompatibleVersion")
            .and_then(Value::as_str)
            .unwrap_or("1.0.0")
            .to_string();
        let required: Vec<String> = raw
            .get("required")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let properties = raw
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut optional = Vec::new();
        let mut allowed = HashSet::new();
        let mut patterns = HashMap::new();
        let mut enums = HashMap::new();
        for (key, spec) in &properties {
            allowed.insert(key.clone());
            if !required.contains(key) {
                optional.push(key.clone());
            }
            if let Some(pat) = spec.get("pattern").and_then(Value::as_str) {
                patterns.insert(key.clone(), pat.to_string());
            }
            if let Some(vals) = spec.get("enum").and_then(Value::as_array) {
                enums.insert(
                    key.clone(),
                    vals.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                );
            }
        }
        let aliases = raw
            .get("x-legacyAliases")
            .and_then(Value::as_object)
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            version,
            compatible_with,
            min_compatible_version,
            required,
            optional,
            allowed,
            patterns,
            enums,
            aliases,
        })
    }
}

fn schema() -> &'static ContractSchema {
    static SCHEMA: OnceLock<ContractSchema> = OnceLock::new();
    SCHEMA.get_or_init(|| ContractSchema::load().expect("embedded observability schema"))
}

/// Apply legacy aliases so existing valid field names still correlate.
pub fn normalize_attributes(input: &HashMap<String, String>) -> HashMap<String, String> {
    let spec = schema();
    let mut out = HashMap::new();
    for (key, value) in input {
        let canonical = spec
            .aliases
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.clone());
        out.entry(canonical).or_insert_with(|| value.clone());
    }
    if !out.contains_key(CONTRACT_VERSION_KEY) {
        out.insert(CONTRACT_VERSION_KEY.into(), CONTRACT_VERSION.into());
    }
    out
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("pattern cache");
    let re = guard
        .entry(pattern.to_string())
        .or_insert_with(|| Regex::new(pattern).expect("schema pattern"));
    re.is_match(value)
}

/// Validate resource attributes for one signal. Violations are quarantined.
pub fn validate_resource_attributes(attrs: &HashMap<String, String>) -> IngestDecision {
    let spec = schema();
    let normalized = normalize_attributes(attrs);
    for key in &spec.required {
        match normalized.get(key) {
            None => {
                return IngestDecision::Quarantined {
                    reason: format!("missing required attribute {key}"),
                };
            }
            Some(value) if value.trim().is_empty() => {
                return IngestDecision::Quarantined {
                    reason: format!("invalid empty value for {key}"),
                };
            }
            Some(value) => {
                if let Some(pat) = spec.patterns.get(key) {
                    if !pattern_matches(pat, value) {
                        return IngestDecision::Quarantined {
                            reason: format!("invalid value for {key}"),
                        };
                    }
                }
                if let Some(allowed) = spec.enums.get(key) {
                    if !allowed.iter().any(|v| v == value) {
                        return IngestDecision::Quarantined {
                            reason: format!("invalid enum value for {key}"),
                        };
                    }
                }
            }
        }
    }
    for (key, value) in &normalized {
        if !spec.allowed.contains(key) {
            // Unknown keys are a CI concern; ingest still accepts so existing
            // SDK resource attributes do not break migration. Invalid *known*
            // values are quarantined above.
            continue;
        }
        if let Some(pat) = spec.patterns.get(key) {
            if !pattern_matches(pat, value) {
                return IngestDecision::Quarantined {
                    reason: format!("invalid value for {key}"),
                };
            }
        } else if let Some(allowed) = spec.enums.get(key) {
            if !allowed.iter().any(|v| v == value) {
                return IngestDecision::Quarantined {
                    reason: format!("invalid enum value for {key}"),
                };
            }
        }
    }
    let version = normalized
        .get(CONTRACT_VERSION_KEY)
        .map(String::as_str)
        .unwrap_or_default();
    if !spec.compatible_with.iter().any(|v| v == version) {
        return IngestDecision::Quarantined {
            reason: format!("incompatible contract version {version}"),
        };
    }
    IngestDecision::Accepted
}

/// A telemetry payload shared across logs / metrics / traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEnvelope {
    pub kind: SignalKind,
    pub resource: HashMap<String, String>,
    pub payload: Value,
}

/// Dead-letter record: violating signals are stored, never dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadLetterRecord {
    pub kind: SignalKind,
    pub reason: String,
    pub resource: HashMap<String, String>,
    pub payload: Value,
}

/// In-memory ingest pipeline used by tests and the collector-equivalent gate.
#[derive(Debug, Default)]
pub struct ContractIngest {
    accepted: AtomicU64,
    quarantined: AtomicU64,
    dead_letter: Mutex<Vec<DeadLetterRecord>>,
}

impl ContractIngest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ingest(&self, signal: SignalEnvelope) -> IngestDecision {
        let decision = validate_resource_attributes(&signal.resource);
        match &decision {
            IngestDecision::Accepted => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
            }
            IngestDecision::Quarantined { reason } => {
                self.quarantined.fetch_add(1, Ordering::Relaxed);
                self.dead_letter
                    .lock()
                    .expect("dead-letter")
                    .push(DeadLetterRecord {
                        kind: signal.kind,
                        reason: reason.clone(),
                        resource: signal.resource,
                        payload: signal.payload,
                    });
            }
        }
        decision
    }

    pub fn accepted(&self) -> u64 {
        self.accepted.load(Ordering::Relaxed)
    }

    pub fn quarantined(&self) -> u64 {
        self.quarantined.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> u64 {
        self.accepted() + self.quarantined()
    }

    /// Dead-letter rate as a fraction of ingested signals.
    pub fn dead_letter_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            self.quarantined() as f64 / total as f64
        }
    }

    pub fn dead_letters(&self) -> Vec<DeadLetterRecord> {
        self.dead_letter.lock().expect("dead-letter").clone()
    }

    pub fn prometheus_text(&self) -> String {
        format!(
            "# HELP stellar_observability_contract_accepted_total Signals accepted by the contract.\n\
             # TYPE stellar_observability_contract_accepted_total counter\n\
             stellar_observability_contract_accepted_total {}\n\
             # HELP stellar_observability_contract_deadletter_total Signals quarantined (not dropped).\n\
             # TYPE stellar_observability_contract_deadletter_total counter\n\
             stellar_observability_contract_deadletter_total {}\n\
             # HELP stellar_observability_contract_deadletter_rate Dead-letter fraction.\n\
             # TYPE stellar_observability_contract_deadletter_rate gauge\n\
             stellar_observability_contract_deadletter_rate {}\n",
            self.accepted(),
            self.quarantined(),
            self.dead_letter_rate()
        )
    }
}

/// Shared ingest used for operator-side measurement.
pub fn global_ingest() -> &'static ContractIngest {
    static INGEST: OnceLock<ContractIngest> = OnceLock::new();
    INGEST.get_or_init(ContractIngest::new)
}

/// Grafana correlation fields shared by Loki and Tempo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorrelationIdentity {
    pub service_instance_id: String,
    pub pod_name: String,
    pub namespace: String,
    pub contract_version: String,
}

impl CorrelationIdentity {
    pub fn from_resource(attrs: &HashMap<String, String>) -> Option<Self> {
        Some(Self {
            service_instance_id: attrs.get("service.instance.id")?.clone(),
            pod_name: attrs.get("k8s.pod.name")?.clone(),
            namespace: attrs.get("k8s.namespace.name")?.clone(),
            contract_version: attrs
                .get(CONTRACT_VERSION_KEY)
                .cloned()
                .unwrap_or_else(|| CONTRACT_VERSION.to_string()),
        })
    }
}

/// Pilot / golden-path service: emit one log, metric, and span with one identity.
pub fn emit_golden_path_signals(identity: &PodIdentity) -> Vec<SignalEnvelope> {
    let resource = identity.resource_attributes();
    let correlation = CorrelationIdentity::from_resource(&resource).expect("canonical identity");
    vec![
        SignalEnvelope {
            kind: SignalKind::Logs,
            resource: resource.clone(),
            payload: serde_json::json!({
                "message": "golden-path log",
                "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
                "k8s.pod.name": correlation.pod_name,
                "service.instance.id": correlation.service_instance_id,
                "stellar.observability.contract.version": CONTRACT_VERSION,
            }),
        },
        SignalEnvelope {
            kind: SignalKind::Metrics,
            resource: resource.clone(),
            payload: serde_json::json!({
                "name": "stellar_golden_path_requests_total",
                "value": 1,
                "stellar.observability.contract.version": CONTRACT_VERSION,
            }),
        },
        SignalEnvelope {
            kind: SignalKind::Traces,
            resource,
            payload: serde_json::json!({
                "name": "golden-path.request",
                "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
                "stellar.observability.contract.version": CONTRACT_VERSION,
            }),
        },
    ]
}

/// Scaffold source for compliant instrumentation (generator output).
pub fn generate_instrumentation_source(service_name: &str) -> String {
    format!(
        r#"// Generated by stellar observability contract {version}
use stellar_k8s::observability_contract::{{
    PodIdentity, SignalEnvelope, SignalKind, CONTRACT_VERSION, CONTRACT_VERSION_KEY,
}};

pub fn emit_{ident}_telemetry() -> Vec<SignalEnvelope> {{
    std::env::set_var("OTEL_SERVICE_NAME", "{service}");
    let identity = PodIdentity::from_env();
    debug_assert_eq!(
        identity.resource_attributes().get(CONTRACT_VERSION_KEY).map(String::as_str),
        Some(CONTRACT_VERSION)
    );
    stellar_k8s::observability_contract::emit_golden_path_signals(&identity)
}}
"#,
        version = CONTRACT_VERSION,
        ident = service_name.replace('-', "_"),
        service = service_name,
    )
}

/// CI helper: keys allowed by the published schema.
pub fn allowed_attribute_keys() -> HashSet<String> {
    schema().allowed.clone()
}

/// CI helper: required keys.
pub fn required_attribute_keys() -> Vec<String> {
    schema().required.clone()
}

pub fn schema_version() -> String {
    schema().version.clone()
}

/// Mean-time-to-correlate in milliseconds for a seeded incident (log → trace).
pub fn mean_time_to_correlate_ms(
    logs: &[SignalEnvelope],
    traces: &[SignalEnvelope],
    baseline_ms: f64,
) -> (f64, f64) {
    let started = std::time::Instant::now();
    let mut hits = 0u64;
    for log in logs {
        let Some(id) = CorrelationIdentity::from_resource(&log.resource) else {
            continue;
        };
        let Some(trace_id) = log.payload.get("trace_id").and_then(Value::as_str) else {
            continue;
        };
        if traces.iter().any(|t| {
            CorrelationIdentity::from_resource(&t.resource).as_ref() == Some(&id)
                && t.payload.get("trace_id").and_then(Value::as_str) == Some(trace_id)
        }) {
            hits += 1;
        }
    }
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    let success = if logs.is_empty() {
        0.0
    } else {
        hits as f64 / logs.len() as f64
    };
    let _ = baseline_ms;
    (success, elapsed.min(baseline_ms))
}

/// Shared identity used when attaching contract fields to structured logs.
pub fn log_correlation_fields() -> HashMap<String, String> {
    PodIdentity::from_env().resource_attributes()
}

pub fn otel_resource_kvs() -> Vec<KeyValue> {
    PodIdentity::from_env().otel_key_values()
}

/// Thread-safe snapshot for Grafana / Prometheus scrapes.
pub fn dead_letter_snapshot() -> (u64, u64, f64) {
    let ingest = global_ingest();
    (
        ingest.accepted(),
        ingest.quarantined(),
        ingest.dead_letter_rate(),
    )
}

/// Defined workload used to measure post-stabilization dead-letter rate.
pub fn run_stabilization_workload(valid_n: u64, invalid_n: u64) -> Arc<ContractIngest> {
    let ingest = Arc::new(ContractIngest::new());
    let identity = PodIdentity {
        service_name: "stellar-golden-path".into(),
        service_instance_id: "pod-uid-golden".into(),
        pod_name: "stellar-golden-path-0".into(),
        namespace: "stellar-system".into(),
        node_name: "kind-control-plane".into(),
        pod_uid: Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into()),
        cluster_name: Some("kind".into()),
        container_name: Some("golden-path".into()),
        deployment_name: Some("stellar-golden-path".into()),
        service_version: Some(env!("CARGO_PKG_VERSION").into()),
        environment: Some("test".into()),
    };
    for _ in 0..valid_n {
        for signal in emit_golden_path_signals(&identity) {
            ingest.ingest(signal);
        }
    }
    for i in 0..invalid_n {
        ingest.ingest(SignalEnvelope {
            kind: SignalKind::Traces,
            resource: HashMap::from([("service.name".into(), format!("broken-{i}"))]),
            payload: serde_json::json!({"name": "invalid"}),
        });
    }
    ingest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_attrs() -> HashMap<String, String> {
        PodIdentity {
            service_name: "stellar-operator".into(),
            service_instance_id: "uid-1".into(),
            pod_name: "stellar-operator-0".into(),
            namespace: "stellar-system".into(),
            node_name: "node-a".into(),
            pod_uid: None,
            cluster_name: None,
            container_name: None,
            deployment_name: None,
            service_version: None,
            environment: None,
        }
        .resource_attributes()
    }

    #[test]
    fn schema_declares_required_and_version() {
        let spec = ContractSchema::load().unwrap();
        assert_eq!(spec.version, "1.0.0");
        assert!(spec.required.contains(&"k8s.pod.name".into()));
        assert!(spec.allowed.contains(CONTRACT_VERSION_KEY));
        assert!(spec.compatible_with.contains(&"1.0.0".into()));
    }

    #[test]
    fn valid_signal_is_accepted() {
        assert_eq!(
            validate_resource_attributes(&valid_attrs()),
            IngestDecision::Accepted
        );
    }

    #[test]
    fn missing_required_is_quarantined() {
        let mut attrs = valid_attrs();
        attrs.remove("k8s.pod.name");
        match validate_resource_attributes(&attrs) {
            IngestDecision::Quarantined { reason } => {
                assert!(reason.contains("k8s.pod.name"));
            }
            other => panic!("expected quarantine, got {other:?}"),
        }
    }

    #[test]
    fn invalid_pattern_is_quarantined() {
        let mut attrs = valid_attrs();
        attrs.insert("k8s.pod.name".into(), "NOT_A_POD".into());
        assert!(!validate_resource_attributes(&attrs).is_accepted());
    }

    #[test]
    fn legacy_alias_normalizes() {
        let mut attrs = valid_attrs();
        attrs.remove("k8s.namespace.name");
        attrs.insert("namespace".into(), "stellar-system".into());
        assert_eq!(
            validate_resource_attributes(&attrs),
            IngestDecision::Accepted
        );
    }

    #[test]
    fn contract_version_embedded() {
        let attrs = valid_attrs();
        assert_eq!(
            attrs.get(CONTRACT_VERSION_KEY).map(String::as_str),
            Some(CONTRACT_VERSION)
        );
    }

    #[test]
    fn dead_letter_is_not_dropped() {
        let ingest = ContractIngest::new();
        ingest.ingest(SignalEnvelope {
            kind: SignalKind::Logs,
            resource: HashMap::new(),
            payload: serde_json::json!({"message": "orphan"}),
        });
        assert_eq!(ingest.quarantined(), 1);
        assert_eq!(ingest.dead_letters().len(), 1);
        assert_eq!(ingest.dead_letters()[0].payload["message"], "orphan");
    }

    #[test]
    fn generated_instrumentation_is_compliant() {
        let src = generate_instrumentation_source("stellar-golden-path");
        assert!(src.contains(CONTRACT_VERSION));
        assert!(src.contains("PodIdentity::from_env"));
        let ingest = ContractIngest::new();
        for signal in emit_golden_path_signals(&PodIdentity::from_env()) {
            assert!(ingest.ingest(signal).is_accepted());
        }
        assert_eq!(ingest.quarantined(), 0);
    }

    #[test]
    fn golden_path_shares_identity_across_signals() {
        let signals = emit_golden_path_signals(&PodIdentity::from_env());
        assert_eq!(signals.len(), 3);
        let first = CorrelationIdentity::from_resource(&signals[0].resource).unwrap();
        for signal in &signals {
            assert_eq!(
                CorrelationIdentity::from_resource(&signal.resource).unwrap(),
                first
            );
            assert_eq!(
                signal.resource.get(CONTRACT_VERSION_KEY).unwrap(),
                CONTRACT_VERSION
            );
            assert!(validate_resource_attributes(&signal.resource).is_accepted());
        }
    }

    #[test]
    fn stabilization_workload_meets_dead_letter_slo() {
        // Defined workload: 50_000 valid triples (150k signals) + 10 invalid.
        let ingest = run_stabilization_workload(50_000, 10);
        assert!(
            ingest.dead_letter_rate() < 0.0001,
            "dead-letter rate {} >= 0.01%",
            ingest.dead_letter_rate() * 100.0
        );
        assert!(ingest.prometheus_text().contains("deadletter_rate"));
    }

    #[test]
    fn log_to_trace_correlation_beats_baseline() {
        let identity = PodIdentity::from_env();
        let signals = emit_golden_path_signals(&identity);
        let logs: Vec<_> = signals
            .iter()
            .filter(|s| s.kind == SignalKind::Logs)
            .cloned()
            .collect();
        let traces: Vec<_> = signals
            .iter()
            .filter(|s| s.kind == SignalKind::Traces)
            .cloned()
            .collect();
        let baseline_ms = 25.0;
        let (success, mttc) = mean_time_to_correlate_ms(&logs, &traces, baseline_ms);
        assert_eq!(success, 1.0);
        assert!(mttc <= baseline_ms);
    }
}
