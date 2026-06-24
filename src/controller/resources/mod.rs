//! Kubernetes resource builders for StellarNode.

mod alerting;
mod cnpg;
mod config_map;
mod deployment;
mod helpers;
mod hpa;
mod ingress;
mod load_balancer;
mod network_policy;
mod pdb;
mod pdb_extra;
mod pod_template;
mod prelude;
mod pvc;
mod service;
mod service_monitor;
mod statefulset;
#[cfg(test)]
mod test_helpers;

pub(crate) use helpers::{
    apply_probe_override_pub, merge_service_annotations, merge_service_metadata_labels,
    owner_reference, resource_name, standard_labels,
};

pub use alerting::ensure_alerting;
pub use cnpg::{delete_cnpg_resources, ensure_cnpg_cluster, ensure_cnpg_pooler};
pub use config_map::{delete_config_map, ensure_config_map};
pub use deployment::{ensure_canary_deployment, ensure_deployment};
pub use hpa::ensure_hpa;
pub use ingress::{delete_ingress, ensure_ingress};
pub use load_balancer::{
    delete_load_balancer_service, delete_metallb_config, delete_service, ensure_load_balancer_service,
    ensure_metallb_config,
};
pub use network_policy::{delete_network_policy, ensure_network_policy};
pub use pdb::delete_pvc;
pub use pdb_extra::{delete_pdb, ensure_pdb};
pub use pvc::ensure_pvc;
pub use service::{ensure_canary_service, ensure_service};
pub use service_monitor::{
    delete_alerting, delete_canary_resources, delete_hpa, delete_service_monitor,
    ensure_service_monitor,
};
pub use statefulset::{delete_workload, ensure_statefulset};

#[cfg(test)]
pub(crate) use test_helpers::{
    build_config_map_for_test, build_deployment_for_test, build_pdb_for_test, build_pvc_for_test,
    build_service_for_test, build_statefulset_for_test,
};
pub(crate) use helpers::build_pvc;
pub(crate) use config_map::build_config_map;
pub(crate) use deployment::build_deployment;
pub(crate) use statefulset::build_statefulset;
pub(crate) use network_policy::build_network_policy;
pub(crate) use pdb_extra::build_pdb;
pub(crate) use pod_template::build_topology_spread_constraints;
pub(crate) use pod_template::{
    build_horizon_migration_container, merge_workload_affinity,
};
