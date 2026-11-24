#!/usr/bin/env python3
"""Derive JSON schemas from repository-owned CRD OpenAPI definitions.

Canonical source: config/crd/*-crd.yaml (and Helm-bundled CRDs that are not
duplicates of those files). Output: schemas/crd/<Kind>-<group>-<version>.json
so kubeconform can load them via:

    -schema-location 'schemas/crd/{{ .ResourceKind }}{{ .KindSuffix }}'

Do not invent schemas by hand. Re-run this script after CRD changes and commit
the regenerated files so CI can detect drift.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

try:
    import yaml
except ImportError as exc:  # pragma: no cover
    raise SystemExit("PyYAML is required: pip install pyyaml") from exc

ROOT = Path(__file__).resolve().parents[2]
CRD_DIRS = [
    ROOT / "config" / "crd",
    ROOT / "charts" / "stellar-operator" / "templates",
]
OUT_DIR = ROOT / "schemas" / "crd"


def iter_yaml_docs(path: Path):
    with path.open(encoding="utf-8") as handle:
        try:
            docs = list(yaml.safe_load_all(handle))
        except yaml.YAMLError as exc:
            print(f"warning: skipping unparseable {path}: {exc}", file=sys.stderr)
            return
    for doc in docs:
        if isinstance(doc, dict):
            yield doc


def is_crd(doc: dict) -> bool:
    return (
        doc.get("kind") == "CustomResourceDefinition"
        and str(doc.get("apiVersion", "")).startswith("apiextensions.k8s.io/")
    )


def schema_filename(kind: str, group: str, version: str) -> str:
    return f"{kind}-{group}-{version}.json"


def wrap_resource_schema(openapi: dict, kind: str, api_version: str) -> dict:
    """Turn a CRD openAPIV3Schema into a full-object JSON Schema for kubeconform."""
    properties = dict(openapi.get("properties") or {})
    required = list(openapi.get("required") or [])
    properties["apiVersion"] = {"type": "string", "enum": [api_version]}
    properties["kind"] = {"type": "string", "enum": [kind]}
    properties.setdefault(
        "metadata",
        {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "namespace": {"type": "string"},
                "labels": {"type": "object", "additionalProperties": {"type": "string"}},
                "annotations": {"type": "object", "additionalProperties": {"type": "string"}},
            },
        },
    )
    for field in ("apiVersion", "kind", "metadata"):
        if field not in required:
            required.append(field)
    return {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "description": openapi.get("description") or f"{kind} custom resource",
        "properties": properties,
        "required": required,
        "additionalProperties": openapi.get("additionalProperties", True),
        "x-kubernetes-group-version-kind": [
            {
                "group": api_version.split("/")[0],
                "version": api_version.split("/")[-1],
                "kind": kind,
            }
        ],
    }


def extract_from_crd(doc: dict, seen: dict[str, Path], origin: Path) -> list[tuple[str, dict]]:
    spec = doc.get("spec") or {}
    group = spec.get("group")
    names = spec.get("names") or {}
    kind = names.get("kind")
    versions = spec.get("versions") or []
    if not group or not kind:
        return []

    extracted = []
    for version in versions:
        if not version.get("served", True):
            continue
        name = version.get("name")
        openapi = ((version.get("schema") or {}).get("openAPIV3Schema")) or {}
        if not name or not openapi:
            continue
        filename = schema_filename(kind, group, name)
        if filename in seen and seen[filename] != origin:
            # Prefer config/crd over Helm template copies of the same CRD.
            if "config" in seen[filename].parts and "charts" in origin.parts:
                continue
        seen[filename] = origin
        api_version = f"{group}/{name}"
        extracted.append((filename, wrap_resource_schema(openapi, kind, api_version)))
    return extracted


def collect_crd_files() -> list[Path]:
    files: list[Path] = []
    config_dir = ROOT / "config" / "crd"
    if config_dir.is_dir():
        files.extend(sorted(config_dir.glob("*.yaml")))
        files.extend(sorted(config_dir.glob("*.yml")))
    helm_dir = ROOT / "charts" / "stellar-operator" / "templates"
    if helm_dir.is_dir():
        for path in sorted(helm_dir.glob("crd*.yaml")):
            files.append(path)
    return files


def generate(out_dir: Path) -> list[Path]:
    out_dir.mkdir(parents=True, exist_ok=True)
    seen: dict[str, Path] = {}
    written: list[Path] = []
    catalog = []
    for path in collect_crd_files():
        for doc in iter_yaml_docs(path):
            if not is_crd(doc):
                continue
            for filename, schema in extract_from_crd(doc, seen, path):
                dest = out_dir / filename
                dest.write_text(json.dumps(schema, indent=2, sort_keys=False) + "\n", encoding="utf-8")
                written.append(dest)
                catalog.append(
                    {
                        "file": filename,
                        "source": str(path.relative_to(ROOT)).replace("\\", "/"),
                        "kind": schema["x-kubernetes-group-version-kind"][0]["kind"],
                        "apiVersion": f"{schema['x-kubernetes-group-version-kind'][0]['group']}/{schema['x-kubernetes-group-version-kind'][0]['version']}",
                    }
                )
    (out_dir / "catalog.json").write_text(
        json.dumps({"schemas": catalog}, indent=2) + "\n", encoding="utf-8"
    )
    return written


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Fail if generated schemas differ from files already in schemas/crd/",
    )
    parser.add_argument("--out", type=Path, default=OUT_DIR)
    args = parser.parse_args()

    if args.check:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            generate(tmp_path)
            expected = {p.name: p.read_text(encoding="utf-8") for p in tmp_path.glob("*.json")}
            existing = {p.name: p.read_text(encoding="utf-8") for p in args.out.glob("*.json")}
            missing = sorted(set(expected) - set(existing))
            extra = sorted(set(existing) - set(expected))
            drifted = sorted(
                name
                for name in set(expected) & set(existing)
                if expected[name] != existing[name]
            )
            if missing or extra or drifted:
                print("CRD JSON schema drift detected.", file=sys.stderr)
                for name in missing:
                    print(f"  missing: {name}", file=sys.stderr)
                for name in extra:
                    print(f"  extra:   {name}", file=sys.stderr)
                for name in drifted:
                    print(f"  drifted: {name}", file=sys.stderr)
                print("Re-run: python3 scripts/ci/extract-crd-json-schemas.py", file=sys.stderr)
                return 1
            print(f"OK: {len(expected)} CRD JSON schemas match config/crd/")
            return 0

    written = generate(args.out)
    print(f"Wrote {len(written)} schemas to {args.out}")
    for path in written:
        print(f"  {path.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
