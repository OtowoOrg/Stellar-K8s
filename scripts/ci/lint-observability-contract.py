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
"""Block resource attributes that are not in the published observability schema.

Issue #1481: CI lint so new resource-attribute keys cannot land without a
schema change (versioned JSON Schema is the only catalog).

Usage:
    python3 scripts/ci/lint-observability-contract.py
    python3 scripts/ci/lint-observability-contract.py --schema path --scan path
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCHEMA = (
    REPO_ROOT / "schemas" / "observability" / "resource-attributes.v1.json"
)

# Resource-attribute catalogs we accept as declarations.
RESOURCE_KEY_RE = re.compile(
    r"""(?:KeyValue::new\s*\(|resource_attr\s*\(|attributes\[)"""
    r"""\s*[\"']"""
    r"""((?:k8s|service|stellar\.observability|host|deployment|telemetry\.sdk)"""
    r"""\.[A-Za-z0-9._-]+)[\"']"""
)

# Span / event attributes that share a prefix but are not resource identity.
SPAN_ALLOWLIST = {
    "service.name",  # also a resource key
}

SCAN_GLOBS = (
    "src/**/*.rs",
    "tests/**/*.rs",
    "charts/stellar-operator/templates/*.yaml",
    "scripts/generate-observability-instrumentation.py",
)


def load_allowed(schema_path: Path) -> set[str]:
    data = json.loads(schema_path.read_text(encoding="utf-8"))
    props = data.get("properties") or {}
    return set(props.keys())


VERSION_KEY = "stellar.observability.contract.version"


def load_schema(schema_path: Path) -> dict:
    return json.loads(schema_path.read_text(encoding="utf-8"))


def validate_resource(resource: dict, schema: dict) -> list[str]:
    """Validate one resource-attribute set against the contract schema.

    Mirrors ``observability_contract::validate_resource_attributes`` so the CI
    lint and the runtime/collector agree. Every message names the attribute.
    """
    errors: list[str] = []
    props = schema.get("properties", {})
    for key in schema.get("required", []):
        # The collector stamps the contract version at ingest (transform/contract runs
        # before the quarantine filter), exactly like the runtime validator's normalization.
        if key == VERSION_KEY:
            continue
        if key not in resource:
            errors.append(f"missing required attribute '{key}'")
    for key, value in resource.items():
        spec = props.get(key)
        if spec is None:
            errors.append(
                f"unknown attribute '{key}' (not in contract v{schema.get('contractVersion')})"
            )
            continue
        if spec.get("type") == "string" and not isinstance(value, str):
            errors.append(f"attribute '{key}': expected string, got {type(value).__name__}")
            continue
        if not isinstance(value, str):
            continue
        if len(value) < spec.get("minLength", 0):
            errors.append(f"attribute '{key}': shorter than minLength {spec['minLength']}")
        if "maxLength" in spec and len(value) > spec["maxLength"]:
            errors.append(f"attribute '{key}': longer than maxLength {spec['maxLength']}")
        if "pattern" in spec and not re.search(spec["pattern"], value):
            errors.append(f"attribute '{key}': value {value!r} does not match {spec['pattern']}")
        if "enum" in spec and value not in spec["enum"]:
            errors.append(f"attribute '{key}': value {value!r} not one of {spec['enum']}")
    return errors


def lint_fixtures(schema_path: Path, fixtures_dir: Path) -> list[str]:
    """Check every fixture is judged as it declares (`expect`: valid|invalid)."""
    schema = load_schema(schema_path)
    violations: list[str] = []
    files = sorted(fixtures_dir.glob("*.json")) if fixtures_dir.is_dir() else []
    if not files:
        return [f"{fixtures_dir}: no observability fixtures found"]
    for path in files:
        doc = json.loads(path.read_text(encoding="utf-8"))
        errors = validate_resource(doc.get("resource", {}), schema)
        expect = doc.get("expect")
        if expect == "valid" and errors:
            violations.append(f"{path}: expected valid but: {'; '.join(errors)}")
        elif expect == "invalid":
            reason = doc.get("reason", "")
            if not errors:
                violations.append(f"{path}: expected invalid but the contract accepts it")
            elif reason and not any(reason in e for e in errors):
                violations.append(f"{path}: expected an error mentioning {reason!r}, got {errors}")
        elif expect not in ("valid", "invalid"):
            violations.append(f"{path}: 'expect' must be 'valid' or 'invalid'")
    return violations


NIL_KEY_RE = re.compile(r'resource\.attributes\["([^"]+)"\]\s*==\s*nil')
SET_VERSION_RE = re.compile(
    r'set\(attributes\["stellar\.observability\.contract\.version"\],\s*"([^"]+)"\)'
)


def lint_collector(schema_path: Path, template: Path) -> list[str]:
    """The collector's quarantine rule must demand exactly the schema's required keys."""
    schema = load_schema(schema_path)
    if not template.is_file():
        return []
    text = template.read_text(encoding="utf-8")
    required = set(schema.get("required", []))
    out: list[str] = []
    seen = [set(NIL_KEY_RE.findall(line)) for line in text.splitlines() if "== nil" in line and "resource.attributes" in line]
    if not seen:
        out.append(f"{template}: no quarantine expression found for required attributes")
    for keys in seen:
        if keys != required:
            out.append(
                f"{template}: collector requires {sorted(keys)} but schema requires {sorted(required)} "
                f"(missing {sorted(required - keys)}, extra {sorted(keys - required)})"
            )
            break
    versions = set(SET_VERSION_RE.findall(text))
    if versions and versions != {schema.get("contractVersion")}:
        out.append(
            f"{template}: collector stamps contract version {sorted(versions)} but schema is {schema.get('contractVersion')}"
        )
    return out


def scan_file(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8", errors="replace")
    return RESOURCE_KEY_RE.findall(text)


def lint(schema_path: Path, roots: list[Path]) -> list[str]:
    allowed = load_allowed(schema_path)
    violations: list[str] = []
    files: list[Path] = []
    for root in roots:
        if root.is_file():
            files.append(root)
            continue
        for glob in SCAN_GLOBS:
            files.extend(root.glob(glob))
            # pathlib.glob('**') can miss a single-segment path on some versions
            files.extend(root.rglob("*.rs"))
            files.extend(root.rglob("*.yaml"))
            files.extend(root.rglob("*.py"))
    seen: set[Path] = set()
    for path in files:
        resolved = path.resolve()
        if resolved in seen:
            continue
        seen.add(resolved)
        if "resource-attributes.v1.json" in path.name:
            continue
        if any(part in {"target", ".git", "node_modules", "vendor"} for part in path.parts):
            continue
        if path.name == "test_lint_observability_contract.py":
            continue
        for key in scan_file(path):
            if key in allowed or key in SPAN_ALLOWLIST:
                continue
            violations.append(f"{path}: unknown resource attribute '{key}'")
    return violations


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument(
        "--scan",
        type=Path,
        action="append",
        default=None,
        help="Root to scan (repeatable). Defaults to the repository root.",
    )
    parser.add_argument(
        "--fixtures",
        type=Path,
        default=REPO_ROOT / "tests" / "fixtures" / "observability",
        help="Directory of resource-attribute fixtures (expect: valid|invalid).",
    )
    parser.add_argument(
        "--collector",
        type=Path,
        default=REPO_ROOT / "charts" / "stellar-operator" / "templates" / "otel-collector.yaml",
        help="Collector config template checked for drift against the schema.",
    )
    args = parser.parse_args(argv)
    roots = args.scan or [REPO_ROOT]
    if not args.schema.is_file():
        print(f"ERROR: schema not found: {args.schema}", file=sys.stderr)
        return 2
    violations = lint(args.schema, roots)
    violations += lint_fixtures(args.schema, args.fixtures)
    violations += lint_collector(args.schema, args.collector)
    if violations:
        print("Observability contract lint failed:", file=sys.stderr)
        for row in violations:
            print(f"  {row}", file=sys.stderr)
        print(
            "Add the key to schemas/observability/resource-attributes.v1.json "
            "(and bump contractVersion if needed).",
            file=sys.stderr,
        )
        return 1
    print(
        f"Observability contract lint passed "
        f"({len(load_allowed(args.schema))} catalog keys)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
