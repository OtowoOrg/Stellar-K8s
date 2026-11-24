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
//! Integration coverage for the zero-trust egress synthesis (#1476).

use chrono::{Duration, Utc};
use stellar_k8s::egress_synthesis::{
    cluster_destinations, render_policy_diff, CandidatePolicy, EgressSynthesizer, ObservedFlow,
    PromotionDecision, StagingMode, MAX_FALSE_POSITIVE_RATE, OBSERVE_WINDOW_DAYS,
};

fn hubble_line(src: &str, dst: &str, port: u16, protocol: &str) -> String {
    format!(
        r#"{{"source":{{"ip":"{src}","namespace":"stellar","pod_name":"validator-0"}},"destination":{{"ip":"{dst}","port":{port}}},"IPProtocol":"{protocol}","bytes":1024}}"#
    )
}

#[tokio::test]
async fn hubble_lines_are_ingested() {
    let ingestor = stellar_k8s::egress_synthesis::FlowIngestor::new();
    let lines = vec![
        hubble_line("10.0.0.5", "1.1.1.1", 443, "TCP"),
        hubble_line("10.0.0.5", "1.1.1.1", 443, "TCP"),
        "not json".to_string(),
    ];

    let stats = ingestor.ingest_lines(&lines).await;

    assert_eq!(stats.accepted, 2);
    assert_eq!(stats.rejected, 1);
    assert_eq!(ingestor.flow_count().await, 2);

    let flows = ingestor.flows().await;
    assert_eq!(flows[0].dst_ip, "1.1.1.1");
    assert_eq!(flows[0].dst_port, 443);
    assert_eq!(flows[0].protocol, "TCP");
    assert_eq!(flows[0].namespace, "stellar");
}

#[tokio::test]
async fn ingestion_keeps_up_with_ten_thousand_flows_per_second() {
    let ingestor = stellar_k8s::egress_synthesis::FlowIngestor::new();
    let flows: Vec<ObservedFlow> = (0..20_000)
        .map(|index| {
            let mut flow = ObservedFlow::new(
                "10.0.0.1",
                &format!("192.168.{}.{}", index / 256, index % 256),
                443,
                "TCP",
            );
            flow.bytes = 128;
            flow
        })
        .collect();

    let stats = ingestor.ingest(flows).await;

    assert_eq!(stats.accepted, 20_000);
    assert!(
        stats.flows_per_second() >= 10_000.0,
        "ingestion was too slow: {:.0} flows/s",
        stats.flows_per_second()
    );
}

#[test]
fn destinations_are_clustered_into_cidr_and_port_tuples() {
    let flows = vec![
        ObservedFlow::new("10.0.0.1", "10.1.2.3", 443, "TCP"),
        ObservedFlow::new("10.0.0.2", "10.1.2.9", 443, "TCP"),
        ObservedFlow::new("10.0.0.3", "10.1.3.4", 80, "TCP"),
    ];

    let clusters = cluster_destinations(&flows, 24);

    assert_eq!(clusters.len(), 2);
    assert_eq!(clusters[0].cidr, "10.1.2.0/24");
    assert_eq!(clusters[0].port, 443);
    assert_eq!(clusters[0].flow_count, 2);
    assert_eq!(clusters[0].destination_count(), 2);
    assert_eq!(clusters[1].cidr, "10.1.3.0/24");
}

#[test]
fn the_candidate_policy_is_rendered_as_a_network_policy() {
    let flows = vec![
        ObservedFlow::new("10.0.0.1", "10.1.2.3", 443, "TCP"),
        ObservedFlow::new("10.0.0.2", "10.1.2.9", 443, "TCP"),
    ];
    let clusters = cluster_destinations(&flows, 24);

    let candidate = CandidatePolicy::build(
        "validator-0",
        "stellar",
        &clusters,
        StagingMode::ObserveOnly,
    );
    let policy = candidate.to_network_policy();

    assert_eq!(policy.metadata.name.as_deref(), Some("egress-validator-0"));
    assert_eq!(policy.metadata.namespace.as_deref(), Some("stellar"));

    let spec = policy.spec.expect("the candidate must have a spec");
    assert_eq!(
        spec.policy_types,
        Some(vec!["Egress".to_string()]),
        "a synthesized policy must only cover egress"
    );

    let egress = spec.egress.expect("the candidate must have egress rules");
    assert_eq!(egress.len(), 1);
    let peers = egress[0].to.as_ref().expect("egress peers");
    assert_eq!(
        peers[0].ip_block.as_ref().map(|block| block.cidr.clone()),
        Some("10.1.2.0/24".to_string())
    );

    let labels = policy
        .metadata
        .labels
        .expect("the candidate must be labelled");
    assert_eq!(
        labels.get("stellar-k8s.io/egress-mode").map(String::as_str),
        Some("observe")
    );

    let yaml = candidate.to_yaml();
    assert!(yaml.contains("10.1.2.0/24"), "{yaml}");
    assert!(yaml.contains("Egress"), "{yaml}");
}

#[test]
fn the_diff_between_applied_and_candidate_is_readable() {
    let applied = CandidatePolicy::build(
        "validator-0",
        "stellar",
        &cluster_destinations(&[ObservedFlow::new("10.0.0.1", "10.1.2.3", 443, "TCP")], 24),
        StagingMode::ObserveOnly,
    )
    .to_network_policy();

    let candidate = CandidatePolicy::build(
        "validator-0",
        "stellar",
        &cluster_destinations(
            &[
                ObservedFlow::new("10.0.0.1", "10.1.2.3", 443, "TCP"),
                ObservedFlow::new("10.0.0.1", "10.9.9.9", 9000, "TCP"),
            ],
            24,
        ),
        StagingMode::ObserveOnly,
    )
    .to_network_policy();

    let diff = render_policy_diff(&applied, &candidate);

    assert_eq!(diff.added, vec!["10.9.9.0/24:9000/TCP".to_string()]);
    assert!(diff.removed.is_empty());
    assert_eq!(diff.unchanged, vec!["10.1.2.0/24:443/TCP".to_string()]);

    let yaml = diff.to_yaml();
    assert!(yaml.contains("added:"), "{yaml}");
    assert!(yaml.contains("10.9.9.0/24:9000/TCP"), "{yaml}");
}

#[test]
fn enforcement_waits_for_the_observe_window_and_the_false_positive_budget() {
    let start = Utc::now();
    let mut synthesizer = EgressSynthesizer::new(start);

    assert_eq!(synthesizer.staging().mode, StagingMode::ObserveOnly);
    match synthesizer.promotion(start + Duration::days(3)) {
        PromotionDecision::RemainObserveOnly { reason } => {
            assert!(reason.contains("observe window"), "{reason}");
        }
        other => panic!("expected the candidate to stay in observe-only mode: {other:?}"),
    }

    let after_window = start + Duration::days(OBSERVE_WINDOW_DAYS + 1);
    synthesizer.record_observed_flow(100_000);
    synthesizer.record_blocked_flow(10);

    assert!(
        synthesizer.staging().false_positive_rate() < MAX_FALSE_POSITIVE_RATE,
        "a rate of 0.0001 is inside the budget"
    );
    assert_eq!(
        synthesizer.promotion(after_window),
        PromotionDecision::PromoteToEnforce
    );

    let mut noisy = EgressSynthesizer::new(start);
    noisy.record_observed_flow(100_000);
    noisy.record_blocked_flow(1_000);
    match noisy.promotion(after_window) {
        PromotionDecision::RemainObserveOnly { reason } => {
            assert!(reason.contains("false-positive"), "{reason}");
        }
        other => panic!("expected the candidate to stay in observe-only mode: {other:?}"),
    }
}

#[tokio::test]
async fn the_synthesizer_produces_a_candidate_from_observed_flows() {
    let synthesizer = EgressSynthesizer::new(Utc::now());
    let stats = synthesizer
        .observe(vec![
            ObservedFlow::new("10.0.0.1", "10.1.2.3", 443, "TCP"),
            ObservedFlow::new("10.0.0.1", "10.1.2.4", 443, "TCP"),
        ])
        .await;

    assert_eq!(stats.accepted, 2);

    let candidate = synthesizer.candidate("validator-0", "stellar").await;
    assert_eq!(candidate.rules.len(), 1);
    assert_eq!(candidate.rules[0].cidr, "10.1.2.0/24");
    assert_eq!(candidate.mode, StagingMode::ObserveOnly);
}
