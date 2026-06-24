#!/usr/bin/env python3
"""Fix import paths in split controller submodules."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTROLLER = ROOT / "src" / "controller"

RECONCILER_SIBLINGS = [
    "archive_health", "conditions", "cve_reconciler", "dr", "dr_drill",
    "finalizers", "health", "kms_secret", "metrics", "mtls", "oci_snapshot",
    "peer_discovery", "quediation", "resources", "service_mesh", "vpa", "vsl", "quorum",
]

# Fix typo
RECONCILER_SIBLINGS = [s for s in RECONCILER_SIBLINGS if s != "quediation"]
RECONCILER_SIBLINGS.append("remediation")


def fix_reconciler_imports(content: str) -> str:
    for sib in RECONCILER_SIBLINGS:
        content = content.replace(f"use super::{sib}", f"use crate::controller::{sib}")
        content = content.replace(f"super::{sib}::", f"crate::controller::{sib}::")
    # quorum submodule inside reconciler
    content = content.replace(
        "use super::quorum::QuorumAnalyzer",
        "use crate::controller::quorum::QuorumAnalyzer",
    )
    content = content.replace(
        "use crate::controller::quorum::QuorumAnalyzer;",
        "use crate::controller::quorum::QuorumAnalyzer;",
    )
    return content


def fix_resources_imports(content: str) -> str:
    return content.replace("use super::kms_secret", "use crate::controller::kms_secret")


def make_pub_crate(content: str, fn_names: list[str]) -> str:
    for fn in fn_names:
        content = content.replace(f"async fn {fn}(", f"pub(crate) async fn {fn}(")
        content = content.replace(f"fn {fn}(", f"pub(crate) fn {fn}(")
    return content


PRIVATE_FNS = [
    "run_archive_integrity_check", "update_dr_status", "get_latest_network_ledger",
    "perform_quorum_analysis", "check_canary_health", "get_canary_ready_replicas",
    "get_current_deployment_version", "get_ready_replicas", "update_archive_health_status",
    "update_status", "update_status_with_canary", "update_status_with_health",
    "update_suspended_status", "reconcile",
]

for f in (CONTROLLER / "reconciler").glob("*.rs"):
    text = f.read_text(encoding="utf-8")
    text = fix_reconciler_imports(text)
    if f.name in ("archive.rs", "dr_status.rs", "network.rs", "quorum.rs", "replicas.rs", "status.rs", "reconcile.rs"):
        text = make_pub_crate(text, PRIVATE_FNS)
    f.write_text(text, encoding="utf-8")

for f in (CONTROLLER / "resources").glob("*.rs"):
    text = fix_resources_imports(f.read_text(encoding="utf-8"))
    f.write_text(text, encoding="utf-8")

# error_policy should be pub(crate) not re-exported as pub
mod = CONTROLLER / "reconciler" / "mod.rs"
text = mod.read_text(encoding="utf-8")
text = text.replace("pub use error_policy::error_policy;\n", "pub(crate) use error_policy::error_policy;\n")
mod.write_text(text, encoding="utf-8")

ep = CONTROLLER / "reconciler" / "error_policy.rs"
text = ep.read_text(encoding="utf-8")
text = text.replace("pub(crate) fn error_policy", "pub(crate) fn error_policy")
if "pub(crate) fn error_policy" not in text:
    text = text.replace("fn error_policy", "pub(crate) fn error_policy")
ep.write_text(text, encoding="utf-8")

print("Fixed imports")
