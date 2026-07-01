#!/usr/bin/env python3
"""Rewrite reconciler submodule imports from super:: to crate::controller::."""

from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
RECON = ROOT / "src" / "controller" / "reconciler"

RECON_INTERNAL = {
    "apply",
    "cleanup",
    "controller",
    "error_policy",
    "events",
    "fuzz",
    "prelude",
    "reconcile",
    "state",
    "support",
}


def fix_super_imports(text: str) -> str:
    def repl(match: re.Match[str]) -> str:
        mod = match.group(1)
        if mod in RECON_INTERNAL:
            return match.group(0)
        return f"crate::controller::{mod}"

    return re.sub(r"\bsuper::(\w+)", repl, text)


for path in RECON.glob("*.rs"):
    original = path.read_text(encoding="utf-8")
    updated = fix_super_imports(original)
    if updated != original:
        path.write_text(updated, encoding="utf-8")
        print(f"patched {path.name}")

# resources prelude
prelude = ROOT / "src" / "controller" / "resources" / "prelude.rs"
text = prelude.read_text(encoding="utf-8")
text = text.replace("use super::kms_secret", "use crate::controller::kms_secret")
text = text.replace(
    "use super::label_propagation::LabelPropagator",
    "use crate::controller::label_propagation::LabelPropagator",
)
prelude.write_text(text, encoding="utf-8")
print("patched resources/prelude.rs")
