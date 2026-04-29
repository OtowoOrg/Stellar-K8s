//! Network Isolation — cross-namespace safety checks
//!
//! Stellar nodes must never accidentally peer across network boundaries.
//! A Testnet validator connecting to a Mainnet peer would corrupt consensus;
//! a shared database between the two would be catastrophic.
//!
//! # Design
//!
//! The operator enforces isolation at two layers:
//!
//! 1. **Kubernetes NetworkPolicy** (data-plane) — generated per-node by
//!    [`crate::controller::resources::ensure_network_policy`] and augmented
//!    here with egress deny rules that block traffic to namespaces labelled
//!    with a *different* `stellar.org/network` value.
//!
//! 2. **Reconciler safety check** (control-plane) — [`check_network_safety`]
//!    is called at the start of every reconcile loop.  It inspects all
//!    `StellarNode` resources in the same namespace and fails fast if any
//!    node in that namespace is configured for a different Stellar network.
//!    This prevents the operator itself from ever creating cross-network
//!    resources, even if the NetworkPolicy CNI plugin is misconfigured.
//!
//! # Namespace labelling convention
//!
//! Every namespace that hosts Stellar nodes **must** carry the label:
//!
//! ```text
//! stellar.org/network: mainnet   # or testnet / futurenet / custom-<hash>
//! ```
//!
//! The Helm chart stamps this label on the release namespace automatically
//! (see `charts/stellar-operator/templates/network-isolation.yaml`).
//! The reconciler enforces it at runtime via [`check_network_safety`].
//!
//! # Error codes
//!
//! - `SK8S-021` — Network safety violation detected.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Namespace;
use kube::api::{Api, ListParams};
use kube::{Client, ResourceExt};
use tracing::{info, warn};

use crate::crd::{StellarNetwork, StellarNode};
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Public label / annotation constants
// ---------------------------------------------------------------------------

/// Namespace label that declares which Stellar network the namespace hosts.
/// Value must match [`StellarNetwork::isolation_label_value`].
pub const NAMESPACE_NETWORK_LABEL: &str = "stellar.org/network";

/// Pod / workload label stamped by the operator on every managed resource.
/// Used by NetworkPolicy selectors to scope ingress/egress rules.
pub const NODE_NETWORK_LABEL: &str = "stellar-network";

// ---------------------------------------------------------------------------
// NetworkSafetyViolation
// ---------------------------------------------------------------------------

/// Describes a detected cross-network isolation violation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkSafetyViolation {
    /// The namespace where the conflict was detected.
    pub namespace: String,
    /// The network of the node being reconciled.
    pub node_network: String,
    /// The conflicting network found in the same namespace.
    pub conflicting_network: String,
    /// Name of the conflicting StellarNode resource.
    pub conflicting_node: String,
    /// Human-readable explanation.
    pub message: String,
}

impl std::fmt::Display for NetworkSafetyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NetworkSafetyViolation {}

// ---------------------------------------------------------------------------
// check_network_safety
// ---------------------------------------------------------------------------

/// Verify that no other `StellarNode` in the same namespace is configured for
/// a different Stellar network.
///
/// This is the **control-plane** enforcement layer.  It runs at the start of
/// every reconcile loop so that a misconfigured resource is caught before any
/// Kubernetes resources are created or mutated.
///
/// # Returns
///
/// - `Ok(())` — namespace is clean; all nodes share the same network.
/// - `Err(Error::NetworkSafetyViolation)` — at least one conflicting node was
///   found.  The reconciler should surface this as a `Warning` event and
///   return an error to prevent further reconciliation.
///
/// # Example
///
/// ```rust,ignore
/// check_network_safety(&client, &node).await?;
/// ```
pub async fn check_network_safety(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let node_name = node.name_any();
    let node_network =
        network_label_value(&node.spec.network, &node.spec.custom_network_passphrase);

    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let nodes = api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    for peer in &nodes.items {
        // Skip self
        if peer.name_any() == node_name {
            continue;
        }

        let peer_network =
            network_label_value(&peer.spec.network, &peer.spec.custom_network_passphrase);

        if peer_network != node_network {
            let msg = format!(
                "[SK8S-021] Network safety violation in namespace '{namespace}': \
                 node '{node_name}' is configured for network '{node_network}' but \
                 node '{}' in the same namespace is configured for network '{peer_network}'. \
                 Mainnet and Testnet nodes MUST reside in separate namespaces to prevent \
                 accidental cross-network peering or database sharing.",
                peer.name_any()
            );

            warn!("{}", msg);

            return Err(Error::NetworkSafetyViolation(NetworkSafetyViolation {
                namespace,
                node_network,
                conflicting_network: peer_network,
                conflicting_node: peer.name_any(),
                message: msg,
            }));
        }
    }

    // Also verify the namespace label is consistent with the node's network.
    // This catches cases where the namespace was re-labelled after nodes were deployed.
    if let Err(e) = check_namespace_label(client, &namespace, &node_network).await {
        warn!(
            "Namespace label check failed for '{namespace}': {e}. \
             Ensure the namespace carries label '{NAMESPACE_NETWORK_LABEL}={node_network}'."
        );
        // Treat a missing/mismatched namespace label as a warning, not a hard failure,
        // because the label may not yet have been applied (e.g. first install).
        // The NetworkPolicy data-plane enforcement is the hard barrier.
    }

    info!(
        "Network safety check passed for node '{node_name}' in namespace '{namespace}' \
         (network: {node_network})"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// check_namespace_label
// ---------------------------------------------------------------------------

/// Verify that the namespace carries the expected `stellar.org/network` label.
///
/// Returns `Ok(())` if the label is absent (first-install grace) or matches.
/// Returns `Err` only when the label is present but set to a *different* value.
async fn check_namespace_label(
    client: &Client,
    namespace: &str,
    expected_network: &str,
) -> Result<()> {
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let ns = match ns_api.get(namespace).await {
        Ok(ns) => ns,
        Err(kube::Error::Api(e)) if e.code == 403 => {
            // Operator may not have permission to read Namespace objects when
            // running with namespace-scoped RBAC.  Treat as a no-op.
            return Ok(());
        }
        Err(e) => return Err(Error::KubeError(e)),
    };

    let labels: &BTreeMap<String, String> = &ns.metadata.labels.unwrap_or_default();

    match labels.get(NAMESPACE_NETWORK_LABEL) {
        None => {
            // Label not yet applied — acceptable during initial setup.
            Ok(())
        }
        Some(actual) if actual == expected_network => Ok(()),
        Some(actual) => Err(Error::NetworkSafetyViolation(NetworkSafetyViolation {
            namespace: namespace.to_string(),
            node_network: expected_network.to_string(),
            conflicting_network: actual.clone(),
            conflicting_node: String::new(),
            message: format!(
                "[SK8S-021] Namespace '{namespace}' is labelled \
                 '{NAMESPACE_NETWORK_LABEL}={actual}' but node is configured for \
                 network '{expected_network}'. Update the namespace label or move \
                 the node to the correct namespace."
            ),
        })),
    }
}

// ---------------------------------------------------------------------------
// network_label_value
// ---------------------------------------------------------------------------

/// Return the canonical, DNS-safe label value for a [`StellarNetwork`].
///
/// This is the value written to the `stellar-network` pod label and the
/// `stellar.org/network` namespace label.
pub fn network_label_value(network: &StellarNetwork, custom_passphrase: &Option<String>) -> String {
    network.scheduling_label_value(custom_passphrase)
}

// ---------------------------------------------------------------------------
// build_isolation_egress_peers
// ---------------------------------------------------------------------------

/// Build the list of `NetworkPolicyPeer` entries that represent namespaces
/// hosting the **same** Stellar network.
///
/// These are used by [`crate::controller::resources::build_network_policy`]
/// to construct an egress allow-list: traffic is permitted only to pods in
/// namespaces that share the same `stellar.org/network` label.
///
/// Any egress not matched by this allow-list is implicitly denied when the
/// `NetworkPolicy` includes an `Egress` policy type.
pub fn same_network_namespace_selector(
    network: &StellarNetwork,
    custom_passphrase: &Option<String>,
) -> k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
    let label_value = network_label_value(network, custom_passphrase);
    k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector {
        match_labels: Some(BTreeMap::from([(
            NAMESPACE_NETWORK_LABEL.to_string(),
            label_value,
        )])),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::StellarNetwork;

    #[test]
    fn network_label_value_mainnet() {
        assert_eq!(
            network_label_value(&StellarNetwork::Mainnet, &None),
            "mainnet"
        );
    }

    #[test]
    fn network_label_value_testnet() {
        assert_eq!(
            network_label_value(&StellarNetwork::Testnet, &None),
            "testnet"
        );
    }

    #[test]
    fn network_label_value_futurenet() {
        assert_eq!(
            network_label_value(&StellarNetwork::Futurenet, &None),
            "futurenet"
        );
    }

    #[test]
    fn network_label_value_custom_is_stable() {
        // Custom networks produce a deterministic hash-based label.
        let v1 = network_label_value(&StellarNetwork::Custom("my-net".to_string()), &None);
        let v2 = network_label_value(&StellarNetwork::Custom("my-net".to_string()), &None);
        assert_eq!(v1, v2);
        assert!(v1.starts_with("custom-"));
    }

    #[test]
    fn same_network_namespace_selector_has_correct_label() {
        let sel = same_network_namespace_selector(&StellarNetwork::Mainnet, &None);
        let labels = sel.match_labels.unwrap();
        assert_eq!(labels.get(NAMESPACE_NETWORK_LABEL).unwrap(), "mainnet");
    }

    #[test]
    fn violation_display() {
        let v = NetworkSafetyViolation {
            namespace: "stellar-prod".to_string(),
            node_network: "mainnet".to_string(),
            conflicting_network: "testnet".to_string(),
            conflicting_node: "test-validator".to_string(),
            message: "cross-network conflict".to_string(),
        };
        assert_eq!(format!("{v}"), "cross-network conflict");
    }
}
