#!/usr/bin/env python3
"""Split reconciler.rs and resources.rs into focused submodules."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CTRL = ROOT / "src" / "controller"


def read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8").splitlines(keepends=True)


def write_module(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def split_reconciler() -> None:
    src = CTRL / "reconciler.rs"
    lines = read_lines(src)
    d = CTRL / "reconciler"
    d.mkdir(exist_ok=True)

    prelude = "".join(lines[21:87])  # shared imports

    mod_content = (
        "".join(lines[0:21])
        + "\nmod apply;\nmod cleanup;\nmod controller;\nmod error_policy;\nmod events;\n"
        + '#[cfg(feature = "reconciler-fuzz")]\nmod fuzz;\n'
        + "mod prelude;\nmod reconcile;\nmod state;\nmod support;\n\n"
        + "".join(lines[92:129])  # traits
        + "".join(lines[130:176])  # macros
        + "".join(lines[177:273])  # BatchSummaryReport
        + "\npub use controller::run_controller;\n"
        + "pub use state::ControllerState;\n"
        + '#[cfg(feature = "reconciler-fuzz")]\n'
        + "pub use fuzz::reconcile_for_fuzz;\n"
        + "pub(crate) use apply::apply_stellar_node;\n"
        + "pub(crate) use cleanup::cleanup_stellar_node;\n"
        + "pub(crate) use error_policy::error_policy;\n"
        + "pub(crate) use reconcile::reconcile;\n"
    )

    prelude_content = "//! Shared imports for reconciler submodules.\n\n" + prelude

    child = lambda doc, body: doc + "use super::prelude::*;\n\n" + "".join(body)

    write_module(d / "mod.rs", mod_content)
    write_module(d / "prelude.rs", prelude_content)
    write_module(d / "state.rs", child("//! Controller shared state.\n\n", lines[274:343]))
    write_module(d / "controller.rs", child("//! Controller loop entry point.\n\n", lines[344:603]))
    write_module(d / "events.rs", child("//! Event recording helpers.\n\n", lines[604:789]))
    write_module(d / "reconcile.rs", child("//! Reconcile dispatch.\n\n", lines[790:895]))
    write_module(d / "apply.rs", child("//! Apply reconciliation path.\n\n", lines[896:2794]))
    write_module(d / "cleanup.rs", child("//! Cleanup on deletion.\n\n", lines[2795:2976]))
    write_module(d / "support.rs", child("//! Status and helper functions.\n\n", lines[2977:4167]))
    write_module(
        d / "fuzz.rs",
        child("//! Fuzzing entry point.\n\n", lines[4168:4176]),
    )
    write_module(d / "error_policy.rs", child("//! Error retry policy.\n\n", lines[4178:]))

    src.unlink()
    print("reconciler split complete")


def split_resources() -> None:
    src = CTRL / "resources.rs"
    lines = read_lines(src)
    d = CTRL / "resources"
    d.mkdir(exist_ok=True)

    prelude = "".join(lines[5:54])  # imports after module doc

    sections = [
        ("helpers.rs", 55, 341, "//! Shared labels and naming helpers.\n\n"),
        ("pvc.rs", 341, 412, "//! PersistentVolumeClaim management.\n\n"),
        ("pdb.rs", 412, 532, "//! PodDisruptionBudget management.\n\n"),
        ("config_map.rs", 532, 744, "//! ConfigMap management.\n\n"),
        ("deployment.rs", 744, 874, "//! Deployment management.\n\n"),
        ("statefulset.rs", 874, 1003, "//! StatefulSet management.\n\n"),
        ("service.rs", 1003, 1185, "//! Service management.\n\n"),
        ("load_balancer.rs", 1185, 1234, "//! MetalLB load balancer stubs.\n\n"),
        ("cnpg.rs", 1234, 1486, "//! CloudNativePG resources.\n\n"),
        ("ingress.rs", 1486, 1887, "//! Ingress management.\n\n"),
        ("pod_template.rs", 1887, 3548, "//! Pod template builders.\n\n"),
        ("hpa.rs", 3548, 3574, "//! HPA management.\n\n"),
        ("alerting.rs", 3574, 3834, "//! Alerting ConfigMap management.\n\n"),
        ("service_monitor.rs", 3834, 3968, "//! ServiceMonitor management.\n\n"),
        ("network_policy.rs", 3968, 4482, "//! NetworkPolicy management.\n\n"),
        ("pdb_extra.rs", 4482, 4580, "//! Additional PDB builders.\n\n"),
        ("test_helpers.rs", 4580, len(lines), "//! Test builder wrappers.\n\n"),
    ]

    prelude_content = "//! Shared imports for resources submodules.\n\n" + prelude
    write_module(d / "prelude.rs", prelude_content)

    for name, start, end, doc in sections:
        extra = ""
        if name != "helpers.rs":
            extra = "use super::helpers::*;\n"
        if name in ("deployment.rs", "statefulset.rs"):
            extra += "use super::pod_template::*;\n"
        if name == "pdb_extra.rs":
            extra += "use super::pdb::build_pdb;\n"
        body = doc + "use super::prelude::*;\n" + extra + "\n" + "".join(lines[start:end])
        write_module(d / name, body)

    mod_rs = """//! Kubernetes resource builders for StellarNode.

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

pub use alerting::{delete_alerting, ensure_alerting};
pub use cnpg::{delete_cnpg_resources, ensure_cnpg_cluster, ensure_cnpg_pooler};
pub use config_map::{delete_config_map, ensure_config_map};
pub use deployment::{
    delete_canary_resources, delete_workload, ensure_canary_deployment, ensure_deployment,
};
pub use hpa::{delete_hpa, ensure_hpa};
pub use ingress::{delete_ingress, ensure_ingress};
pub use load_balancer::{
    delete_load_balancer_service, delete_metallb_config, ensure_load_balancer_service,
    ensure_metallb_config,
};
pub use network_policy::{delete_network_policy, ensure_network_policy};
pub use pdb::{delete_pdb, ensure_pdb};
pub use pvc::{delete_pvc, ensure_pvc};
pub use service::{delete_service, ensure_canary_service, ensure_service};
pub use service_monitor::{delete_service_monitor, ensure_service_monitor};
pub use statefulset::ensure_statefulset;

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
pub(crate) use pod_template::{
    build_horizon_migration_container, build_topology_spread_constraints, merge_workload_affinity,
};
"""
    write_module(d / "mod.rs", mod_rs)
    src.unlink()
    print("resources split complete")


if __name__ == "__main__":
    split_reconciler()
    split_resources()
