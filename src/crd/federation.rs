use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Registry of clusters in the federation
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "ClusterRegistry",
    cluster_scoped
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterRegistrySpec {
    pub clusters: Vec<FederatedCluster>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FederatedCluster {
    pub name: String,
    pub api_endpoint: String,
    pub kubeconfig_secret_ref: String,
    #[serde(default)]
    pub labels: std::collections::BTreeMap<String, String>,
}

/// Federated StellarNode that spans multiple clusters
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[kube(
    group = "stellar.org",
    version = "v1alpha1",
    kind = "FederatedStellarNode",
    namespaced
)]
#[serde(rename_all = "camelCase")]
pub struct FederatedStellarNodeSpec {
    /// Template for the StellarNode resource
    pub template: crate::crd::StellarNodeSpec,
    /// Placement policy for the federated node
    pub placement: FederatedPlacement,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FederatedPlacement {
    /// Clusters to deploy to
    pub clusters: Vec<String>,
    /// How to handle conflicts and versioning
    #[serde(default)]
    pub conflict_resolution: ConflictResolutionStrategy,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ConflictResolutionStrategy {
    #[default]
    PrimaryWins,
    LastWriteWins,
    Manual,
}
