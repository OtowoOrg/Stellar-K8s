#!/usr/bin/env python3
"""Repository-wide schema validation for YAML manifests (issue #1044).

Before this gate, YAML validation in this repository was partial and
advisory: ``scripts/ci/validate-config-samples.sh`` looked at ``examples/``
and ``config/samples/`` only, delegated to ``kubeconform`` with
``-ignore-missing-schemas`` (so every ``stellar.org`` custom resource was
skipped outright), and downgraded every finding to a warning.

This checker covers *every* YAML file in the repository and validates in
four layers:

  L1 syntax      every document parses, with duplicate mapping keys and
                 literal tabs treated as errors (PyYAML accepts both by
                 default, and both have caused real manifest bugs)
  L2 structure   Kubernetes documents carry a well-formed apiVersion/kind,
                 a DNS-1123 metadata.name, and valid label/annotation keys
  L3 schema      custom resources are validated against this repository's
                 own CRDs in config/crd/, and any path may be bound to a
                 JSON Schema in the config file
  L4 fixtures    manifests declared as negative fixtures *must* fail L3, so
                 a schema regression that silently starts accepting bad
                 input is caught too

Usage:
    scripts/validate-yaml-manifests.py [PATH ...]
    scripts/validate-yaml-manifests.py --format json
    scripts/validate-yaml-manifests.py --summary

Exit codes: 0 = all layers passed, 1 = validation failed, 2 = bad invocation.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Sequence

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("PyYAML is required: pip install -r requirements.txt")

try:
    import jsonschema
except ImportError:  # pragma: no cover
    jsonschema = None

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / "config" / "yaml-validation.yaml"
CRD_DIR = REPO_ROOT / "config" / "crd"

ERROR = "error"
WARNING = "warning"

# Kubernetes naming rules (RFC 1123 label / subdomain).
DNS_1123_SUBDOMAIN = re.compile(r"^[a-z0-9]([-a-z0-9]*[a-z0-9])?(\.[a-z0-9]([-a-z0-9]*[a-z0-9])?)*$")
LABEL_KEY_RE = re.compile(r"^(?:(?P<prefix>[^/]+)/)?(?P<name>[A-Za-z0-9]([-A-Za-z0-9_.]*[A-Za-z0-9])?)$")
LABEL_VALUE_RE = re.compile(r"^(?:[A-Za-z0-9]([-A-Za-z0-9_.]*[A-Za-z0-9])?)?$")
API_VERSION_RE = re.compile(r"^(?:[a-z0-9.-]+/)?v[0-9]+(?:(?:alpha|beta)[0-9]+)?$")


# ---------------------------------------------------------------------------
# Findings
# ---------------------------------------------------------------------------


@dataclass
class Issue:
    """One validation problem, located as precisely as the layer allows."""

    path: str
    layer: str
    message: str
    severity: str = ERROR
    line: int | None = None
    doc_index: int | None = None

    def location(self) -> str:
        loc = self.path
        if self.line is not None:
            loc += f":{self.line}"
        if self.doc_index is not None:
            loc += f" (document {self.doc_index + 1})"
        return loc

    def as_dict(self) -> dict:
        return {
            "path": self.path,
            "line": self.line,
            "document": self.doc_index,
            "layer": self.layer,
            "severity": self.severity,
            "message": self.message,
        }


# ---------------------------------------------------------------------------
# L1 — strict YAML loading
# ---------------------------------------------------------------------------


class StrictLoader(yaml.SafeLoader):
    """SafeLoader that rejects duplicate mapping keys.

    Plain PyYAML silently keeps the last value, which is how a manifest ends
    up with two `image:` keys and quietly deploys the wrong one.
    """


def _no_duplicate_keys(loader: StrictLoader, node: yaml.MappingNode, deep: bool = False) -> dict:
    mapping = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in mapping:
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                f"found duplicate key {key!r}",
                key_node.start_mark,
            )
        mapping[key] = loader.construct_object(value_node, deep=deep)
    return mapping


StrictLoader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _no_duplicate_keys)


def load_documents(path: Path) -> tuple[list[Any], list[Issue]]:
    """Parse every document in a YAML file under strict rules."""
    rel = _relative(path)
    text = path.read_text(encoding="utf-8", errors="replace")
    issues: list[Issue] = []

    for number, line in enumerate(text.splitlines(), start=1):
        indent = line[: len(line) - len(line.lstrip())]
        if "\t" in indent:
            issues.append(
                Issue(rel, "L1-syntax", "literal tab used for indentation (YAML forbids tabs)", line=number)
            )

    try:
        documents = list(yaml.load_all(text, Loader=StrictLoader))
    except yaml.YAMLError as exc:
        line = None
        mark = getattr(exc, "problem_mark", None)
        if mark is not None:
            line = mark.line + 1
        detail = getattr(exc, "problem", None) or str(exc).splitlines()[0]
        issues.append(Issue(rel, "L1-syntax", f"YAML parse error: {detail}", line=line))
        return [], issues

    return documents, issues


# ---------------------------------------------------------------------------
# L2 — Kubernetes structural rules
# ---------------------------------------------------------------------------


def is_kubernetes_document(doc: Any) -> bool:
    return isinstance(doc, dict) and "apiVersion" in doc and "kind" in doc


def validate_structure(rel: str, doc: dict, index: int) -> list[Issue]:
    """Check the invariants every Kubernetes object must satisfy."""
    issues: list[Issue] = []

    def add(message: str, severity: str = ERROR) -> None:
        issues.append(Issue(rel, "L2-structure", message, severity=severity, doc_index=index))

    api_version = doc.get("apiVersion")
    if not isinstance(api_version, str) or not API_VERSION_RE.match(api_version):
        add(f"apiVersion {api_version!r} is not of the form 'group/version' or 'version'")

    kind = doc.get("kind")
    if not isinstance(kind, str) or not kind[:1].isupper():
        add(f"kind {kind!r} must be a non-empty PascalCase string")

    metadata = doc.get("metadata")
    if metadata is None:
        # A List has no metadata of its own; everything else needs one.
        if kind not in ("List",):
            add("metadata is missing")
        return issues
    if not isinstance(metadata, dict):
        add("metadata must be a mapping")
        return issues

    name = metadata.get("name")
    if name is None and "generateName" not in metadata:
        add("metadata.name (or metadata.generateName) is required")
    elif name is not None:
        if not isinstance(name, str) or not name:
            add(f"metadata.name {name!r} must be a non-empty string")
        elif len(name) > 253:
            add(f"metadata.name is {len(name)} characters (limit is 253)")
        elif not DNS_1123_SUBDOMAIN.match(name):
            add(f"metadata.name {name!r} is not a valid DNS-1123 subdomain")

    namespace = metadata.get("namespace")
    if namespace is not None:
        if not isinstance(namespace, str) or not DNS_1123_SUBDOMAIN.match(namespace) or len(namespace) > 63:
            add(f"metadata.namespace {namespace!r} is not a valid DNS-1123 label")

    for field_name in ("labels", "annotations"):
        entries = metadata.get(field_name)
        if entries is None:
            continue
        if not isinstance(entries, dict):
            add(f"metadata.{field_name} must be a mapping")
            continue
        for key, value in entries.items():
            issues.extend(_check_metadata_entry(rel, index, field_name, key, value))

    return issues


def _check_metadata_entry(rel: str, index: int, field_name: str, key: Any, value: Any) -> list[Issue]:
    issues: list[Issue] = []

    def add(message: str) -> None:
        issues.append(Issue(rel, "L2-structure", message, doc_index=index))

    if not isinstance(key, str) or not LABEL_KEY_RE.match(key):
        add(f"metadata.{field_name} key {key!r} is not a valid qualified name")
        return issues

    match = LABEL_KEY_RE.match(key)
    prefix = match.group("prefix")
    if prefix and (len(prefix) > 253 or not DNS_1123_SUBDOMAIN.match(prefix)):
        add(f"metadata.{field_name} key prefix {prefix!r} is not a valid DNS-1123 subdomain")
    if len(match.group("name")) > 63:
        add(f"metadata.{field_name} key name {match.group('name')!r} exceeds 63 characters")

    # Label values are constrained; annotation values are free-form strings.
    if field_name == "labels":
        if not isinstance(value, str):
            add(f"metadata.labels[{key!r}] must be a string, got {type(value).__name__}")
        elif len(value) > 63 or not LABEL_VALUE_RE.match(value):
            add(f"metadata.labels[{key!r}] value {value!r} is not a valid label value")
    elif value is not None and not isinstance(value, str):
        add(f"metadata.annotations[{key!r}] must be a string, got {type(value).__name__}")

    return issues


# ---------------------------------------------------------------------------
# L3 — schema validation
# ---------------------------------------------------------------------------

_X_KUBERNETES_DROP = (
    "x-kubernetes-list-type",
    "x-kubernetes-list-map-keys",
    "x-kubernetes-map-type",
    "x-kubernetes-validations",
    "x-kubernetes-embedded-resource",
)


def openapi_to_jsonschema(schema: Any) -> Any:
    """Convert a CRD's openAPIV3Schema into something jsonschema can run.

    Kubernetes' dialect is OpenAPI v3 plus `x-kubernetes-*` extensions. The
    extensions have to be translated (int-or-string) or dropped, otherwise a
    Draft-7 validator either errors out or silently ignores them.
    """
    if isinstance(schema, list):
        return [openapi_to_jsonschema(item) for item in schema]
    if not isinstance(schema, dict):
        return schema

    out: dict[str, Any] = {}
    for key, value in schema.items():
        if key in _X_KUBERNETES_DROP:
            continue
        if key == "x-kubernetes-int-or-string":
            if value:
                out["type"] = ["integer", "string"]
            continue
        if key == "x-kubernetes-preserve-unknown-fields":
            # Anything goes below this point; don't constrain it.
            if value:
                out["additionalProperties"] = True
            continue
        if key == "nullable":
            continue
        if key in ("properties", "patternProperties", "definitions"):
            out[key] = {k: openapi_to_jsonschema(v) for k, v in (value or {}).items()}
            continue
        if key in ("items", "additionalProperties", "not"):
            out[key] = openapi_to_jsonschema(value)
            continue
        if key in ("allOf", "anyOf", "oneOf"):
            out[key] = [openapi_to_jsonschema(v) for v in value]
            continue
        out[key] = value

    # OpenAPI's `nullable: true` means "or null" in JSON Schema terms.
    if schema.get("nullable") and "type" in out:
        types = out["type"]
        out["type"] = ([types] if isinstance(types, str) else list(types)) + ["null"]

    return out


@dataclass
class CrdSchema:
    """A single served CRD version, ready to validate custom resources."""

    group: str
    kind: str
    version: str
    schema: dict
    source: str

    @property
    def api_version(self) -> str:
        return f"{self.group}/{self.version}"


def load_crd_schemas(crd_dir: Path) -> tuple[dict[tuple[str, str], CrdSchema], list[Issue]]:
    """Index every served CRD version by (apiVersion, kind)."""
    index: dict[tuple[str, str], CrdSchema] = {}
    issues: list[Issue] = []
    if not crd_dir.is_dir():
        return index, issues

    for path in sorted(crd_dir.glob("*.yaml")):
        documents, parse_issues = load_documents(path)
        issues.extend(parse_issues)
        for doc in documents:
            if not isinstance(doc, dict) or doc.get("kind") != "CustomResourceDefinition":
                continue
            spec = doc.get("spec") or {}
            group = spec.get("group")
            kind = ((spec.get("names") or {}).get("kind"))
            if not group or not kind:
                issues.append(Issue(_relative(path), "L3-schema", "CRD is missing spec.group or spec.names.kind"))
                continue
            for version in spec.get("versions") or []:
                if not version.get("served", True):
                    continue
                raw = ((version.get("schema") or {}).get("openAPIV3Schema")) or {}
                index[(f"{group}/{version['name']}", kind)] = CrdSchema(
                    group=group,
                    kind=kind,
                    version=version["name"],
                    schema=openapi_to_jsonschema(raw),
                    source=_relative(path),
                )
    return index, issues


def _validator_for(schema: dict):
    if jsonschema is None:  # pragma: no cover
        raise RuntimeError("jsonschema is required for schema validation")
    cls = jsonschema.validators.validator_for(schema, default=jsonschema.Draft7Validator)
    return cls(schema)


def validate_against_schema(rel: str, doc: Any, schema: dict, index: int, layer: str) -> list[Issue]:
    """Run one document through a JSON Schema and format the failures."""
    issues: list[Issue] = []
    validator = _validator_for(schema)
    for error in sorted(validator.iter_errors(doc), key=lambda e: list(e.absolute_path)):
        pointer = ".".join(str(p) for p in error.absolute_path) or "<root>"
        issues.append(Issue(rel, layer, f"{pointer}: {error.message}", doc_index=index))
    return issues


# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------


@dataclass
class SchemaBinding:
    glob: str
    schema: str


@dataclass
class Deviation:
    """A recorded, pre-existing failure that the gate tolerates for now."""

    path: str
    layer: str
    contains: str
    reason: str
    matched: bool = False


@dataclass
class Config:
    exclude: list[str] = field(default_factory=list)
    schemas: list[SchemaBinding] = field(default_factory=list)
    expect_invalid: list[str] = field(default_factory=list)
    deviations: list[Deviation] = field(default_factory=list)

    @classmethod
    def load(cls, path: Path) -> "Config":
        if not path.is_file():
            return cls()
        data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        return cls(
            exclude=[str(p) for p in (data.get("exclude") or [])],
            schemas=[SchemaBinding(glob=b["glob"], schema=b["schema"]) for b in (data.get("schemas") or [])],
            expect_invalid=[str(p) for p in (data.get("expect_invalid") or [])],
            deviations=[
                Deviation(
                    path=d["path"],
                    layer=d.get("layer", ""),
                    contains=d.get("contains", ""),
                    reason=d["reason"],
                )
                for d in (data.get("known_deviations") or [])
            ],
        )

    def is_excluded(self, rel: str) -> bool:
        return any(fnmatch.fnmatch(rel, pattern) for pattern in self.exclude)

    def is_negative_fixture(self, rel: str) -> bool:
        return any(fnmatch.fnmatch(rel, pattern) for pattern in self.expect_invalid)

    def schema_for(self, rel: str) -> Path | None:
        for binding in self.schemas:
            if fnmatch.fnmatch(rel, binding.glob):
                return REPO_ROOT / binding.schema
        return None


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def _relative(path: Path) -> str:
    try:
        return str(path.resolve().relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def discover(targets: Sequence[str], config: Config) -> list[Path]:
    """Collect every YAML file under ``targets`` that the config includes."""
    found: list[Path] = []
    for target in targets:
        base = Path(target)
        if not base.is_absolute():
            base = REPO_ROOT / base
        if base.is_file():
            candidates: Iterable[Path] = [base]
        elif base.is_dir():
            candidates = (p for p in base.rglob("*") if p.is_file())
        else:
            raise SystemExit(f"no such path: {target}")
        for path in candidates:
            if path.suffix not in (".yaml", ".yml"):
                continue
            if {".git", "target", "node_modules"} & set(path.parts):
                continue
            if config.is_excluded(_relative(path)):
                continue
            found.append(path)
    return sorted(set(found))


@dataclass
class Report:
    issues: list[Issue] = field(default_factory=list)
    files: int = 0
    documents: int = 0
    kubernetes_documents: int = 0
    schema_validated: int = 0
    negative_fixtures: int = 0

    @property
    def errors(self) -> list[Issue]:
        return [i for i in self.issues if i.severity == ERROR]

    @property
    def warnings(self) -> list[Issue]:
        return [i for i in self.issues if i.severity == WARNING]


def validate(paths: Sequence[Path], config: Config) -> Report:
    """Run all four validation layers over ``paths``."""
    report = Report(files=len(paths))
    crd_index, crd_issues = load_crd_schemas(CRD_DIR)
    report.issues.extend(crd_issues)

    for path in paths:
        rel = _relative(path)
        documents, parse_issues = load_documents(path)
        report.issues.extend(parse_issues)
        if parse_issues and not documents:
            continue

        negative = config.is_negative_fixture(rel)
        fixture_issues: list[Issue] = []
        bound_schema = config.schema_for(rel)

        for index, doc in enumerate(documents):
            if doc is None:
                continue
            report.documents += 1

            if bound_schema is not None:
                schema = json.loads(bound_schema.read_text(encoding="utf-8"))
                found = validate_against_schema(rel, doc, schema, index, "L3-schema")
                report.schema_validated += 1
                (fixture_issues if negative else report.issues).extend(found)
                continue

            if not is_kubernetes_document(doc):
                continue
            report.kubernetes_documents += 1

            structural = validate_structure(rel, doc, index)
            (fixture_issues if negative else report.issues).extend(structural)

            crd = crd_index.get((doc.get("apiVersion"), doc.get("kind")))
            if crd is not None:
                found = validate_against_schema(rel, doc, crd.schema, index, "L3-schema")
                report.schema_validated += 1
                (fixture_issues if negative else report.issues).extend(found)

        if negative:
            report.negative_fixtures += 1
            if not fixture_issues:
                report.issues.append(
                    Issue(
                        rel,
                        "L4-fixture",
                        "declared as a negative fixture but validated cleanly — "
                        "the schema it was meant to exercise has regressed",
                    )
                )

    _apply_deviations(report, config)
    report.issues.sort(key=lambda i: (i.path, i.doc_index or 0, i.line or 0, i.message))
    return report


def _apply_deviations(report: Report, config: Config) -> None:
    """Downgrade recorded pre-existing failures, and flag stale waivers."""
    kept: list[Issue] = []
    for issue in report.issues:
        waiver = next(
            (
                d
                for d in config.deviations
                if fnmatch.fnmatch(issue.path, d.path)
                and (not d.layer or d.layer == issue.layer)
                and (not d.contains or d.contains in issue.message)
            ),
            None,
        )
        if waiver is None:
            kept.append(issue)
            continue
        waiver.matched = True
        issue.severity = WARNING
        issue.message = f"{issue.message}  [known deviation: {waiver.reason}]"
        kept.append(issue)

    for waiver in config.deviations:
        if not waiver.matched:
            kept.append(
                Issue(
                    waiver.path,
                    "L0-config",
                    f"known_deviations entry no longer matches anything and should be removed "
                    f"(reason was: {waiver.reason})",
                )
            )
    report.issues = kept


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def render_text(report: Report, summary_only: bool) -> str:
    annotate = os.environ.get("GITHUB_ACTIONS") == "true"
    lines: list[str] = []
    lines.append("→ Repository-wide YAML manifest validation")
    lines.append(
        f"  files: {report.files}   documents: {report.documents}   "
        f"kubernetes: {report.kubernetes_documents}   schema-checked: {report.schema_validated}   "
        f"negative fixtures: {report.negative_fixtures}"
    )
    lines.append("")

    if not summary_only:
        for issue in report.issues:
            marker = "✗" if issue.severity == ERROR else "⚠"
            lines.append(f"  {marker} [{issue.layer}] {issue.location()}")
            lines.append(f"      {issue.message}")
            if annotate:
                level = "error" if issue.severity == ERROR else "warning"
                line_arg = f",line={issue.line}" if issue.line else ""
                lines.append(f"::{level} file={issue.path}{line_arg},title={issue.layer}::{issue.message}")
        if report.issues:
            lines.append("")

    lines.append("━" * 60)
    lines.append(f"YAML Validation Summary:  errors: {len(report.errors)}   warnings: {len(report.warnings)}")
    lines.append("━" * 60)
    lines.append("")
    lines.append("❌ YAML manifest validation FAILED" if report.errors else "✅ All YAML manifests valid")
    return "\n".join(lines)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Repository-wide YAML manifest schema validation")
    parser.add_argument("paths", nargs="*", help="files or directories to validate (default: repository root)")
    parser.add_argument("--config", default=str(DEFAULT_CONFIG))
    parser.add_argument("--format", choices=("text", "json"), default="text")
    parser.add_argument("--summary", action="store_true", help="print only the summary line")
    parser.add_argument("--strict", action="store_true", help="treat warnings as failures")
    args = parser.parse_args(argv)

    if jsonschema is None:
        sys.exit("jsonschema is required: pip install -r requirements.txt")

    config = Config.load(Path(args.config))
    paths = discover(args.paths or [str(REPO_ROOT)], config)
    report = validate(paths, config)

    if args.format == "json":
        print(
            json.dumps(
                {
                    "files": report.files,
                    "documents": report.documents,
                    "kubernetes_documents": report.kubernetes_documents,
                    "schema_validated": report.schema_validated,
                    "negative_fixtures": report.negative_fixtures,
                    "errors": len(report.errors),
                    "warnings": len(report.warnings),
                    "issues": [i.as_dict() for i in report.issues],
                },
                indent=2,
            )
        )
    else:
        print(render_text(report, args.summary))

    if report.errors:
        return 1
    return 1 if (args.strict and report.warnings) else 0


if __name__ == "__main__":
    sys.exit(main())
