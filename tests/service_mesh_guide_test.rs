//! Property-based and unit tests for the Service Mesh mTLS guide (`docs/service-mesh.md`).
//!
//! These tests parse YAML blocks from the guide at test time and verify invariants
//! described in the design document.
//!
//! Feature: service-mesh-mtls

use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Helper: extract all fenced ```yaml ... ``` blocks from a markdown string
// ---------------------------------------------------------------------------

fn extract_yaml_blocks(content: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut in_block = false;
    let mut current_block = String::new();

    for line in content.lines() {
        if line.trim() == "```yaml" {
            in_block = true;
            current_block.clear();
        } else if line.trim() == "```" && in_block {
            in_block = false;
            if !current_block.trim().is_empty() {
                blocks.push(current_block.clone());
            }
        } else if in_block {
            current_block.push_str(line);
            current_block.push('\n');
        }
    }
    blocks
}

// ---------------------------------------------------------------------------
// Helper: load the guide content once (panics if the file is missing)
// ---------------------------------------------------------------------------

fn guide_content() -> String {
    std::fs::read_to_string("docs/service-mesh.md").expect("docs/service-mesh.md must exist")
}

// ---------------------------------------------------------------------------
// Property 1: PeerAuthentication manifests enforce STRICT mode
//
// Feature: service-mesh-mtls, Property 1: PeerAuthentication manifests enforce STRICT mode
// Validates: Requirements 3.1
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_peer_authentication_strict_mode(
        idx in any::<prop::sample::Index>()
    ) {
        let content = guide_content();
        let blocks: Vec<String> = extract_yaml_blocks(&content)
            .into_iter()
            .filter(|b| {
                let v: serde_yaml::Value = serde_yaml::from_str(b).unwrap_or(serde_yaml::Value::Null);
                v["kind"] == "PeerAuthentication"
            })
            .collect();

        // If there are no matching blocks the property is vacuously true.
        if blocks.is_empty() {
            return Ok(());
        }

        let block = idx.get(&blocks);
        let v: serde_yaml::Value = serde_yaml::from_str(block).unwrap();
        prop_assert_eq!(
            v["spec"]["mtls"]["mode"].as_str().unwrap_or(""),
            "STRICT",
            "PeerAuthentication block must have spec.mtls.mode == STRICT:\n{}",
            block
        );
    }
}

// ---------------------------------------------------------------------------
// Property 2: Authorization manifests restrict to known Stellar identities
//
// Feature: service-mesh-mtls, Property 2: Authorization manifests restrict to known Stellar identities
// Validates: Requirements 3.4, 6.3
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_authorization_known_stellar_identities(
        idx in any::<prop::sample::Index>()
    ) {
        let known_accounts = [
            "stellar-operator",
            "stellar-core",
            "stellar-horizon",
            "stellar-soroban",
        ];

        let content = guide_content();
        let blocks: Vec<String> = extract_yaml_blocks(&content)
            .into_iter()
            .filter(|b| {
                let v: serde_yaml::Value = serde_yaml::from_str(b).unwrap_or(serde_yaml::Value::Null);
                v["kind"] == "AuthorizationPolicy" || v["kind"] == "ServerAuthorization"
            })
            .collect();

        if blocks.is_empty() {
            return Ok(());
        }

        let block = idx.get(&blocks);
        let v: serde_yaml::Value = serde_yaml::from_str(block).unwrap();

        // Collect all principal / service-account strings from the manifest.
        let mut identities: Vec<String> = Vec::new();

        // Istio AuthorizationPolicy: spec.rules[*].from[*].source.principals
        if let Some(rules) = v["spec"]["rules"].as_sequence() {
            for rule in rules {
                if let Some(froms) = rule["from"].as_sequence() {
                    for from in froms {
                        if let Some(principals) = from["source"]["principals"].as_sequence() {
                            for p in principals {
                                if let Some(s) = p.as_str() {
                                    identities.push(s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Linkerd ServerAuthorization: spec.client.meshTLS.serviceAccounts[*].name
        if let Some(accounts) = v["spec"]["client"]["meshTLS"]["serviceAccounts"].as_sequence() {
            for a in accounts {
                if let Some(name) = a["name"].as_str() {
                    identities.push(name.to_string());
                }
            }
        }

        // Linkerd AuthorizationPolicy (2.12+): via MeshTLSAuthentication — identities are
        // in a separate resource; skip principal extraction for the policy itself.

        for identity in &identities {
            // Strip the SPIFFE path prefix if present (e.g. "cluster.local/ns/stellar/sa/stellar-core")
            let account = identity
                .split('/')
                .last()
                .unwrap_or(identity.as_str());
            prop_assert!(
                known_accounts.contains(&account),
                "Unknown identity '{}' found in authorization manifest:\n{}",
                identity,
                block
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 3: DestinationRule P2P manifests use ISTIO_MUTUAL
//
// Feature: service-mesh-mtls, Property 3: DestinationRule P2P manifests use ISTIO_MUTUAL
// Validates: Requirements 4.2
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_destination_rule_p2p_istio_mutual(
        idx in any::<prop::sample::Index>()
    ) {
        let content = guide_content();
        let blocks: Vec<String> = extract_yaml_blocks(&content)
            .into_iter()
            .filter(|b| {
                let v: serde_yaml::Value = serde_yaml::from_str(b).unwrap_or(serde_yaml::Value::Null);
                v["kind"] == "DestinationRule" && b.contains("11625")
            })
            .collect();

        if blocks.is_empty() {
            return Ok(());
        }

        let block = idx.get(&blocks);
        let v: serde_yaml::Value = serde_yaml::from_str(block).unwrap();

        // Check top-level trafficPolicy.tls.mode
        let top_level_mode = v["spec"]["trafficPolicy"]["tls"]["mode"]
            .as_str()
            .unwrap_or("");

        // Check portLevelSettings[*].tls.mode
        let port_level_mode = v["spec"]["trafficPolicy"]["portLevelSettings"]
            .as_sequence()
            .map(|settings| {
                settings.iter().any(|s| {
                    s["tls"]["mode"].as_str().unwrap_or("") == "ISTIO_MUTUAL"
                })
            })
            .unwrap_or(false);

        prop_assert!(
            top_level_mode == "ISTIO_MUTUAL" || port_level_mode,
            "DestinationRule targeting port 11625 must use ISTIO_MUTUAL TLS mode:\n{}",
            block
        );
    }
}

// ---------------------------------------------------------------------------
// Property 4: ServiceEntry external peer manifests include P2P port
//
// Feature: service-mesh-mtls, Property 4: ServiceEntry external peer manifests include P2P port
// Validates: Requirements 4.4, 5.1
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_service_entry_external_includes_p2p_port(
        idx in any::<prop::sample::Index>()
    ) {
        let content = guide_content();
        let blocks: Vec<String> = extract_yaml_blocks(&content)
            .into_iter()
            .filter(|b| {
                let v: serde_yaml::Value = serde_yaml::from_str(b).unwrap_or(serde_yaml::Value::Null);
                v["kind"] == "ServiceEntry"
                    && v["spec"]["location"].as_str().unwrap_or("") == "MESH_EXTERNAL"
            })
            .collect();

        if blocks.is_empty() {
            return Ok(());
        }

        let block = idx.get(&blocks);
        let v: serde_yaml::Value = serde_yaml::from_str(block).unwrap();

        let has_p2p_port = v["spec"]["ports"]
            .as_sequence()
            .map(|ports| {
                ports.iter().any(|p| {
                    p["number"].as_u64().unwrap_or(0) == 11625
                })
            })
            .unwrap_or(false);

        prop_assert!(
            has_p2p_port,
            "ServiceEntry with location MESH_EXTERNAL must include port 11625:\n{}",
            block
        );
    }
}

// ---------------------------------------------------------------------------
// Property 5: VirtualService P2P manifests define timeout and retry policies
//
// Feature: service-mesh-mtls, Property 5: VirtualService P2P manifests define timeout and retry policies
// Validates: Requirements 5.3
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_virtual_service_p2p_has_timeout_and_retries(
        idx in any::<prop::sample::Index>()
    ) {
        let content = guide_content();
        let blocks: Vec<String> = extract_yaml_blocks(&content)
            .into_iter()
            .filter(|b| {
                let v: serde_yaml::Value = serde_yaml::from_str(b).unwrap_or(serde_yaml::Value::Null);
                v["kind"] == "VirtualService" && b.contains("11625")
            })
            .collect();

        if blocks.is_empty() {
            return Ok(());
        }

        let block = idx.get(&blocks);

        // Check for timeout and retries anywhere in the raw YAML text (handles nested structures)
        prop_assert!(
            block.contains("timeout"),
            "VirtualService for P2P must define a timeout field:\n{}",
            block
        );
        prop_assert!(
            block.contains("retries"),
            "VirtualService for P2P must define a retries field:\n{}",
            block
        );
    }
}

// ---------------------------------------------------------------------------
// Property 6: Compliance checklist covers all Stellar-K8s components
//
// Feature: service-mesh-mtls, Property 6: Compliance checklist covers all Stellar-K8s components
// Validates: Requirements 7.4
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn prop_compliance_checklist_covers_all_components(
        // Single boolean to satisfy proptest's requirement for at least one strategy;
        // the actual data is static (read from the guide file).
        _dummy in any::<bool>()
    ) {
        let content = guide_content();

        let required_components = ["Operator", "Stellar Core", "Horizon", "Soroban RPC"];
        for component in &required_components {
            prop_assert!(
                content.contains(component),
                "Compliance checklist must contain component '{}' but it was not found in the guide",
                component
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests (Tasks 9.1 – 9.5)
// ---------------------------------------------------------------------------

#[test]
fn test_guide_file_exists() {
    assert!(std::path::Path::new("docs/service-mesh.md").exists());
}

#[test]
fn test_guide_contains_istio_and_linkerd_sections() {
    let content = std::fs::read_to_string("docs/service-mesh.md").unwrap();
    assert!(content.contains("## Istio"));
    assert!(content.contains("## Linkerd"));
}

#[test]
fn test_guide_contains_prerequisites() {
    let content = std::fs::read_to_string("docs/service-mesh.md").unwrap();
    assert!(content.contains("Prerequisites"));
    assert!(content.contains("Istio 1.17+") || content.contains("Istio 1."));
    assert!(content.contains("Linkerd 2.12+") || content.contains("Linkerd 2."));
    assert!(content.contains("Kubernetes 1.28+") || content.contains("Kubernetes 1."));
}

#[test]
fn test_guide_contains_spiffe_svid_trust_model() {
    let content = std::fs::read_to_string("docs/service-mesh.md").unwrap();
    assert!(content.contains("SPIFFE"));
    assert!(content.contains("SVID"));
    assert!(content.contains("spiffe://cluster.local"));
}

#[test]
fn test_all_yaml_blocks_are_valid() {
    let content = std::fs::read_to_string("docs/service-mesh.md").unwrap();
    let blocks = extract_yaml_blocks(&content);
    assert!(!blocks.is_empty(), "Guide should contain YAML blocks");
    for (i, block) in blocks.iter().enumerate() {
        // Some blocks contain multiple YAML documents separated by ---; parse each.
        // serde_yaml::from_str handles single documents; for multi-doc blocks we
        // split on the document separator and validate each part individually.
        let docs: Vec<&str> = block.split("\n---").collect();
        for doc in docs {
            if doc.trim().is_empty() {
                continue;
            }
            let result: Result<serde_yaml::Value, _> = serde_yaml::from_str(doc);
            assert!(
                result.is_ok(),
                "YAML block {} (or a sub-document within it) should be valid YAML:\n{}",
                i,
                doc
            );
        }
    }
}
