//! Finalizer handling for StellarNode cleanup
//!
//! Finalizers ensure that when a StellarNode is deleted:
//! 1. All dependent resources (Deployments, Services, ConfigMaps) are cleaned up
//! 2. Persistent Volumes/Claims are deleted based on retention policy
//! 3. External resources (cloud storage, DNS) are properly removed

use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::info;

use crate::crd::StellarNode;
use crate::error::Result;

/// Finalizer name used to protect StellarNode resources
///
/// This finalizer is added when a StellarNode is created and prevents
/// the resource from being deleted until cleanup is complete.
pub const STELLAR_NODE_FINALIZER: &str = "stellarnode.stellar.org/finalizer";

/// Add finalizer to a StellarNode if not present
///
/// Called during the Apply phase to ensure the finalizer is set.
/// The kube-rs `finalizer` helper handles this automatically, but
/// this function can be used for manual finalizer management.
#[allow(dead_code)]
pub async fn add_finalizer(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let finalizers: Vec<String> = node.finalizers().to_vec();
    if !finalizers.contains(&STELLAR_NODE_FINALIZER.to_string()) {
        let mut new_finalizers = finalizers;
        new_finalizers.push(STELLAR_NODE_FINALIZER.to_string());

        let patch = json!({
            "metadata": {
                "finalizers": new_finalizers
            }
        });
        api.patch(
            &node.name_any(),
            &PatchParams::apply("stellar-operator"),
            &Patch::Merge(&patch),
        )
        .await?;
        info!("Added finalizer to StellarNode: {}", node.name_any());
    }
    Ok(())
}

/// Remove finalizer after cleanup is complete
///
/// Called after all resources have been cleaned up. Once the finalizer
/// is removed, Kubernetes will complete the deletion of the StellarNode.
#[allow(dead_code)]
pub async fn remove_finalizer(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let finalizers: Vec<String> = node
        .finalizers()
        .iter()
        .filter(|f| f.as_str() != STELLAR_NODE_FINALIZER)
        .cloned()
        .collect();

    let patch = json!({
        "metadata": {
            "finalizers": finalizers
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await?;

    info!("Removed finalizer from StellarNode: {}", node.name_any());
    Ok(())
}

/// Check if the node is being deleted
///
/// A deletion timestamp indicates the user has requested deletion,
/// but finalizers are preventing the actual removal.
#[allow(dead_code)]
pub fn is_being_deleted(node: &StellarNode) -> bool {
    node.metadata.deletion_timestamp.is_some()
}

/// Check if the node has our finalizer
#[allow(dead_code)]
pub fn has_finalizer(node: &StellarNode) -> bool {
    node.finalizers()
        .iter()
        .any(|f| f == STELLAR_NODE_FINALIZER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finalizer_name() {
        assert_eq!(STELLAR_NODE_FINALIZER, "stellarnode.stellar.org/finalizer");
    }
}
