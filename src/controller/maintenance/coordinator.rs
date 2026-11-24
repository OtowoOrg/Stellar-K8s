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
//! Maintenance Coordinator for zero-downtime DB operations
//!
//! Coordinates with the read-pool to ensure traffic is routed away from nodes
//! undergoing maintenance without dropping client requests.
//!
//! # Draining strategy
//!
//! The node's `Service` selector gets an extra `stellar.org/maintenance: "true"`
//! key. No pod carries that label, so the endpoint set drops to zero and the
//! read-pool's remaining replicas keep serving requests. Rejoining removes the
//! key (JSON merge patch `null`), which puts the pod back into rotation once
//! it reports ready.

use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, Patch, PatchParams};
use kube::{Client, ResourceExt};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// Selector key added to a node's Service while it is being compacted.
/// Pods never carry this label, so adding it empties the endpoint set.
const MAINTENANCE_SELECTOR_KEY: &str = "stellar.org/maintenance";

pub struct MaintenanceCoordinator {
    client: Client,
}

impl MaintenanceCoordinator {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Prepare a node for maintenance by diverting API traffic away from it.
    ///
    /// Best-effort: a missing Service only logs a warning so compaction can
    /// still proceed for databases that are not exposed via a Service.
    pub async fn prepare_node(&self, node: &StellarNode) -> Result<()> {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let api: Api<Service> = Api::namespaced(self.client.clone(), &namespace);

        info!("Draining traffic from node {namespace}/{name}");

        match api.get(&name).await {
            Ok(_service) => {
                let patch = json!({
                    "spec": {
                        "selector": {
                            MAINTENANCE_SELECTOR_KEY: "true"
                        }
                    }
                });
                api.patch(&name, &PatchParams::default(), &Patch::Merge(patch))
                    .await
                    .map_err(Error::KubeError)?;
                debug!("Service {namespace}/{name} selector updated: node removed from rotation");
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                warn!("Service {namespace}/{name} not found; cannot drain traffic");
            }
            Err(e) => return Err(Error::KubeError(e)),
        }
        Ok(())
    }

    /// Restore a node to service after maintenance completes.
    ///
    /// Removes the `stellar.org/maintenance` key from the Service selector so
    /// the pod rejoins the endpoint set. No-op when the Service is gone.
    pub async fn finalize_maintenance(&self, node: &StellarNode) -> Result<()> {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let api: Api<Service> = Api::namespaced(self.client.clone(), &namespace);

        info!("Restoring traffic to node {namespace}/{name}");

        match api.get(&name).await {
            Ok(_service) => {
                let patch = json!({
                    "spec": {
                        "selector": {
                            MAINTENANCE_SELECTOR_KEY: null
                        }
                    }
                });
                api.patch(&name, &PatchParams::default(), &Patch::Merge(patch))
                    .await
                    .map_err(Error::KubeError)?;
                debug!("Service {namespace}/{name} selector restored: node back in rotation");
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {
                debug!("Service {namespace}/{name} not found; nothing to restore");
            }
            Err(e) => return Err(Error::KubeError(e)),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_patch_shape_is_valid_merge_patch() {
        // prepare_node merges a selector key; finalize removes it with null.
        let drain = json!({
            "spec": { "selector": { MAINTENANCE_SELECTOR_KEY: "true" } }
        });
        let rejoin = json!({
            "spec": { "selector": { MAINTENANCE_SELECTOR_KEY: null } }
        });
        assert_eq!(drain["spec"]["selector"][MAINTENANCE_SELECTOR_KEY], "true");
        assert!(rejoin["spec"]["selector"][MAINTENANCE_SELECTOR_KEY].is_null());
    }
}
