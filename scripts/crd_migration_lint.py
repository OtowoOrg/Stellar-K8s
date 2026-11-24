#!/usr/bin/env python3
"""Migration linter for backward-compatible CRD evolution (issue #1065).

Compares the CRD manifests in config/crd/ against a baseline git ref
(default: origin/main) and reports changes that would break existing
custom resources:

  * a served API version was removed
  * a schema property was removed
  * a property changed its declared type
  * a previously optional field became required

Usage:
    scripts/crd_migration_lint.py [--against REF] [--crd-dir DIR]

Exit codes: 0 = no breaking changes, 1 = breaking changes found.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

import yaml


def _schema_of(version: dict) -> dict:
    return (version.get("schema") or {}).get("openAPIV3Schema") or {}


def _walk_properties(schema: dict, prefix: str = "") -> dict:
    """Flatten an openAPIV3Schema into {dotted.path: type}."""
    out = {}
    for name, prop in (schema.get("properties") or {}).items():
        path = f"{prefix}.{name}" if prefix else name
        out[path] = prop.get("type", "object")
        if isinstance(prop, dict):
            out.update(_walk_properties(prop, path))
            items = prop.get("items")
            if isinstance(items, dict):
                out.update(_walk_properties(items, f"{path}[]"))
    return out


def _required_paths(schema: dict, prefix: str = "") -> set:
    out = set()
    for name in schema.get("required") or []:
        out.add(f"{prefix}.{name}" if prefix else name)
    for name, prop in (schema.get("properties") or {}).items():
        if isinstance(prop, dict):
            path = f"{prefix}.{name}" if prefix else name
            out.update(_required_paths(prop, path))
    return out


def compare_crds(old: dict, new: dict) -> list:
    """Return a list of human-readable breaking changes between two CRDs."""
    problems = []
    name = new.get("metadata", {}).get("name", "<unknown>")
    old_versions = {v["name"]: v for v in old.get("spec", {}).get("versions", [])}
    new_versions = {v["name"]: v for v in new.get("spec", {}).get("versions", [])}

    for ver_name, old_ver in old_versions.items():
        if old_ver.get("served") and ver_name not in new_versions:
            problems.append(f"{name}: served version '{ver_name}' was removed")
            continue
        if ver_name not in new_versions:
            continue

        old_schema = _schema_of(old_ver)
        new_schema = _schema_of(new_versions[ver_name])
        old_props = _walk_properties(old_schema)
        new_props = _walk_properties(new_schema)

        for path, old_type in old_props.items():
            if path not in new_props:
                problems.append(
                    f"{name}/{ver_name}: property '{path}' was removed"
                )
            elif new_props[path] != old_type:
                problems.append(
                    f"{name}/{ver_name}: property '{path}' changed type "
                    f"'{old_type}' -> '{new_props[path]}'"
                )

        newly_required = _required_paths(new_schema) - _required_paths(old_schema)
        for path in sorted(newly_required):
            if path in old_props:
                problems.append(
                    f"{name}/{ver_name}: existing field '{path}' became required"
                )
    return problems


def _first_crd_doc(text: str):
    """Return the first CustomResourceDefinition document in a YAML stream."""
    for doc in yaml.safe_load_all(text):
        if isinstance(doc, dict) and doc.get("kind") == "CustomResourceDefinition":
            return doc
    return None


def _load_at_ref(ref: str, rel_path: str, repo_root: Path):
    proc = subprocess.run(
        ["git", "show", f"{ref}:{rel_path}"],
        capture_output=True,
        text=True,
        cwd=repo_root,
    )
    if proc.returncode != 0:
        return None  # file did not exist at the baseline ref -> new CRD, skip
    return _first_crd_doc(proc.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--against", default="origin/main", help="baseline git ref")
    parser.add_argument("--crd-dir", default="config/crd", help="CRD manifest directory")
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    crd_dir = repo_root / args.crd_dir
    if not crd_dir.is_dir():
        print(f"error: CRD directory not found: {crd_dir}", file=sys.stderr)
        return 1

    all_problems = []
    for crd_file in sorted(crd_dir.glob("*.yaml")):
        rel = crd_file.relative_to(repo_root).as_posix()
        old = _load_at_ref(args.against, rel, repo_root)
        if old is None:
            print(f"skip (new or non-CRD file at baseline): {rel}")
            continue
        new = _first_crd_doc(crd_file.read_text())
        if new is None:
            all_problems.append(f"{rel}: CRD document was removed from the file")
            print(f"FAIL: {rel}")
            continue
        problems = compare_crds(old, new)
        all_problems.extend(problems)
        status = "FAIL" if problems else "ok"
        print(f"{status}: {rel}")

    if all_problems:
        print("\nBackward-incompatible CRD changes detected:", file=sys.stderr)
        for p in all_problems:
            print(f"  - {p}", file=sys.stderr)
        return 1
    print("\nAll CRDs are backward compatible with", args.against)
    return 0


if __name__ == "__main__":
    sys.exit(main())
