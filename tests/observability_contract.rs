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
//! Integration tests for the unified observability contract (issue #1481).

use std::collections::HashMap;
use std::process::Command;
use stellar_k8s::observability_contract::{
    emit_golden_path_signals, generate_instrumentation_source, validate_resource_attributes,
    ContractIngest, CorrelationIdentity, IngestDecision, PodIdentity, SignalEnvelope, SignalKind,
    CONTRACT_VERSION, CONTRACT_VERSION_KEY,
};

fn golden_identity() -> PodIdentity {
    PodIdentity {
        service_name: "stellar-golden-path".into(),
        service_instance_id: "11111111-2222-3333-4444-555555555555".into(),
        pod_name: "stellar-golden-path-0".into(),
        namespace: "stellar-system".into(),
        node_name: "kind-control-plane".into(),
        pod_uid: Some("11111111-2222-3333-4444-555555555555".into()),
        cluster_name: Some("kind".into()),
        container_name: Some("golden-path".into()),
        deployment_name: Some("stellar-golden-path".into()),
        service_version: Some(env!("CARGO_PKG_VERSION").into()),
        environment: Some("test".into()),
    }
}

#[test]
fn emitted_signals_carry_full_required_attribute_set() {
    let required = stellar_k8s::observability_contract::required_attribute_keys();
    for signal in emit_golden_path_signals(&golden_identity()) {
        for key in &required {
            assert!(
                signal.resource.contains_key(key),
                "{:?} missing required attribute {key}",
                signal.kind
            );
        }
        assert_eq!(
            signal
                .resource
                .get(CONTRACT_VERSION_KEY)
                .map(String::as_str),
            Some(CONTRACT_VERSION)
        );
    }
}

#[test]
fn valid_logs_metrics_traces_are_accepted() {
    let ingest = ContractIngest::new();
    for signal in emit_golden_path_signals(&golden_identity()) {
        assert!(
            ingest.ingest(signal).is_accepted(),
            "golden-path signal must be accepted"
        );
    }
    assert_eq!(ingest.accepted(), 3);
    assert_eq!(ingest.quarantined(), 0);
}

#[test]
fn missing_required_attribute_is_quarantined_not_dropped() {
    let ingest = ContractIngest::new();
    let mut resource = golden_identity().resource_attributes();
    resource.remove("k8s.node.name");
    let payload = serde_json::json!({"name": "missing-node"});
    ingest.ingest(SignalEnvelope {
        kind: SignalKind::Traces,
        resource: resource.clone(),
        payload: payload.clone(),
    });
    assert_eq!(ingest.accepted(), 0);
    assert_eq!(ingest.quarantined(), 1);
    let rec = &ingest.dead_letters()[0];
    assert_eq!(rec.payload, payload);
    assert!(rec.reason.contains("k8s.node.name"));
}

#[test]
fn invalid_attribute_value_is_quarantined() {
    let ingest = ContractIngest::new();
    let mut resource = golden_identity().resource_attributes();
    resource.insert(CONTRACT_VERSION_KEY.into(), "not-a-semver".into());
    match ingest.ingest(SignalEnvelope {
        kind: SignalKind::Metrics,
        resource,
        payload: serde_json::json!({"name": "bad-version"}),
    }) {
        IngestDecision::Quarantined { reason } => {
            assert!(
                reason.contains("stellar.observability.contract.version")
                    || reason.contains("invalid")
            );
        }
        IngestDecision::Accepted => panic!("invalid version must be quarantined"),
    }
}

#[test]
fn every_accepted_signal_embeds_contract_version() {
    for kind in [SignalKind::Logs, SignalKind::Metrics, SignalKind::Traces] {
        let mut resource = golden_identity().resource_attributes();
        assert_eq!(
            resource.get(CONTRACT_VERSION_KEY).unwrap(),
            CONTRACT_VERSION
        );
        let decision = validate_resource_attributes(&resource);
        assert!(decision.is_accepted(), "{kind:?} {decision:?}");
        resource.insert(CONTRACT_VERSION_KEY.into(), CONTRACT_VERSION.into());
        assert_eq!(
            CorrelationIdentity::from_resource(&resource)
                .unwrap()
                .contract_version,
            CONTRACT_VERSION
        );
    }
}

#[test]
fn generated_instrumentation_passes_contract() {
    let src = generate_instrumentation_source("stellar-golden-path");
    assert!(src.contains("stellar-golden-path"));
    assert!(src.contains(CONTRACT_VERSION));
    let ingest = ContractIngest::new();
    for signal in emit_golden_path_signals(&golden_identity()) {
        assert!(ingest.ingest(signal).is_accepted());
    }
}

#[test]
fn grafana_correlation_uses_canonical_identity() {
    let signals = emit_golden_path_signals(&golden_identity());
    let log = signals.iter().find(|s| s.kind == SignalKind::Logs).unwrap();
    let trace = signals
        .iter()
        .find(|s| s.kind == SignalKind::Traces)
        .unwrap();
    let log_id = CorrelationIdentity::from_resource(&log.resource).unwrap();
    let trace_id = CorrelationIdentity::from_resource(&trace.resource).unwrap();
    assert_eq!(log_id, trace_id);
    assert_eq!(log.payload.get("trace_id"), trace.payload.get("trace_id"));
}

#[test]
fn collector_template_routes_dead_letter_not_drop() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let template =
        std::fs::read_to_string(root.join("charts/stellar-operator/templates/otel-collector.yaml"))
            .expect("collector template");
    assert!(template.contains("file/deadletter"));
    assert!(template.contains("traces/deadletter"));
    assert!(template.contains("metrics/deadletter"));
    assert!(template.contains("logs/deadletter"));
    assert!(template.contains("filter/keep_invalid"));
    assert!(template.contains("stellar.observability.contract.version"));
    assert!(template.contains("health_check"));
}

#[test]
fn generator_cli_writes_compliant_scaffold() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = tempfile::tempdir().expect("tmpdir");
    let dest = out.path().join("observability_instrumentation.rs");
    let status = Command::new("python3")
        .arg(root.join("scripts/generate-observability-instrumentation.py"))
        .arg("--service")
        .arg("stellar-golden-path")
        .arg("--out")
        .arg(&dest)
        .status()
        .expect("run generator");
    assert!(status.success());
    let generated = std::fs::read_to_string(dest).unwrap();
    assert!(generated.contains("REQUIRED_RESOURCE_KEYS"));
    assert!(generated.contains("k8s.pod.name"));
    assert!(generated.contains(CONTRACT_VERSION));
}

#[test]
fn ci_lint_blocks_unknown_attribute() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let status = Command::new("python3")
        .arg("-m")
        .arg("unittest")
        .arg("scripts.tests.test_lint_observability_contract")
        .current_dir(root)
        .status()
        .expect("lint unit tests");
    assert!(status.success());
}

#[test]
fn grafana_dashboard_has_correlation_fields() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dash = std::fs::read_to_string(root.join("monitoring/grafana-log-trace-correlation.json"))
        .expect("dashboard");
    for needle in [
        "service.instance.id",
        "k8s.pod.name",
        "trace_id",
        "stellar_observability_contract_deadletter_rate",
        "internalLink",
    ] {
        assert!(dash.contains(needle), "dashboard missing {needle}");
    }
}

#[test]
fn empty_resource_is_not_accepted() {
    let empty: HashMap<String, String> = HashMap::new();
    assert!(!validate_resource_attributes(&empty).is_accepted());
}

/// The CI lint (`scripts/ci/lint-observability-contract.py`) and the runtime
/// validator must judge every shared fixture identically (#1960).
#[test]
fn runtime_validator_agrees_with_shared_ci_fixtures() {
    use stellar_k8s::observability_contract::validate_resource_attributes;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/observability");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("fixtures dir") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let attrs: std::collections::HashMap<String, String> = doc["resource"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        let decision = validate_resource_attributes(&attrs);
        // `runtime` overrides `expect` where ingest is intentionally more lenient than CI.
        let expect_valid = match doc.get("runtime").and_then(|v| v.as_str()) {
            Some("accepted") => true,
            Some(other) => panic!("{}: unknown runtime expectation {other}", path.display()),
            None => doc["expect"] == "valid",
        };
        assert_eq!(
            decision.is_accepted(),
            expect_valid,
            "{}: runtime validator disagrees with fixture expectation ({decision:?})",
            path.display()
        );
        seen += 1;
    }
    assert!(seen >= 10, "expected the shared fixture set, found {seen}");
}
