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
//! Integration coverage for the `stellar-core.cfg` auto-generation (#1551).
//!
//! The tests live in `tests/` on purpose: they exercise the public API of the
//! generator the way an operator would use it, without depending on the
//! internal layout of the controller.

use stellar_k8s::controller::network_defaults::{
    generate, GenerationRequest, NetworkProfile, KNOWN_PEERS_KEY, NETWORK_PASSPHRASE_KEY,
};
use stellar_k8s::crd::StellarNetwork;

/// The SDF published testnet configuration.
const SDF_TESTNET: &str = include_str!("fixtures/stellar_core_cfg/sdf/stellar-core_testnet.cfg");
/// The SDF published mainnet configuration.
const SDF_MAINNET: &str = include_str!("fixtures/stellar_core_cfg/sdf/stellar-core_mainnet.cfg");

fn parse_toml(text: &str) -> toml::Value {
    toml::from_str(text).unwrap_or_else(|err| panic!("config is not valid TOML: {err}\n{text}"))
}

fn generate_for(network: &StellarNetwork, user_config: &str) -> String {
    let request = GenerationRequest {
        network,
        custom_passphrase: None,
        passphrase_in_secret: false,
        user_config,
    };
    let generated = generate(&request);
    format!(
        "{}{}",
        generated.render_root_keys(),
        generated.render_table_sections()
    )
}

#[test]
fn testnet_defaults_match_the_published_sdf_configuration() {
    let generated = generate_for(&StellarNetwork::Testnet, "");
    let generated = parse_toml(&generated);
    let sdf = parse_toml(SDF_TESTNET);

    assert_eq!(
        generated
            .get(NETWORK_PASSPHRASE_KEY)
            .and_then(|v| v.as_str()),
        sdf.get(NETWORK_PASSPHRASE_KEY).and_then(|v| v.as_str()),
        "the generated passphrase must match the SDF configuration"
    );

    let generated_peers: Vec<&str> = generated
        .get(KNOWN_PEERS_KEY)
        .and_then(|v| v.as_array())
        .map(|peers| peers.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();
    let sdf_peers: Vec<&str> = sdf
        .get(KNOWN_PEERS_KEY)
        .and_then(|v| v.as_array())
        .map(|peers| peers.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default();

    assert!(
        !sdf_peers.is_empty(),
        "the SDF fixture must contain known peers"
    );
    assert_eq!(
        generated_peers, sdf_peers,
        "the generated known peers must match the SDF configuration"
    );

    let generated_domains = generated.get("HOME_DOMAINS").and_then(|v| v.as_array());
    let sdf_domains = sdf.get("HOME_DOMAINS").and_then(|v| v.as_array());
    assert_eq!(
        generated_domains.map(|v| v.len()),
        sdf_domains.map(|v| v.len()),
        "the generated home domains must match the SDF configuration"
    );
}

#[test]
fn mainnet_defaults_match_the_published_sdf_configuration() {
    let generated = parse_toml(&generate_for(&StellarNetwork::Mainnet, ""));
    let sdf = parse_toml(SDF_MAINNET);

    assert_eq!(
        generated
            .get(NETWORK_PASSPHRASE_KEY)
            .and_then(|v| v.as_str()),
        sdf.get(NETWORK_PASSPHRASE_KEY).and_then(|v| v.as_str())
    );

    let generated_validators = generated.get("VALIDATORS").and_then(|v| v.as_array());
    let sdf_validators = sdf.get("VALIDATORS").and_then(|v| v.as_array());
    assert_eq!(
        generated_validators.map(|v| v.len()),
        sdf_validators.map(|v| v.len()),
        "the generated validator list must match the SDF configuration"
    );

    let generated_peers = generated
        .get(KNOWN_PEERS_KEY)
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or_default();
    let sdf_peers = sdf
        .get(KNOWN_PEERS_KEY)
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or_default();
    assert_eq!(generated_peers, sdf_peers);
}

#[test]
fn a_user_quorum_replaces_the_generated_quorum() {
    let user = r#"[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["GCEZW7", "GCB2F5"]"#;

    let generated = generate_for(&StellarNetwork::Testnet, user);
    let parsed = parse_toml(&generated);

    assert!(
        parsed.get("QUORUM_SET").is_some(),
        "the user quorum must survive: {generated}"
    );
    assert!(
        parsed.get("HOME_DOMAINS").is_none(),
        "stellar-core rejects a mix of [QUORUM_SET] and [[HOME_DOMAINS]]: {generated}"
    );
    assert!(
        parsed.get("VALIDATORS").is_none(),
        "stellar-core rejects a mix of [QUORUM_SET] and [[VALIDATORS]]: {generated}"
    );
    assert!(
        parsed.get(NETWORK_PASSPHRASE_KEY).is_some(),
        "the passphrase is independent of the quorum: {generated}"
    );
}

#[test]
fn a_user_passphrase_wins_over_the_generated_one() {
    let user = r#"NETWORK_PASSPHRASE="My Own Network""#;
    let generated = generate_for(&StellarNetwork::Testnet, user);
    let parsed = parse_toml(&generated);

    assert_eq!(
        parsed.get(NETWORK_PASSPHRASE_KEY).and_then(|v| v.as_str()),
        Some("My Own Network")
    );
}

#[test]
fn a_secret_backed_passphrase_is_never_rendered() {
    let request = GenerationRequest {
        network: &StellarNetwork::Custom("private".to_string()),
        custom_passphrase: Some("super-secret-passphrase"),
        passphrase_in_secret: true,
        user_config: "",
    };
    let generated = generate(&request);

    assert!(
        !generated
            .render_root_keys()
            .contains("super-secret-passphrase"),
        "a secret backed passphrase must never reach the ConfigMap"
    );
    assert!(
        !generated
            .render_root_keys()
            .contains(NETWORK_PASSPHRASE_KEY),
        "a secret backed passphrase must not be rendered at all"
    );
}

#[test]
fn an_invalid_user_configuration_generates_nothing() {
    let request = GenerationRequest {
        network: &StellarNetwork::Testnet,
        custom_passphrase: None,
        passphrase_in_secret: false,
        user_config: "this is = = not toml",
    };
    let generated = generate(&request);

    assert!(
        generated.is_empty(),
        "an invalid user configuration must not produce defaults"
    );
}

#[test]
fn the_future_network_only_gets_a_passphrase() {
    let generated = generate_for(&StellarNetwork::Futurenet, "");
    let parsed = parse_toml(&generated);

    assert!(parsed.get(NETWORK_PASSPHRASE_KEY).is_some());
    assert!(parsed.get(KNOWN_PEERS_KEY).is_none());
    assert!(parsed.get("VALIDATORS").is_none());
}

#[test]
fn a_custom_network_runs_standalone() {
    let request = GenerationRequest {
        network: &StellarNetwork::Custom("private".to_string()),
        custom_passphrase: Some("private passphrase"),
        passphrase_in_secret: false,
        user_config: "",
    };
    let generated = generate(&request);
    let rendered = format!(
        "{}{}",
        generated.render_root_keys(),
        generated.render_table_sections()
    );
    let parsed = parse_toml(&rendered);

    assert_eq!(
        parsed.get("RUN_STANDALONE").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(parsed.get("QUORUM_SET").is_some());
    assert!(NetworkProfile::for_network(&StellarNetwork::Custom("x".into())).is_standalone());
}
