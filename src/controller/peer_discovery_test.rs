//! Unit tests for peer discovery logic
//!
//! Covers: peer list building from StellarNode CRDs, DNS lookup mocking,
//! peer scoring/selection, and edge cases (empty list, all-unreachable).

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::controller::peer_discovery::{PeerDiscoveryConfig, PeerInfo};
    use crate::crd::NodeType;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn make_peer(name: &str, namespace: &str, ip: &str, port: u16) -> PeerInfo {
        PeerInfo {
            name: name.to_string(),
            namespace: namespace.to_string(),
            node_type: NodeType::Validator,
            ip: ip.to_string(),
            port,
        }
    }

    fn default_config() -> PeerDiscoveryConfig {
        PeerDiscoveryConfig::default()
    }

    // -------------------------------------------------------------------------
    // PeerDiscoveryConfig defaults
    // -------------------------------------------------------------------------

    #[test]
    fn test_config_default_values() {
        let cfg = default_config();
        assert_eq!(cfg.config_namespace, "stellar-system");
        assert_eq!(cfg.config_map_name, "stellar-peers");
        assert_eq!(cfg.peer_port, 11625);
    }

    // -------------------------------------------------------------------------
    // PeerInfo: to_peer_string
    // -------------------------------------------------------------------------

    #[test]
    fn test_peer_string_format() {
        let peer = make_peer("validator-0", "stellar-system", "10.0.0.1", 11625);
        assert_eq!(peer.to_peer_string(), "10.0.0.1:11625");
    }

    #[test]
    fn test_peer_string_non_default_port() {
        let peer = make_peer("validator-1", "stellar-system", "192.168.1.50", 9999);
        assert_eq!(peer.to_peer_string(), "192.168.1.50:9999");
    }

    // -------------------------------------------------------------------------
    // PeerInfo: to_json
    // -------------------------------------------------------------------------

    #[test]
    fn test_peer_json_contains_expected_fields() {
        let peer = make_peer("validator-0", "stellar-system", "10.0.0.1", 11625);
        let json = peer.to_json();

        assert_eq!(json["name"], "validator-0");
        assert_eq!(json["namespace"], "stellar-system");
        assert_eq!(json["nodeType"], "Validator");
        assert_eq!(json["ip"], "10.0.0.1");
        assert_eq!(json["port"], 11625);
        assert_eq!(json["peerString"], "10.0.0.1:11625");
    }

    #[test]
    fn test_peer_json_node_type_horizon() {
        let peer = PeerInfo {
            name: "horizon-0".to_string(),
            namespace: "default".to_string(),
            node_type: NodeType::Horizon,
            ip: "10.0.0.2".to_string(),
            port: 11625,
        };
        assert_eq!(peer.to_json()["nodeType"], "Horizon");
    }

    #[test]
    fn test_peer_json_node_type_soroban() {
        let peer = PeerInfo {
            name: "soroban-0".to_string(),
            namespace: "default".to_string(),
            node_type: NodeType::SorobanRpc,
            ip: "10.0.0.3".to_string(),
            port: 11625,
        };
        assert_eq!(peer.to_json()["nodeType"], "SorobanRpc");
    }

    // -------------------------------------------------------------------------
    // Peer list building from CRD data
    // (simulates what process_node_event does when service IPs are available)
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_peer_list_from_multiple_validators() {
        // Simulate the IPs that would be returned by extract_peer_info for each node.
        let peers: HashSet<PeerInfo> = [
            make_peer("validator-0", "stellar-system", "10.0.0.1", 11625),
            make_peer("validator-1", "stellar-system", "10.0.0.2", 11625),
            make_peer("validator-2", "stellar-system", "10.0.0.3", 11625),
        ]
        .into_iter()
        .collect();

        assert_eq!(peers.len(), 3);

        let peer_strings: HashSet<String> = peers.iter().map(|p| p.to_peer_string()).collect();
        assert!(peer_strings.contains("10.0.0.1:11625"));
        assert!(peer_strings.contains("10.0.0.2:11625"));
        assert!(peer_strings.contains("10.0.0.3:11625"));
    }

    #[test]
    fn test_non_validator_nodes_excluded_from_peer_list() {
        // Only Validator nodes should ever enter the peer set;
        // process_node_event returns early for non-Validator node_type.
        let all_nodes = [
            PeerInfo {
                name: "validator-0".to_string(),
                namespace: "stellar-system".to_string(),
                node_type: NodeType::Validator,
                ip: "10.0.0.1".to_string(),
                port: 11625,
            },
            PeerInfo {
                name: "horizon-0".to_string(),
                namespace: "stellar-system".to_string(),
                node_type: NodeType::Horizon,
                ip: "10.0.0.4".to_string(),
                port: 11625,
            },
            PeerInfo {
                name: "soroban-0".to_string(),
                namespace: "stellar-system".to_string(),
                node_type: NodeType::SorobanRpc,
                ip: "10.0.0.5".to_string(),
                port: 11625,
            },
        ];

        // Replicate the guard from process_node_event
        let validators: Vec<&PeerInfo> = all_nodes
            .iter()
            .filter(|p| p.node_type == NodeType::Validator)
            .collect();

        assert_eq!(validators.len(), 1);
        assert_eq!(validators[0].name, "validator-0");
    }

    // -------------------------------------------------------------------------
    // DNS lookup mock: address resolution
    // In production, extract_peer_info queries the k8s Service for an IP.
    // Here we model the same behaviour with synchronous helpers to confirm
    // that the resolution result is handled correctly.
    // -------------------------------------------------------------------------

    /// Simulate a successful DNS/Service IP resolution.
    fn mock_resolve_success(hostname: &str) -> Option<String> {
        // Mimics a happy-path resolution that would come back from
        // the k8s Service cluster-IP lookup.
        match hostname {
            "validator-0.stellar-system" => Some("10.0.0.1".to_string()),
            "validator-1.stellar-system" => Some("10.0.0.2".to_string()),
            _ => None,
        }
    }

    /// Simulate a failing DNS/Service IP resolution (service not yet ready).
    fn mock_resolve_unreachable(_hostname: &str) -> Option<String> {
        None
    }

    #[test]
    fn test_dns_lookup_returns_ip_on_success() {
        let ip = mock_resolve_success("validator-0.stellar-system");
        assert_eq!(ip, Some("10.0.0.1".to_string()));
    }

    #[test]
    fn test_dns_lookup_returns_none_for_unknown_host() {
        let ip = mock_resolve_success("unknown.stellar-system");
        assert!(ip.is_none());
    }

    #[test]
    fn test_peer_built_from_resolved_address() {
        let ip = mock_resolve_success("validator-1.stellar-system").unwrap();
        let peer = PeerInfo {
            name: "validator-1".to_string(),
            namespace: "stellar-system".to_string(),
            node_type: NodeType::Validator,
            ip,
            port: 11625,
        };
        assert_eq!(peer.to_peer_string(), "10.0.0.2:11625");
    }

    // -------------------------------------------------------------------------
    // Peer scoring / selection
    // There is no explicit scoring function yet; the operator uses the full
    // validator set. These tests validate the selection invariants:
    // - All resolved validators are selected.
    // - Unresolved (no IP) peers are excluded.
    // - Duplicates are deduplicated by the HashSet.
    // -------------------------------------------------------------------------

    #[test]
    fn test_selection_excludes_unresolved_peers() {
        let hostnames = [
            "validator-0.stellar-system",
            "validator-1.stellar-system",
            "validator-2.stellar-system", // will not resolve
        ];

        let selected: HashSet<PeerInfo> = hostnames
            .iter()
            .filter_map(|h| {
                mock_resolve_success(h).map(|ip| PeerInfo {
                    name: h.to_string(),
                    namespace: "stellar-system".to_string(),
                    node_type: NodeType::Validator,
                    ip,
                    port: 11625,
                })
            })
            .collect();

        // Only two hosts resolve successfully
        assert_eq!(selected.len(), 2);
        let ips: Vec<String> = selected.iter().map(|p| p.ip.clone()).collect();
        assert!(ips.contains(&"10.0.0.1".to_string()));
        assert!(ips.contains(&"10.0.0.2".to_string()));
    }

    #[test]
    fn test_selection_deduplicates_identical_peers() {
        let mut peers: HashSet<PeerInfo> = HashSet::new();
        let peer = make_peer("validator-0", "stellar-system", "10.0.0.1", 11625);
        peers.insert(peer.clone());
        peers.insert(peer); // duplicate – HashSet must keep only one

        assert_eq!(peers.len(), 1);
    }

    // -------------------------------------------------------------------------
    // Edge case: empty peer list
    // -------------------------------------------------------------------------

    #[test]
    fn test_empty_peer_list_produces_empty_strings() {
        let peers: HashSet<PeerInfo> = HashSet::new();

        let peer_strings: Vec<String> = peers.iter().map(|p| p.to_peer_string()).collect();
        assert!(peer_strings.is_empty());
    }

    #[test]
    fn test_empty_peer_list_json_serialises_to_empty_array() {
        let peers: HashSet<PeerInfo> = HashSet::new();
        let json_peers: Vec<serde_json::Value> = peers.iter().map(|p| p.to_json()).collect();
        let serialised = serde_json::to_string(&json_peers).unwrap();
        assert_eq!(serialised, "[]");
    }

    // -------------------------------------------------------------------------
    // Edge case: all peers unreachable
    // -------------------------------------------------------------------------

    #[test]
    fn test_all_peers_unreachable_results_in_empty_set() {
        let hostnames = [
            "validator-0.stellar-system",
            "validator-1.stellar-system",
            "validator-2.stellar-system",
        ];

        let selected: HashSet<PeerInfo> = hostnames
            .iter()
            .filter_map(|h| {
                mock_resolve_unreachable(h).map(|ip| PeerInfo {
                    name: h.to_string(),
                    namespace: "stellar-system".to_string(),
                    node_type: NodeType::Validator,
                    ip,
                    port: 11625,
                })
            })
            .collect();

        assert!(selected.is_empty());
    }

    // -------------------------------------------------------------------------
    // ConfigMap round-trip (peers.json serialise → deserialise)
    // Mirrors the logic inside update_peers_config_map / get_peers_from_config_map.
    // -------------------------------------------------------------------------

    #[test]
    fn test_peers_json_round_trip() {
        let original: Vec<PeerInfo> = vec![
            make_peer("validator-0", "stellar-system", "10.0.0.1", 11625),
            make_peer("validator-1", "stellar-system", "10.0.0.2", 11625),
        ];

        // Serialise (as done in update_peers_config_map)
        let json_values: Vec<serde_json::Value> = original.iter().map(|p| p.to_json()).collect();
        let json_str = serde_json::to_string_pretty(&json_values).unwrap();

        // Deserialise (as done in get_peers_from_config_map)
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&json_str).unwrap();
        let restored: Vec<PeerInfo> = parsed
            .iter()
            .filter_map(|v| {
                Some(PeerInfo {
                    name: v.get("name")?.as_str()?.to_string(),
                    namespace: v.get("namespace")?.as_str()?.to_string(),
                    node_type: match v.get("nodeType")?.as_str()? {
                        "Validator" => NodeType::Validator,
                        "Horizon" => NodeType::Horizon,
                        "SorobanRpc" => NodeType::SorobanRpc,
                        _ => return None,
                    },
                    ip: v.get("ip")?.as_str()?.to_string(),
                    port: v.get("port")?.as_u64()? as u16,
                })
            })
            .collect();

        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].name, original[0].name);
        assert_eq!(restored[0].ip, original[0].ip);
        assert_eq!(restored[1].name, original[1].name);
        assert_eq!(restored[1].ip, original[1].ip);
    }

    #[test]
    fn test_peers_txt_format() {
        // Verify the newline-separated peer list format used in ConfigMap peers.txt
        let peers = [
            make_peer("validator-0", "stellar-system", "10.0.0.1", 11625),
            make_peer("validator-1", "stellar-system", "10.0.0.2", 11625),
        ];

        let peers_txt: Vec<String> = peers.iter().map(|p| p.to_peer_string()).collect();
        let output = peers_txt.join("\n");

        assert!(output.contains("10.0.0.1:11625"));
        assert!(output.contains("10.0.0.2:11625"));
        assert!(output.contains('\n'));
    }

    #[test]
    fn test_peer_count_matches_set_size() {
        let peers: HashSet<PeerInfo> = [
            make_peer("validator-0", "stellar-system", "10.0.0.1", 11625),
            make_peer("validator-1", "stellar-system", "10.0.0.2", 11625),
            make_peer("validator-2", "stellar-system", "10.0.0.3", 11625),
        ]
        .into_iter()
        .collect();

        let peer_count = peers.len().to_string();
        assert_eq!(peer_count, "3");
    }
}
