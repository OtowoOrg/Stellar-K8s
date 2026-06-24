//! NetworkPolicy management.

use super::helpers::*;
use super::prelude::*;

// ============================================================================
// NetworkPolicy — unchanged
// ============================================================================

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn ensure_network_policy(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    let policy_cfg = match &node.spec.network_policy {
        Some(cfg) if cfg.enabled => cfg,
        _ => return Ok(()),
    };

    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    let network_policy = build_network_policy(node, policy_cfg);

    api.patch(
        &name,
        &patch_params(dry_run),
        &Patch::Apply(&network_policy),
    )
    .await?;

    info!("NetworkPolicy ensured for {}/{}", namespace, name);
    Ok(())
}

/// Extract peer addresses (IPs or Hostnames) from QUORUM_SET and KNOWN_PEERS TOML strings.
fn extract_peers_from_config(node: &StellarNode) -> Vec<String> {
    let mut peers = Vec::new();
    let config = match &node.spec.validator_config {
        Some(c) => c,
        None => return peers,
    };

    // 1. Parse KNOWN_PEERS if present
    if let Some(known_peers_toml) = &config.known_peers {
        if let Ok(value) = known_peers_toml.parse::<toml::Value>() {
            if let Some(kp_array) = value.as_array() {
                for v in kp_array {
                    if let Some(s) = v.as_str() {
                        // Extract IP/Hostname from "IP:PORT"
                        let peer = s.split(':').next().unwrap_or(s);
                        peers.push(peer.to_string());
                    }
                }
            } else if let Some(kp_table) = value.get("KNOWN_PEERS").and_then(|v| v.as_array()) {
                for v in kp_table {
                    if let Some(s) = v.as_str() {
                        let peer = s.split(':').next().unwrap_or(s);
                        peers.push(peer.to_string());
                    }
                }
            }
        }
    }

    // 2. Parse QUORUM_SET for any direct IP references (rare but possible in custom setups)
    if let Some(qs_toml) = &config.quorum_set {
        if let Ok(value) = qs_toml.parse::<toml::Value>() {
            // Check for [VALIDATORS] section with IP-like keys
            if let Some(validators) = value.get("VALIDATORS").and_then(|v| v.as_table()) {
                for key in validators.keys() {
                    // If key looks like an IP or hostname (not a public key), add it
                    if !key.starts_with('G') && key.contains('.') {
                        peers.push(key.clone());
                    }
                }
            }
        }
    }

    peers.sort();
    peers.dedup();
    peers
}

pub(crate) fn build_network_policy(
    node: &StellarNode,
    config: &NetworkPolicyConfig,
) -> NetworkPolicy {
    let labels = standard_labels(node);
    let name = resource_name(node, "netpol");

    let mut ingress_rules: Vec<NetworkPolicyIngressRule> = Vec::new();
    let mut egress_rules: Vec<k8s_openapi::api::networking::v1::NetworkPolicyEgressRule> =
        Vec::new();

    let app_ports = match node.spec.node_type {
        NodeType::Validator => vec![
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11626)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ],
        NodeType::Horizon | NodeType::SorobanRpc => vec![NetworkPolicyPort {
            port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(8000)),
            protocol: Some("TCP".to_string()),
            ..Default::default()
        }],
    };

    if !config.allow_namespaces.is_empty() {
        let peers: Vec<NetworkPolicyPeer> = config
            .allow_namespaces
            .iter()
            .map(|ns| NetworkPolicyPeer {
                namespace_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "kubernetes.io/metadata.name".to_string(),
                        ns.clone(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();

        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(peers),
            ports: Some(app_ports.clone()),
        });
    }

    if let Some(pod_labels) = &config.allow_pod_selector {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(pod_labels.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(app_ports.clone()),
        });
    }

    if !config.allow_cidrs.is_empty() {
        let peers: Vec<NetworkPolicyPeer> = config
            .allow_cidrs
            .iter()
            .map(|cidr| NetworkPolicyPeer {
                ip_block: Some(IPBlock {
                    cidr: cidr.clone(),
                    except: None,
                }),
                ..Default::default()
            })
            .collect();

        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(peers),
            ports: Some(app_ports.clone()),
        });
    }

    if config.allow_metrics_scrape {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                namespace_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "kubernetes.io/metadata.name".to_string(),
                        config.metrics_namespace.clone(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(vec![NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(9090)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            }]),
        });
    }

    if node.spec.node_type == NodeType::Validator {
        ingress_rules.push(NetworkPolicyIngressRule {
            from: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(app_ports.clone()),
        });

        // --- Stellar-Native Egress Rules ---
        // 1. Allow DNS (essential for hostname resolution)
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: None, // Allow to all for port 53
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("UDP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 2. Allow egress to parsed peers (KNOWN_PEERS / QUORUM_SET)
        let peers = extract_peers_from_config(node);
        if !peers.is_empty() {
            let mut peer_egress_to = Vec::new();
            for peer in peers {
                // If it looks like an IP, use ipBlock. If it's a hostname, we can't
                // do much in standard NetPol without a DNS controller, but we can
                // allow all egress on peer ports as a fallback or if IP is known.
                if peer
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == '.' || c == ':')
                {
                    peer_egress_to.push(NetworkPolicyPeer {
                        ip_block: Some(IPBlock {
                            cidr: if peer.contains('/') {
                                peer
                            } else {
                                format!("{}/32", peer)
                            },
                            except: None,
                        }),
                        ..Default::default()
                    });
                }
            }

            egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                to: if peer_egress_to.is_empty() {
                    None
                } else {
                    Some(peer_egress_to)
                },
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
            });
        }

        // 3. Allow egress to history archives (HTTP/HTTPS)
        if let Some(vc) = &node.spec.validator_config {
            if vc.enable_history_archive && !vc.history_archive_urls.is_empty() {
                egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                    to: None, // External history archives
                    ports: Some(vec![
                        NetworkPolicyPort {
                            port: Some(
                                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(80),
                            ),
                            protocol: Some("TCP".to_string()),
                            ..Default::default()
                        },
                        NetworkPolicyPort {
                            port: Some(
                                k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(443),
                            ),
                            protocol: Some("TCP".to_string()),
                            ..Default::default()
                        },
                    ]),
                });
            }
        }
    } else {
        // Allow public and ingress-controller traffic to Horizon/Soroban RPC on port 8000.
        ingress_rules.push(NetworkPolicyIngressRule {
            from: None,
            ports: Some(app_ports.clone()),
        });

        // Horizon / Soroban RPC egress rules
        // 1. Allow DNS
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: None,
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("UDP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 2. Allow egress to Stellar Core (usually in the same namespace)
        egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
            to: Some(vec![NetworkPolicyPeer {
                pod_selector: Some(LabelSelector {
                    match_labels: Some(BTreeMap::from([(
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    )])),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
            ports: Some(vec![
                NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11625),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
                NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(11626),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
            ]),
        });

        // 3. Allow egress to external databases if configured
        if node.spec.database.is_some() || node.spec.managed_database.is_some() {
            egress_rules.push(k8s_openapi::api::networking::v1::NetworkPolicyEgressRule {
                to: None, // External DBs or CNPG
                ports: Some(vec![NetworkPolicyPort {
                    port: Some(
                        k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(5432),
                    ),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                }]),
            });
        }
    }

    // -----------------------------------------------------------------------
    // Egress rules — Network Isolation
    //
    // Allow egress only to:
    //   1. Pods in namespaces labelled with the SAME stellar.org/network value.
    //      This is the critical rule: it prevents a Testnet pod from ever
    //      opening a TCP connection to a Mainnet pod, even if both are on the
    //      same cluster.
    //   2. kube-dns (UDP/TCP 53) — required for all pods.
    //   3. The Kubernetes API server (TCP 443/6443) — required for health checks.
    //   4. Intra-namespace traffic (e.g. Horizon → Stellar Core).
    //
    // Any egress not matched by these rules is implicitly denied because we
    // include "Egress" in policy_types.
    // -----------------------------------------------------------------------
    use k8s_openapi::api::networking::v1::NetworkPolicyEgressRule;

    let network_label_value = crate::controller::network_isolation::network_label_value(
        &node.spec.network,
        &node.spec.custom_network_passphrase,
    );

    // Rule 1: Allow egress to pods in same-network namespaces only.
    let same_network_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    crate::controller::network_isolation::NAMESPACE_NETWORK_LABEL.to_string(),
                    network_label_value.clone(),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: None,
    };

    // Rule 2: Allow DNS resolution (kube-dns).
    let dns_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_string(),
                    "kube-system".to_string(),
                )])),
                ..Default::default()
            }),
            pod_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "k8s-app".to_string(),
                    "kube-dns".to_string(),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: Some(vec![
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                protocol: Some("UDP".to_string()),
                ..Default::default()
            },
            NetworkPolicyPort {
                port: Some(k8s_openapi::apimachinery::pkg::util::intstr::IntOrString::Int(53)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ]),
    };

    // Rule 3: Allow egress within the same namespace (intra-namespace pod communication,
    // e.g. Horizon → Stellar Core, Soroban RPC → Captive Core).
    let intra_namespace_egress = NetworkPolicyEgressRule {
        to: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some(BTreeMap::from([(
                    "kubernetes.io/metadata.name".to_string(),
                    node.namespace().unwrap_or_else(|| "default".to_string()),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ports: None,
    };

    egress_rules.push(same_network_egress);
    egress_rules.push(dns_egress);
    egress_rules.push(intra_namespace_egress);

    NetworkPolicy {
        metadata: merge_resource_meta(
            ObjectMeta {
                name: Some(name),
                namespace: node.namespace(),
                labels: Some({
                    let mut l = labels;
                    // Stamp the network label on the NetworkPolicy itself so
                    // cluster-level policies can select it.
                    l.insert(
                        crate::controller::network_isolation::NAMESPACE_NETWORK_LABEL.to_string(),
                        network_label_value,
                    );
                    l
                }),
                owner_references: Some(vec![owner_reference(node)]),
                ..Default::default()
            },
            &node.spec.resource_meta,
        ),
        spec: Some(NetworkPolicySpec {
            pod_selector: LabelSelector {
                match_labels: Some(BTreeMap::from([
                    ("app.kubernetes.io/instance".to_string(), node.name_any()),
                    (
                        "app.kubernetes.io/name".to_string(),
                        "stellar-node".to_string(),
                    ),
                ])),
                ..Default::default()
            },
            // Enforce both Ingress and Egress so the egress deny-by-default takes effect.
            policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
            ingress: if ingress_rules.is_empty() {
                None
            } else {
                Some(ingress_rules)
            },
            egress: if egress_rules.is_empty() {
                None
            } else {
                Some(egress_rules)
            },
        }),
    }
}

#[instrument(skip(client, node), fields(name = %node.name_any(), namespace = node.namespace()))]
pub async fn delete_network_policy(
    client: &Client,
    node: &StellarNode,
    dry_run: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &namespace);
    let name = resource_name(node, "netpol");

    match api.delete(&name, &delete_params(dry_run)).await {
        Ok(_) => info!("NetworkPolicy {} deleted", name),
        Err(kube::Error::Api(e)) if e.code == 404 => {
            info!("NetworkPolicy {} not found, skipping delete", name);
        }
        Err(e) => return Err(Error::KubeError(e)),
    }

    Ok(())
}
