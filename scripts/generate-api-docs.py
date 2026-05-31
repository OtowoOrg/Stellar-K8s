#!/usr/bin/env python3
"""Generate API reference documentation from CRD OpenAPI schema."""

import argparse
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("Error: PyYAML is required. Install with: pip install pyyaml", file=sys.stderr)
    sys.exit(1)

DEFAULT_CRD = "config/crd/stellarnode-crd.yaml"
DEFAULT_OUTPUT = "docs/api-reference.md"


def parse_args():
    parser = argparse.ArgumentParser(description="Generate API docs from CRD schema")
    parser.add_argument("--crd", default=DEFAULT_CRD, help="Path to CRD YAML file")
    parser.add_argument("--output", default=DEFAULT_OUTPUT, help="Output markdown file")
    parser.add_argument("--check", action="store_true", help="Check mode: exit 1 if docs are out of date")
    return parser.parse_args()


def load_crd(path):
    with open(path) as f:
        return yaml.safe_load(f)


def get_nested(d, *keys, default=None):
    for k in keys:
        if not isinstance(d, dict):
            return default
        d = d.get(k, default)
        if d is None:
            return default
    return d


def format_type(schema):
    t = schema.get("type", "")
    fmt = schema.get("format", "")
    if fmt:
        return f"`{t}` ({fmt})"
    if t == "array":
        items = schema.get("items", {})
        item_type = items.get("type", "object")
        return f"`array` of `{item_type}`"
    return f"`{t}`" if t else "`object`"


def is_required(field_name, parent_schema):
    return field_name in parent_schema.get("required", [])


def render_field(path, name, schema, parent_schema, depth=0, lines=None):
    if lines is None:
        lines = []

    full_path = f"{path}.{name}" if path else name
    heading = "#" * min(depth + 3, 6)
    lines.append(f"\n{heading} `{full_path}`\n")
    lines.append("| | |")
    lines.append("|---|---|")
    lines.append(f"| **Path** | `{full_path}` |")
    lines.append(f"| **Type** | {format_type(schema)} |")

    description = schema.get("description", "")
    if description:
        lines.append(f"| **Description** | {description} |")

    if is_required(name, parent_schema):
        lines.append("| **Required** | *(required)* |")

    default = schema.get("default")
    if default is not None:
        lines.append(f"| **Default** | `{default}` |")

    nullable = schema.get("nullable", False)
    if nullable:
        lines.append("| **Nullable** | `true` |")

    enum_vals = schema.get("enum")
    if enum_vals:
        vals = ", ".join(f"`{v}`" for v in enum_vals)
        lines.append(f"| **Enum** | {vals} |")

    props = schema.get("properties", {})
    if props:
        for child_name, child_schema in sorted(props.items()):
            render_field(full_path, child_name, child_schema, schema, depth + 1, lines)

    return lines


def generate_docs(crd):
    lines = []

    metadata = crd.get("metadata", {})
    spec = crd.get("spec", {})
    names = spec.get("names", {})
    group = spec.get("group", "")
    scope = spec.get("scope", "")
    kind = names.get("kind", "")
    plural = names.get("plural", "")
    short_names = names.get("shortNames", [])
    crd_name = metadata.get("name", "")

    lines.append(f"# {kind} API Reference")
    lines.append("> Auto-generated from the CRD OpenAPI schema. Do not edit manually.")
    lines.append("> Re-generate with: `make generate-api-docs`")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("## Overview")
    lines.append("")
    lines.append("| | |")
    lines.append("|---|---|")
    lines.append(f"| **CRD Name** | `{crd_name}` |")
    lines.append(f"| **API Group** | `{group}` |")
    lines.append(f"| **Kind** | `{kind}` |")
    lines.append(f"| **Plural** | `{plural}` |")
    short_names_str = ", ".join(f"`{s}`" for s in short_names) if short_names else ""
    lines.append(f"| **Short Names** | {short_names_str} |")
    lines.append(f"| **Scope** | `{scope}` |")

    versions = spec.get("versions", [])
    for version in versions:
        ver_name = version.get("name", "")
        served = str(version.get("served", True)).lower()
        storage = str(version.get("storage", True)).lower()
        subresources = version.get("subresources", {})
        subresource_names = ", ".join(f"`{k}`" for k in subresources.keys()) if subresources else ""

        lines.append(f"\n## Version `{ver_name}`\n")
        lines.append("| | |")
        lines.append("|---|---|")
        lines.append(f"| **Served** | `{served}` |")
        lines.append(f"| **Storage** | `{storage}` |")
        if subresource_names:
            lines.append(f"| **Subresources** | {subresource_names} |")

        columns = version.get("additionalPrinterColumns", [])
        if columns:
            lines.append("\n### kubectl Printer Columns\n")
            lines.append("| Name | Type | JSON Path |")
            lines.append("|---|---|---|")
            for col in columns:
                col_name = col.get("name", "")
                col_type = col.get("type", "")
                json_path = col.get("jsonPath", "")
                lines.append(f"| `{col_name}` | `{col_type}` | `{json_path}` |")

        schema = get_nested(version, "schema", "openAPIV3Schema", default={})
        spec_props = get_nested(schema, "properties", "spec", "properties", default={})
        spec_required = get_nested(schema, "properties", "spec", "required", default=[])
        spec_schema = {"properties": spec_props, "required": spec_required}

        if spec_props:
            lines.append("\n## Spec Fields\n")
            lines.append("Fields marked *(required)* must be present in every `StellarNode` manifest.\n")
            for field_name, field_schema in sorted(spec_props.items()):
                render_field("spec", field_name, field_schema, spec_schema, 0, lines)

        status_props = get_nested(schema, "properties", "status", "properties", default={})
        if status_props:
            status_schema = get_nested(schema, "properties", "status", default={})
            lines.append("\n## Status Fields\n")
            for field_name, field_schema in sorted(status_props.items()):
                render_field("status", field_name, field_schema, status_schema, 0, lines)

    return "\n".join(lines) + "\n"


def main():
    args = parse_args()

    crd_path = Path(args.crd)
    output_path = Path(args.output)

    if not crd_path.exists():
        print(f"Error: CRD file not found: {crd_path}", file=sys.stderr)
        sys.exit(1)

    crd = load_crd(crd_path)
    content = generate_docs(crd)

    if args.check:
        if not output_path.exists():
            print(f"Error: {output_path} does not exist. Run 'make generate-api-docs' first.")
            sys.exit(1)
        existing = output_path.read_text()
        if existing != content:
            print(f"Error: {output_path} is out of date. Run 'make generate-api-docs' and commit.")
            sys.exit(1)
        print(f"OK: {output_path} is up to date.")
    else:
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(content)
        print(f"✓ Generated {output_path}")


if __name__ == "__main__":
    main()
