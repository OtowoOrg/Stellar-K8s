#!/usr/bin/env python3
"""Fix visibility and imports after controller module split."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
RECON = ROOT / "src" / "controller" / "reconciler"
RES = ROOT / "src" / "controller" / "resources"


def patch(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    if old not in text:
        return
    path.write_text(text.replace(old, new, 1), encoding="utf-8")


# events.rs - pub(crate) exports for macros
events = RECON / "events.rs"
t = events.read_text(encoding="utf-8")
for fn in ("emit_event_owned", "publish_stellar_event_owned", "apply_or_emit_owned"):
    t = t.replace(f"fn {fn}(", f"pub(crate) fn {fn}(")
events.write_text(t, encoding="utf-8")

# support.rs - pub(crate) on all top-level fns
support = RECON / "support.rs"
t = support.read_text(encoding="utf-8")
t = re.sub(r"^async fn ", "pub(crate) async fn ", t, flags=re.M)
t = re.sub(r"^fn parse_duration", "pub(crate) fn parse_duration", t, flags=re.M)
support.write_text(t, encoding="utf-8")

# reconcile.rs
patch(
    RECON / "reconcile.rs",
    "use super::prelude::*;\n\nfn reconcile(",
    "use super::prelude::*;\nuse super::apply::apply_stellar_node;\nuse super::cleanup::cleanup_stellar_node;\n\npub(crate) fn reconcile(",
)

# apply.rs + cleanup.rs
for name in ("apply.rs", "cleanup.rs", "controller.rs", "error_policy.rs", "fuzz.rs"):
    patch(
        RECON / name,
        "use super::prelude::*;\n\n",
        "use super::prelude::*;\nuse super::state::ControllerState;\nuse super::support::*;\n\n",
    )

# mod.rs export BatchSummaryReport
patch(
    RECON / "mod.rs",
    "pub use state::ControllerState;\n",
    "pub use state::ControllerState;\npub use BatchSummaryReport;\n",
)

# resources - pub(crate) on build_* in submodules and fix exports
for f in RES.glob("*.rs"):
    if f.name in ("mod.rs", "prelude.rs"):
        continue
    t = f.read_text(encoding="utf-8")
    t = re.sub(r"^pub\(crate\) fn build_", "pub(crate) fn build_", t, flags=re.M)
    t = re.sub(r"^fn build_", "pub(crate) fn build_", t, flags=re.M)
    f.write_text(t, encoding="utf-8")

# deployment.rs - export delete_workload, canary fns
dep = RES / "deployment.rs"
t = dep.read_text(encoding="utf-8")
# ensure pub on key fns
for fn in (
    "ensure_deployment", "ensure_canary_deployment", "delete_workload", "delete_canary_resources",
    "build_deployment",
):
    t = t.replace(f"pub async fn {fn}", f"pub async fn {fn}")
    t = t.replace(f"async fn {fn}", f"pub async fn {fn}")
dep.write_text(t, encoding="utf-8")

print("visibility fixes applied")
