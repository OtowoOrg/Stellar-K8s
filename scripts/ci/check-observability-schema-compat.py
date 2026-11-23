#!/usr/bin/env python3
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""Fail CI on incompatible changes to the observability resource-attribute schema (#1960).

Compares the schema in the working tree against the version on a base git ref
(default ``origin/main``) or an explicit file.

Rules (see docs/observability/unified-contract.md#versioning):

* BREAKING (requires a MAJOR bump of ``contractVersion``): removing an attribute,
  adding a required attribute, changing an attribute's type, narrowing an enum,
  lowering ``maxLength``, raising ``minLength``, changing ``pattern``.
* COMPATIBLE (requires at least a MINOR bump): adding an optional attribute,
  widening an enum, relaxing limits.
* Any schema change requires ``contractVersion`` to increase, and a non-major
  bump must keep the previous version in ``compatibleWith``.

Usage:
    python3 scripts/ci/check-observability-schema-compat.py [--base-ref origin/main | --base FILE]
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SCHEMA_REL = "schemas/observability/resource-attributes.v1.json"
DEFAULT_SCHEMA = REPO_ROOT / SCHEMA_REL


def parse_version(v: str) -> tuple[int, int, int]:
    parts = v.split(".")
    if len(parts) != 3 or not all(p.isdigit() for p in parts):
        raise ValueError(f"invalid semantic version {v!r}")
    return int(parts[0]), int(parts[1]), int(parts[2])


def diff_schemas(old: dict, new: dict) -> tuple[list[str], list[str]]:
    """Return (breaking, compatible) change descriptions."""
    breaking: list[str] = []
    compatible: list[str] = []
    old_p, new_p = old.get("properties", {}), new.get("properties", {})
    for key in sorted(set(old_p) - set(new_p)):
        breaking.append(f"attribute '{key}' was removed")
    for key in sorted(set(new_p) - set(old_p)):
        if key in new.get("required", []):
            breaking.append(f"required attribute '{key}' was added")
        else:
            compatible.append(f"optional attribute '{key}' was added")
    for key in sorted(set(new.get("required", [])) - set(old.get("required", []))):
        if key in old_p:
            breaking.append(f"attribute '{key}' became required")
    for key in sorted(set(old_p) & set(new_p)):
        o, n = old_p[key], new_p[key]
        if o.get("type") != n.get("type"):
            breaking.append(f"attribute '{key}': type changed {o.get('type')} -> {n.get('type')}")
        if "enum" in o or "enum" in n:
            oe, ne = set(o.get("enum", [])), set(n.get("enum", []))
            if "enum" in o and "enum" not in n:
                compatible.append(f"attribute '{key}': enum constraint removed")
            elif "enum" not in o:
                breaking.append(f"attribute '{key}': enum constraint added")
            elif oe - ne:
                breaking.append(f"attribute '{key}': enum values removed {sorted(oe - ne)}")
            elif ne - oe:
                compatible.append(f"attribute '{key}': enum values added {sorted(ne - oe)}")
        if n.get("minLength", 0) > o.get("minLength", 0):
            breaking.append(f"attribute '{key}': minLength raised")
        elif n.get("minLength", 0) < o.get("minLength", 0):
            compatible.append(f"attribute '{key}': minLength relaxed")
        if "maxLength" in n and n["maxLength"] < o.get("maxLength", float("inf")):
            breaking.append(f"attribute '{key}': maxLength lowered")
        elif o.get("maxLength", float("inf")) < n.get("maxLength", float("inf")):
            compatible.append(f"attribute '{key}': maxLength relaxed")
        if o.get("pattern") != n.get("pattern"):
            breaking.append(f"attribute '{key}': pattern changed")
    return breaking, compatible


def check(old: dict, new: dict) -> list[str]:
    """Return policy violations for the transition old -> new."""
    errors: list[str] = []
    breaking, compatible = diff_schemas(old, new)
    ov, nv = old.get("contractVersion"), new.get("contractVersion")
    try:
        o, n = parse_version(ov), parse_version(nv)
    except (ValueError, TypeError) as e:
        return [str(e)]
    changed = bool(breaking or compatible)
    if n < o:
        errors.append(f"contractVersion went backwards: {ov} -> {nv}")
    if changed and n <= o:
        errors.append(f"schema changed but contractVersion was not increased (still {nv})")
    if breaking and n[0] == o[0]:
        errors.append(
            "incompatible changes require a MAJOR contractVersion bump "
            f"({ov} -> {nv}): " + "; ".join(breaking)
        )
    if compatible and not breaking and n[0] == o[0] and n[1] == o[1] and n > o:
        errors.append(
            f"added/relaxed attributes require at least a MINOR bump ({ov} -> {nv}): " + "; ".join(compatible)
        )
    if n[0] == o[0] and n > o and ov not in new.get("compatibleWith", []):
        errors.append(f"compatibleWith must keep previous version {ov} for a non-major bump")
    if nv not in new.get("compatibleWith", []):
        errors.append(f"compatibleWith must include the current version {nv}")
    if parse_version(new.get("minCompatibleVersion", nv)) > n:
        errors.append("minCompatibleVersion is greater than contractVersion")
    if n[0] > o[0] and new.get("minCompatibleVersion") == old.get("minCompatibleVersion") and breaking:
        errors.append("major bump must raise minCompatibleVersion (old emitters are no longer compatible)")
    return errors


def load_base(base_ref: str | None, base_file: Path | None) -> dict | None:
    if base_file:
        return json.loads(base_file.read_text(encoding="utf-8"))
    try:
        out = subprocess.run(
            ["git", "show", f"{base_ref}:{SCHEMA_REL}"],
            cwd=REPO_ROOT, check=True, capture_output=True, text=True,
        ).stdout
    except subprocess.CalledProcessError:
        return None  # schema did not exist on the base: nothing to compare
    return json.loads(out)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    ap.add_argument("--base-ref", default="origin/main")
    ap.add_argument("--base", type=Path, help="explicit base schema file (overrides --base-ref)")
    args = ap.parse_args(argv)
    new = json.loads(args.schema.read_text(encoding="utf-8"))
    old = load_base(args.base_ref, args.base)
    if old is None:
        print(f"No base schema at {args.base_ref}; skipping compatibility check.")
        return 0
    errors = check(old, new)
    if errors:
        print("Observability schema compatibility check failed:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    br, co = diff_schemas(old, new)
    print(
        f"Observability schema compatible: {old.get('contractVersion')} -> {new.get('contractVersion')} "
        f"({len(br)} breaking, {len(co)} compatible changes)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
