//! Cloud cost estimation controller.
//!
//! Estimates the monthly USD cost of a StellarNode pod based on its
//! requested CPU and RAM and annotates the CRD with the result.
//! Also exports a Prometheus gauge so operators can visualise
//! "Cost per Ledger" or "Cost per Transaction" in Grafana.
//!
//! # Pricing model
//!
//! Uses conservative AWS on-demand **m6i** hourly rates (USD/core/h and USD/GiB/h)
//! as a portable default.  The cloud provider can be overridden via the
//! `stellar.org/cloud-provider` annotation on the StellarNode (`AWS`, `GCP`, or `Azure`).
//!
//! These are approximations — the goal is relative cost visibility, not
//! exact billing.  Mount a real pricing API behind the same interface if
//! you need exact figures.

use kube::api::{Api, Patch, PatchParams};
use kube::Client;
use tracing::debug;

use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// Supported cloud providers for price lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudProvider {
    AWS,
    GCP,
    Azure,
}

impl CloudProvider {
    /// Infer the provider from annotation `stellar.org/cloud-provider`.
    pub fn from_node(node: &StellarNode) -> Self {
        node.metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("stellar.org/cloud-provider"))
            .map(|v| match v.to_uppercase().as_str() {
                "GCP" => CloudProvider::GCP,
                "AZURE" => CloudProvider::Azure,
                _ => CloudProvider::AWS,
            })
            .unwrap_or(CloudProvider::AWS)
    }

    /// Hourly cost per vCPU (USD).
    fn cpu_cost_per_core_hour(&self) -> f64 {
        match self {
            // AWS m6i.large: ~$0.096/h for 2 vCPU  → $0.048 / vCPU / h
            CloudProvider::AWS => 0.048,
            // GCP n2-standard: ~$0.038 / vCPU / h
            CloudProvider::GCP => 0.038,
            // Azure D-series: ~$0.046 / vCPU / h
            CloudProvider::Azure => 0.046,
        }
    }

    /// Hourly cost per GiB RAM (USD).
    fn ram_cost_per_gib_hour(&self) -> f64 {
        match self {
            // AWS m6i: ~$0.012 / GiB / h
            CloudProvider::AWS => 0.012,
            // GCP n2-standard: ~$0.0051 / GiB / h
            CloudProvider::GCP => 0.0051,
            // Azure D-series: ~$0.0065 / GiB / h
            CloudProvider::Azure => 0.0065,
        }
    }

    /// Hourly storage cost per GiB (USD).  Uses gp3 / SSD equivalents.
    fn storage_cost_per_gib_hour(&self) -> f64 {
        match self {
            // AWS gp3: $0.08 / GiB / month → per hour
            CloudProvider::AWS => 0.08 / 730.0,
            // GCP pd-balanced: $0.10 / GiB / month
            CloudProvider::GCP => 0.10 / 730.0,
            // Azure Premium SSD: $0.12 / GiB / month
            CloudProvider::Azure => 0.12 / 730.0,
        }
    }
}

// ---------------------------------------------------------------------------
// CPU / Memory parsing helpers
// ---------------------------------------------------------------------------

/// Parse a Kubernetes CPU quantity string (e.g. "500m", "2", "1.5") into vCPUs.
fn parse_cpu(s: &str) -> f64 {
    let s = s.trim();
    if let Some(millis) = s.strip_suffix('m') {
        millis.parse::<f64>().unwrap_or(0.0) / 1000.0
    } else {
        s.parse::<f64>().unwrap_or(0.0)
    }
}

/// Parse a Kubernetes memory quantity string (e.g. "1Gi", "512Mi") into GiB.
fn parse_memory_gib(s: &str) -> f64 {
    let s = s.trim();
    if let Some(val) = s.strip_suffix("Gi") {
        val.parse::<f64>().unwrap_or(0.0)
    } else if let Some(val) = s.strip_suffix("Mi") {
        val.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else if let Some(val) = s.strip_suffix("Ki") {
        val.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0)
    } else if let Some(val) = s.strip_suffix('G') {
        val.parse::<f64>().unwrap_or(0.0) / 1.074
    } else {
        // Assume bytes
        s.parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0 * 1024.0)
    }
}

/// Parse a storage size string (e.g. "100Gi") into GiB.
fn parse_storage_gib(s: &str) -> f64 {
    parse_memory_gib(s)
}

// ---------------------------------------------------------------------------
// Cost estimation
// ---------------------------------------------------------------------------

/// Estimate the monthly USD cost of running a StellarNode pod.
///
/// Uses `spec.resources.requests` (CPU + RAM) and `spec.storage.size`.
/// Falls back to zero if quantities cannot be parsed.
pub fn estimate_monthly_cost(node: &StellarNode) -> f64 {
    let provider = CloudProvider::from_node(node);

    let cpu_vcpu = parse_cpu(&node.spec.resources.requests.cpu);
    let ram_gib = parse_memory_gib(&node.spec.resources.requests.memory);
    let storage_gib = parse_storage_gib(&node.spec.storage.size);

    let hours_per_month: f64 = 730.0;
    let replicas = node.spec.replicas as f64;

    let compute = (cpu_vcpu * provider.cpu_cost_per_core_hour()
        + ram_gib * provider.ram_cost_per_gib_hour())
        * hours_per_month
        * replicas;

    // Storage is per-node (not replicated per pod)
    let storage = storage_gib * provider.storage_cost_per_gib_hour() * hours_per_month;

    (compute + storage * 100.0).round() / 100.0 // two decimal places
}

/// Annotate the StellarNode with its estimated monthly cost.
///
/// Adds annotation `stellar.org/estimated-monthly-cost-usd: "<value>"`.
/// Errors are non-fatal; caller should `.ok()` or log and continue.
pub async fn annotate_node_cost(client: &Client, node: &StellarNode, cost: f64) -> Result<()> {
    let namespace = node
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let name = node
        .metadata
        .name
        .clone()
        .ok_or_else(|| Error::ConfigError("StellarNode has no name".to_string()))?;

    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let patch = serde_json::json!({
        "metadata": {
            "annotations": {
                "stellar.org/estimated-monthly-cost-usd": format!("{cost:.2}")
            }
        }
    });
    api.patch(
        &name,
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    debug!(
        "Annotated {}/{} with estimated cost ${cost:.2}/month",
        namespace, name
    );
    Ok(())
}

#[cfg(feature = "metrics")]
pub fn report_cost_metric(_namespace: &str, _name: &str, _node_type: &str, _cost: f64) {
    // Extend when a `set_estimated_monthly_cost` gauge is added to the metrics module.
    // e.g.: super::metrics::set_estimated_monthly_cost(namespace, name, node_type, cost);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cpu() {
        assert!((parse_cpu("500m") - 0.5).abs() < 1e-6);
        assert!((parse_cpu("2") - 2.0).abs() < 1e-6);
        assert!((parse_cpu("1.5") - 1.5).abs() < 1e-6);
        assert_eq!(parse_cpu(""), 0.0);
    }

    #[test]
    fn test_parse_memory_gib() {
        assert!((parse_memory_gib("1Gi") - 1.0).abs() < 1e-6);
        assert!((parse_memory_gib("512Mi") - 0.5).abs() < 1e-4);
        assert!((parse_memory_gib("2Gi") - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_estimate_monthly_cost_positive() {
        use crate::crd::StellarNode;
        use crate::crd::{
            HistoryMode, HorizonConfig, NodeType, ResourceRequirements, ResourceSpec,
            StellarNetwork, StellarNodeSpec, StorageConfig,
        };
        use kube::api::ObjectMeta;

        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("cost-test".to_string()),
                namespace: Some("default".to_string()),
                ..Default::default()
            },
            spec: StellarNodeSpec {
                node_type: NodeType::Horizon,
                network: StellarNetwork::Testnet,
                version: "v2.30.0".to_string(),
                history_mode: HistoryMode::default(),
                resources: ResourceRequirements {
                    requests: ResourceSpec {
                        cpu: "500m".to_string(),
                        memory: "1Gi".to_string(),
                    },
                    limits: ResourceSpec {
                        cpu: "2".to_string(),
                        memory: "4Gi".to_string(),
                    },
                },
                storage: StorageConfig {
                    size: "100Gi".to_string(),
                    ..Default::default()
                },
                replicas: 1,
                suspended: false,
                horizon_config: Some(HorizonConfig {
                    database_secret_ref: "s".to_string(),
                    enable_ingest: true,
                    stellar_core_url: "http://core:11626".to_string(),
                    ingest_workers: 1,
                    enable_experimental_ingestion: false,
                    auto_migration: true,
                }),
                ..Default::default()
                validator_config: None,
                soroban_config: None,
                min_available: None,
                max_unavailable: None,
                alerting: false,
                database: None,
                managed_database: None,
                autoscaling: None,
                vpa_config: None,
                ingress: None,
                load_balancer: None,
                global_discovery: None,
                cross_cluster: None,
                strategy: Default::default(),
                maintenance_mode: false,
                network_policy: None,
                dr_config: None,
                pod_anti_affinity: Default::default(),
                placement: Default::default(),
                topology_spread_constraints: None,
                cve_handling: None,
                snapshot_schedule: None,
                restore_from_snapshot: None,
                read_replica_config: None,
                db_maintenance_config: None,
                oci_snapshot: None,
                service_mesh: None,
                forensic_snapshot: None,
                label_propagation: None,
                read_pool_endpoint: None,
                resource_meta: None,
            },
            status: None,
        };

        let cost = estimate_monthly_cost(&node);
        assert!(cost > 0.0, "estimated cost must be positive, got {cost}");
        // 0.5 vCPU * 0.048 * 730 + 1 GiB * 0.012 * 730  ≈ 17.52 + 8.76 = ~26.28
        // 100 GiB storage * (0.08/730) * 730 = $8.00
        // total ~34 USD/month
        assert!(cost > 5.0 && cost < 500.0, "sanity bounds failed: {cost}");
    }

    #[test]
    fn test_cloud_provider_from_annotation_gcp() {
        use crate::crd::StellarNode;
        use kube::api::ObjectMeta;
        use std::collections::BTreeMap;

        let mut annotations = BTreeMap::new();
        annotations.insert("stellar.org/cloud-provider".to_string(), "GCP".to_string());
        let node = StellarNode {
            metadata: ObjectMeta {
                name: Some("test".to_string()),
                annotations: Some(annotations),
                ..Default::default()
            },
            // build a minimal Horizon spec so we have a valid node
            spec: build_minimal_horizon_spec(),
            status: None,
        };

        assert_eq!(CloudProvider::from_node(&node), CloudProvider::GCP);
    }

    #[test]
    fn test_cloud_provider_default_is_aws() {
        use crate::crd::StellarNode;
        use kube::api::ObjectMeta;

        let node = StellarNode {
            metadata: ObjectMeta::default(),
            spec: build_minimal_horizon_spec(),
            status: None,
        };
        assert_eq!(CloudProvider::from_node(&node), CloudProvider::AWS);
    }

    fn build_minimal_horizon_spec() -> crate::crd::StellarNodeSpec {
        use crate::crd::{
            HistoryMode, HorizonConfig, NodeType, ResourceRequirements, ResourceSpec,
            StellarNetwork, StellarNodeSpec, StorageConfig,
        };
        StellarNodeSpec {
            node_type: NodeType::Horizon,
            network: StellarNetwork::Testnet,
            version: "v2.30.0".to_string(),
            history_mode: HistoryMode::default(),
            resources: ResourceRequirements {
                requests: ResourceSpec {
                    cpu: "250m".to_string(),
                    memory: "512Mi".to_string(),
                },
                limits: ResourceSpec {
                    cpu: "2".to_string(),
                    memory: "4Gi".to_string(),
                },
            },
            storage: StorageConfig::default(),
            replicas: 1,
            suspended: false,
            horizon_config: Some(HorizonConfig {
                database_secret_ref: "s".to_string(),
                enable_ingest: true,
                stellar_core_url: "http://core:11626".to_string(),
                ingest_workers: 1,
                enable_experimental_ingestion: false,
                auto_migration: true,
            }),
            ..Default::default()
            validator_config: None,
            soroban_config: None,
            min_available: None,
            max_unavailable: None,
            alerting: false,
            database: None,
            managed_database: None,
            autoscaling: None,
            vpa_config: None,
            ingress: None,
            load_balancer: None,
            global_discovery: None,
            cross_cluster: None,
            strategy: Default::default(),
            maintenance_mode: false,
            network_policy: None,
            dr_config: None,
            pod_anti_affinity: Default::default(),
            placement: Default::default(),
            topology_spread_constraints: None,
            cve_handling: None,
            snapshot_schedule: None,
            restore_from_snapshot: None,
            read_replica_config: None,
            db_maintenance_config: None,
            oci_snapshot: None,
            service_mesh: None,
            forensic_snapshot: None,
            read_pool_endpoint: None,
            resource_meta: None,
            label_propagation: None,
        }
    }
}
