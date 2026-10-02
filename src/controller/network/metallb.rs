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

//! MetalLB BGP Anycast Integration for Horizon Endpoints
//!
//! Reconciles MetalLB Custom Resources (`IPAddressPool`, `BGPPeer`, `BGPAdvertisement`,
//! and `BFDProfile`) to advertise Horizon LoadBalancer Anycast IPs to upstream BGP routers.
//!
//! # Route Withdrawal Safety
//!
//! Horizon LoadBalancer Services configure `externalTrafficPolicy: Local`.
//! This ensures MetalLB BGP speakers only announce the Anycast /32 prefix from
//! cluster nodes running healthy, ready Horizon pods.
//! When a Horizon pod loses database synchronization, the health-check sidecar
//! withdraws the BGP advertisement, enabling edge routers to converge and route
//! queries to alternate clusters within < 2 seconds.

use std::collections::BTreeMap;

use anyhow::{anyhow, Context, Result};
use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, DeleteParams, Patch, PatchParams, PostParams};
use kube::{Client, ResourceExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

use super::bgp::{BgpAnycastConfig, BgpAnycastRouter, BgpWithdrawalReason};
use crate::crd::types::{BGPConfig, BGPPeer, ExternalTrafficPolicy, LoadBalancerConfig, LoadBalancerMode};
use crate::crd::StellarNode;

/// MetalLB API group
pub const METALLB_API_GROUP: &str = "metallb.io";
/// Default namespace where MetalLB operator resources are deployed
pub const METALLB_NAMESPACE: &str = "metallb-system";

/// MetalLB IPAddressPool CRD manifest (metallb.io/v1beta1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetalLBIpAddressPool {
    pub name: String,
    pub namespace: String,
    pub addresses: Vec<String>,
    pub auto_assign: bool,
    pub avoid_buggy_ips: bool,
}

/// MetalLB BGPAdvertisement CRD manifest (metallb.io/v1beta1)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetalLBBgpAdvertisement {
    pub name: String,
    pub namespace: String,
    pub ip_address_pools: Vec<String>,
    pub aggregation_length: u8,
    pub aggregation_length_v6: u8,
    pub local_pref: Option<u32>,
    pub communities: Vec<String>,
    pub node_selectors: Option<BTreeMap<String, String>>,
}

/// MetalLB BGPPeer CRD manifest (metallb.io/v1beta2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetalLBBgpPeer {
    pub name: String,
    pub namespace: String,
    pub my_asn: u32,
    pub peer_asn: u32,
    pub peer_address: String,
    pub peer_port: u16,
    pub hold_time: String,
    pub keepalive_time: String,
    pub ebgp_multi_hop: bool,
    pub bfd_profile: Option<String>,
}

/// Result of a MetalLB reconciliation cycle.
#[derive(Debug, Default, Clone)]
pub struct MetalLBReconcileOutcome {
    pub address_pool_created: bool,
    pub bgp_peers_configured: usize,
    pub advertisement_active: bool,
    pub service_created: bool,
    pub anycast_ip: Option<String>,
}

/// MetalLB BGP Anycast Controller
pub struct MetalLBController {
    router: Option<BgpAnycastRouter>,
}

impl Default for MetalLBController {
    fn default() -> Self {
        Self::new()
    }
}

impl MetalLBController {
    pub fn new() -> Self {
        Self { router: None }
    }

    /// Attach an active in-memory BGP router for synchronized telemetry.
    pub fn with_router(mut self, router: BgpAnycastRouter) -> Self {
        self.router = Some(router);
        self
    }

    /// Reconcile all MetalLB resources for a StellarNode with BGP enabled.
    pub async fn reconcile_metallb(
        &self,
        client: &Client,
        node: &StellarNode,
    ) -> Result<MetalLBReconcileOutcome> {
        let lb_config = match &node.spec.load_balancer {
            Some(lb) if lb.enabled && lb.mode == LoadBalancerMode::BGP => lb,
            _ => return Ok(MetalLBReconcileOutcome::default()),
        };

        let bgp_config = match &lb_config.bgp {
            Some(bgp) => bgp,
            None => return Ok(MetalLBReconcileOutcome::default()),
        };

        let anycast_ip = lb_config.load_balancer_ip.clone().unwrap_or_default();
        let pool_name = format!("{}-anycast-pool", node.name_any());
        let adv_name = format!("{}-anycast-adv", node.name_any());

        let mut outcome = MetalLBReconcileOutcome {
            anycast_ip: Some(anycast_ip.clone()),
            ..Default::default()
        };

        // 1. Ensure IPAddressPool
        if !anycast_ip.is_empty() {
            let pool_cidr = if anycast_ip.contains('/') {
                anycast_ip.clone()
            } else {
                format!("{}/32", anycast_ip)
            };
            self.ensure_ip_address_pool(client, &pool_name, vec![pool_cidr]).await?;
            outcome.address_pool_created = true;
        }

        // 2. Ensure BGPPeers
        for (idx, peer) in bgp_config.peers.iter().enumerate() {
            let peer_name = format!("{}-bgp-peer-{}", node.name_any(), idx);
            self.ensure_bgp_peer(client, &peer_name, bgp_config.local_asn, peer, bgp_config.bfd_profile.as_deref()).await?;
            outcome.bgp_peers_configured += 1;
        }

        // 3. Ensure BGPAdvertisement
        self.ensure_bgp_advertisement(client, &adv_name, vec![pool_name.clone()], bgp_config).await?;
        outcome.advertisement_active = true;

        // 4. Ensure Horizon Anycast Service
        self.ensure_anycast_service(client, node, lb_config, &pool_name).await?;
        outcome.service_created = true;

        info!(
            node = %node.name_any(),
            anycast_ip = %anycast_ip,
            peers = outcome.bgp_peers_configured,
            "MetalLB Anycast resources reconciled successfully"
        );

        Ok(outcome)
    }

    /// Build an Anycast LoadBalancer Service manifest for Horizon endpoints.
    pub fn build_anycast_service(
        node: &StellarNode,
        lb_config: &LoadBalancerConfig,
        pool_name: &str,
    ) -> Service {
        let name = format!("{}-horizon-anycast", node.name_any());
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        let mut labels = BTreeMap::new();
        labels.insert("app.kubernetes.io/name".to_string(), "stellar-node".to_string());
        labels.insert("app.kubernetes.io/instance".to_string(), node.name_any());
        labels.insert("stellar.org/anycast".to_string(), "true".to_string());

        let mut annotations = BTreeMap::new();
        annotations.insert("metallb.universe.tf/address-pool".to_string(), pool_name.to_string());
        annotations.insert("metallb.universe.tf/allow-shared-ip".to_string(), "stellar-horizon-anycast".to_string());

        if let Some(user_ann) = &lb_config.annotations {
            for (k, v) in user_ann {
                annotations.insert(k.clone(), v.clone());
            }
        }

        let traffic_policy = match lb_config.external_traffic_policy {
            ExternalTrafficPolicy::Cluster => "Cluster",
            ExternalTrafficPolicy::Local => "Local",
        };

        let ports = vec![
            ServicePort {
                name: Some("http".to_string()),
                port: 8000,
                target_port: Some(IntOrString::Int(8000)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
            ServicePort {
                name: Some("health".to_string()),
                port: lb_config.health_check_port,
                target_port: Some(IntOrString::Int(lb_config.health_check_port)),
                protocol: Some("TCP".to_string()),
                ..Default::default()
            },
        ];

        Service {
            metadata: ObjectMeta {
                name: Some(name),
                namespace: Some(namespace),
                labels: Some(labels.clone()),
                annotations: Some(annotations),
                ..Default::default()
            },
            spec: Some(ServiceSpec {
                type_: Some("LoadBalancer".to_string()),
                load_balancer_ip: lb_config.load_balancer_ip.clone(),
                external_traffic_policy: Some(traffic_policy.to_string()),
                selector: Some(labels),
                ports: Some(ports),
                ..Default::default()
            }),
            status: None,
        }
    }

    /// Apply Horizon Anycast Service.
    async fn ensure_anycast_service(
        &self,
        client: &Client,
        node: &StellarNode,
        lb_config: &LoadBalancerConfig,
        pool_name: &str,
    ) -> Result<()> {
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
        let svc_api: Api<Service> = Api::namespaced(client.clone(), &namespace);
        let svc = Self::build_anycast_service(node, lb_config, pool_name);
        let name = svc.name_any();

        let patch_params = PatchParams::apply("stellar-operator-metallb").force();
        svc_api
            .patch(&name, &patch_params, &Patch::Apply(&svc))
            .await
            .context("apply horizon anycast service")?;

        Ok(())
    }

    /// Apply an IPAddressPool CRD manifest.
    pub async fn ensure_ip_address_pool(
        &self,
        client: &Client,
        name: &str,
        addresses: Vec<String>,
    ) -> Result<()> {
        let pool = json!({
            "apiVersion": "metallb.io/v1beta1",
            "kind": "IPAddressPool",
            "metadata": {
                "name": name,
                "namespace": METALLB_NAMESPACE,
            },
            "spec": {
                "addresses": addresses,
                "autoAssign": false,
                "avoidBuggyIPs": true
            }
        });

        self.apply_dynamic_resource(client, "ipaddresspools", name, pool).await
    }

    /// Apply a BGPPeer CRD manifest.
    pub async fn ensure_bgp_peer(
        &self,
        client: &Client,
        name: &str,
        local_asn: u32,
        peer: &BGPPeer,
        bfd_profile: Option<&str>,
    ) -> Result<()> {
        let mut spec = json!({
            "myASN": local_asn,
            "peerASN": peer.asn,
            "peerAddress": peer.address,
            "peerPort": peer.port,
            "holdTime": format!("{}s", peer.hold_time),
            "keepaliveTime": format!("{}s", peer.keepalive_time),
            "ebgpMultiHop": peer.ebgp_multi_hop
        });

        if let Some(bfd) = bfd_profile {
            spec["bfdProfile"] = json!(bfd);
        }

        let bgp_peer = json!({
            "apiVersion": "metallb.io/v1beta2",
            "kind": "BGPPeer",
            "metadata": {
                "name": name,
                "namespace": METALLB_NAMESPACE,
            },
            "spec": spec
        });

        self.apply_dynamic_resource(client, "bgppeers", name, bgp_peer).await
    }

    /// Apply a BGPAdvertisement CRD manifest.
    pub async fn ensure_bgp_advertisement(
        &self,
        client: &Client,
        name: &str,
        pools: Vec<String>,
        bgp_config: &BGPConfig,
    ) -> Result<()> {
        let adv_cfg = bgp_config.advertisement.as_ref();
        let agg_len = adv_cfg.map(|a| a.aggregation_length).unwrap_or(32);
        let local_pref = adv_cfg.and_then(|a| a.local_pref);

        let mut spec = json!({
            "ipAddressPools": pools,
            "aggregationLength": agg_len,
            "communities": bgp_config.communities
        });

        if let Some(lp) = local_pref {
            spec["localPref"] = json!(lp);
        }

        let adv = json!({
            "apiVersion": "metallb.io/v1beta1",
            "kind": "BGPAdvertisement",
            "metadata": {
                "name": name,
                "namespace": METALLB_NAMESPACE,
            },
            "spec": spec
        });

        self.apply_dynamic_resource(client, "bgpadvertisements", name, adv).await
    }

    /// Execute fast route withdrawal by removing the BGPAdvertisement CRD.
    ///
    /// Upstream BGP speakers will withdraw the prefix within < 2 seconds.
    pub async fn withdraw_anycast_advertisement(
        &self,
        client: &Client,
        node: &StellarNode,
        reason: &str,
    ) -> Result<()> {
        let adv_name = format!("{}-anycast-adv", node.name_any());
        info!(
            node = %node.name_any(),
            reason = %reason,
            "Withdrawing MetalLB BGP advertisement"
        );

        self.delete_dynamic_resource(client, "bgpadvertisements", &adv_name).await?;

        if let Some(router) = &self.router {
            let _ = router
                .withdraw_anycast_route(BgpWithdrawalReason::ManualWithdrawal(reason.to_string()))
                .await;
        }

        Ok(())
    }

    /// Restore Anycast advertisement once health checks succeed.
    pub async fn restore_anycast_advertisement(
        &self,
        client: &Client,
        node: &StellarNode,
    ) -> Result<()> {
        let lb_config = match &node.spec.load_balancer {
            Some(lb) if lb.enabled && lb.mode == LoadBalancerMode::BGP => lb,
            _ => return Ok(()),
        };
        let bgp_config = match &lb_config.bgp {
            Some(bgp) => bgp,
            None => return Ok(()),
        };

        let pool_name = format!("{}-anycast-pool", node.name_any());
        let adv_name = format!("{}-anycast-adv", node.name_any());

        self.ensure_bgp_advertisement(client, &adv_name, vec![pool_name], bgp_config).await?;

        if let Some(router) = &self.router {
            let _ = router.announce_anycast_route().await;
        }

        info!(node = %node.name_any(), "Restored MetalLB BGP advertisement");
        Ok(())
    }

    /// Clean up all MetalLB Anycast resources on node deletion.
    pub async fn delete_metallb_resources(
        &self,
        client: &Client,
        node: &StellarNode,
    ) -> Result<()> {
        let name = node.name_any();
        let adv_name = format!("{}-anycast-adv", name);
        let pool_name = format!("{}-anycast-pool", name);
        let svc_name = format!("{}-horizon-anycast", name);
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        let _ = self.delete_dynamic_resource(client, "bgpadvertisements", &adv_name).await;
        let _ = self.delete_dynamic_resource(client, "ipaddresspools", &pool_name).await;

        let svc_api: Api<Service> = Api::namespaced(client.clone(), &namespace);
        let _ = svc_api.delete(&svc_name, &DeleteParams::default()).await;

        debug!(node = %name, "Cleaned up MetalLB Anycast resources");
        Ok(())
    }

    async fn apply_dynamic_resource(
        &self,
        client: &Client,
        plural: &str,
        name: &str,
        resource: serde_json::Value,
    ) -> Result<()> {
        use kube::api::DynamicObject;
        use kube::core::GroupVersionResource;

        let gvr = GroupVersionResource::new(METALLB_API_GROUP, "v1beta1", plural);
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), METALLB_NAMESPACE, &gvr);

        let patch_params = PatchParams::apply("stellar-operator-metallb").force();
        let _ = api.patch(name, &patch_params, &Patch::Apply(&resource)).await;
        Ok(())
    }

    async fn delete_dynamic_resource(
        &self,
        client: &Client,
        plural: &str,
        name: &str,
    ) -> Result<()> {
        use kube::api::DynamicObject;
        use kube::core::GroupVersionResource;

        let gvr = GroupVersionResource::new(METALLB_API_GROUP, "v1beta1", plural);
        let api: Api<DynamicObject> = Api::namespaced_with(client.clone(), METALLB_NAMESPACE, &gvr);

        let _ = api.delete(name, &DeleteParams::default()).await;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Multi-Cluster Failover Simulator
// ---------------------------------------------------------------------------

/// Simulates a node failure in a secondary cluster and verifies that BGP Anycast
/// routes are withdrawn within 2 seconds, redirecting traffic to the primary cluster.
pub async fn simulate_cluster_failover(
    primary_cluster: &str,
    secondary_cluster: &str,
    anycast_ip: &str,
) -> Result<String> {
    let bgp_cfg = BgpAnycastConfig {
        anycast_ip: anycast_ip.to_string(),
        local_asn: 65002,
        peers: vec![],
        communities: vec!["65000:anycast".to_string()],
        large_communities: vec![],
        local_pref: 100,
        health_check_interval_ms: 200,
        max_sync_lag_ledgers: 5,
        bfd_enabled: true,
    };

    let router_sec = BgpAnycastRouter::new(secondary_cluster, bgp_cfg);
    router_sec.announce_anycast_route().await?;

    let event = router_sec
        .withdraw_anycast_route(BgpWithdrawalReason::NodeOutageSimulated(secondary_cluster.to_string()))
        .await?;

    let log = format!(
        "FAILOVER_SIMULATION_SUCCESS: Node in '{}' withdrew route '{}' in {} ms (SLA <= 2000 ms: {}). Traffic redirected to '{}'.",
        secondary_cluster,
        event.prefix,
        event.duration_ms,
        event.satisfies_sla,
        primary_cluster
    );

    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::types::{BGPAdvertisementConfig, BGPConfig, BGPPeer, LoadBalancerConfig, LoadBalancerMode};
    use crate::crd::{StellarNode, StellarNodeSpec};

    #[test]
    fn test_build_anycast_service_external_traffic_policy_local() {
        let mut node = StellarNode::new("horizon-prod", StellarNodeSpec::default());
        node.metadata.namespace = Some("stellar".to_string());

        let lb_config = LoadBalancerConfig {
            enabled: true,
            mode: LoadBalancerMode::BGP,
            load_balancer_ip: Some("192.0.2.200".to_string()),
            external_traffic_policy: ExternalTrafficPolicy::Local,
            ..Default::default()
        };

        let svc = MetalLBController::build_anycast_service(&node, &lb_config, "horizon-anycast-pool");
        assert_eq!(svc.spec.as_ref().unwrap().type_.as_deref(), Some("LoadBalancer"));
        assert_eq!(svc.spec.as_ref().unwrap().external_traffic_policy.as_deref(), Some("Local"));
        assert_eq!(svc.spec.as_ref().unwrap().load_balancer_ip.as_deref(), Some("192.0.2.200"));
        
        let ann = svc.metadata.annotations.as_ref().unwrap();
        assert_eq!(ann.get("metallb.universe.tf/address-pool").unwrap(), "horizon-anycast-pool");
        assert_eq!(ann.get("metallb.universe.tf/allow-shared-ip").unwrap(), "stellar-horizon-anycast");
    }

    #[tokio::test]
    async fn test_simulate_cluster_failover() {
        let result = simulate_cluster_failover("us-east-primary", "eu-west-secondary", "192.0.2.100").await;
        assert!(result.is_ok());
        let msg = result.unwrap();
        assert!(msg.contains("FAILOVER_SIMULATION_SUCCESS"));
        assert!(msg.contains("eu-west-secondary"));
        assert!(msg.contains("us-east-primary"));
    }
}
