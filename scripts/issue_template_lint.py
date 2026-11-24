#!/usr/bin/env python3
"""
scripts/issue_template_lint.py

Lints GitHub issue templates and verifies metadata consistency across `.github/ISSUE_TEMPLATE/`.
Used in CI and repository health checks.

It also validates the first-class issue type registry in `.github/issue-types.yml`
(issue #1836): every issue form must be registered exactly once, and the
registry must agree with the form it points at.

Usage:
    python3 scripts/issue_template_lint.py [--check]
    python3 scripts/issue_template_lint.py --registry-only
"""

import os
import sys
import glob
import argparse

try:
    import yaml
except ImportError:
    import json
    # Simple fallback parser for basic YAML validation if PyYAML is missing
    yaml = None

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
TEMPLATE_DIR = os.path.join(REPO_ROOT, ".github", "ISSUE_TEMPLATE")
REGISTRY_PATH = os.path.join(REPO_ROOT, ".github", "issue-types.yml")

REQUIRED_FORM_FIELDS = {"name", "description", "body"}
ALLOWED_INPUT_TYPES = {"markdown", "input", "textarea", "checkboxes", "dropdown"}
REQUIRED_TYPE_FIELDS = {"id", "name", "description", "template"}


def lint_yaml_file(filepath):
    """Parses a YAML file and returns (is_valid, content_dict, error_msg)."""
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            content_str = f.read()
            if yaml:
                data = yaml.safe_load(content_str)
            else:
                # Basic check when PyYAML is not installed
                data = {"raw": content_str}
        return True, data, ""
    except Exception as e:
        return False, None, str(e)


def validate_issue_form(filepath, data):
    """Validates GitHub Issue Form schema and metadata consistency."""
    errors = []
    rel_path = os.path.relpath(filepath, REPO_ROOT)

    if not isinstance(data, dict):
        errors.append(f"{rel_path}: Root content must be a YAML dictionary")
        return errors

    # Check required top-level keys
    missing_keys = REQUIRED_FORM_FIELDS - set(data.keys())
    if missing_keys:
        errors.append(f"{rel_path}: Missing required top-level keys: {', '.join(sorted(missing_keys))}")

    # Validate name and description
    name = data.get("name")
    if not name or not isinstance(name, str) or not name.strip():
        errors.append(f"{rel_path}: 'name' must be a non-empty string")

    desc = data.get("description")
    if not desc or not isinstance(desc, str) or not desc.strip():
        errors.append(f"{rel_path}: 'description' must be a non-empty string")

    # Validate body elements
    body = data.get("body")
    if not isinstance(body, list) or len(body) == 0:
        errors.append(f"{rel_path}: 'body' must be a non-empty list of form fields")
    else:
        for idx, item in enumerate(body):
            if not isinstance(item, dict):
                errors.append(f"{rel_path}: body[{idx}] must be a dictionary")
                continue

            field_type = item.get("type")
            if not field_type or field_type not in ALLOWED_INPUT_TYPES:
                errors.append(
                    f"{rel_path}: body[{idx}] has invalid or missing 'type' ('{field_type}'). "
                    f"Allowed types: {', '.join(sorted(ALLOWED_INPUT_TYPES))}"
                )

            attributes = item.get("attributes")
            if field_type != "markdown" and not isinstance(attributes, dict):
                errors.append(f"{rel_path}: body[{idx}] of type '{field_type}' must have an 'attributes' dict")
            elif attributes and "label" in attributes:
                if not isinstance(attributes["label"], str) or not attributes["label"].strip():
                    errors.append(f"{rel_path}: body[{idx}] label must be a non-empty string")

    return errors


def validate_config_yml(filepath, data):
    """Validates .github/ISSUE_TEMPLATE/config.yml structure."""
    errors = []
    rel_path = os.path.relpath(filepath, REPO_ROOT)

    if not isinstance(data, dict):
        errors.append(f"{rel_path}: config.yml must be a dictionary")
        return errors

    if "blank_issues_enabled" in data:
        if not isinstance(data["blank_issues_enabled"], bool):
            errors.append(f"{rel_path}: 'blank_issues_enabled' must be a boolean")

    if "contact_links" in data:
        links = data["contact_links"]
        if not isinstance(links, list):
            errors.append(f"{rel_path}: 'contact_links' must be a list")
        else:
            for idx, link in enumerate(links):
                if not isinstance(link, dict) or "name" not in link or "url" not in link:
                    errors.append(f"{rel_path}: contact_links[{idx}] must contain 'name' and 'url'")

    return errors


def validate_registry(registry_data, template_data):
    """Validates the first-class issue type registry against the issue forms.

    `registry_data` is the parsed `.github/issue-types.yml` mapping and
    `template_data` maps a template file name to its parsed form.
    """
    errors = []
    rel_registry = os.path.relpath(REGISTRY_PATH, REPO_ROOT)

    if not isinstance(registry_data, dict):
        errors.append(f"{rel_registry}: Root content must be a YAML dictionary")
        return errors

    types = registry_data.get("types")
    if not isinstance(types, list) or not types:
        errors.append(f"{rel_registry}: 'types' must be a non-empty list")
        return errors

    seen_ids = set()
    seen_titles = {}
    registered_templates = {}

    for idx, entry in enumerate(types):
        where = f"{rel_registry}: types[{idx}]"
        if not isinstance(entry, dict):
            errors.append(f"{where}: must be a YAML dictionary")
            continue

        missing = REQUIRED_TYPE_FIELDS - set(entry.keys())
        if missing:
            errors.append(f"{where}: Missing required fields: {', '.join(sorted(missing))}")

        type_id = entry.get("id")
        if not isinstance(type_id, str) or not type_id.strip():
            errors.append(f"{where}: 'id' must be a non-empty string")
        else:
            type_id = type_id.strip()
            if type_id in seen_ids:
                errors.append(f"{where}: duplicate issue type id '{type_id}'")
            seen_ids.add(type_id)

        name = entry.get("name")
        if not isinstance(name, str) or not name.strip():
            errors.append(f"{where}: 'name' must be a non-empty string")

        description = entry.get("description")
        if not isinstance(description, str) or not description.strip():
            errors.append(f"{where}: 'description' must be a non-empty string")

        template = entry.get("template")
        if not isinstance(template, str) or not template.strip():
            errors.append(f"{where}: 'template' must be a non-empty string")
            continue
        template = template.strip()

        if template in registered_templates:
            errors.append(
                f"{where}: template '{template}' is already registered by type "
                f"'{registered_templates[template]}'"
            )
        registered_templates[template] = type_id

        title = entry.get("title")
        if title is not None and (not isinstance(title, str) or not title.strip()):
            errors.append(f"{where}: 'title' must be a non-empty string when present")

        labels = entry.get("labels", [])
        if not isinstance(labels, list) or any(not isinstance(l, str) for l in labels):
            errors.append(f"{where}: 'labels' must be a list of strings")
            labels = []

        if template not in template_data:
            errors.append(
                f"{where}: template '{template}' not found in "
                f"{os.path.relpath(TEMPLATE_DIR, REPO_ROOT)}/"
            )
            continue

        form = template_data[template]

        form_labels = form.get("labels") or []
        if not isinstance(form_labels, list):
            form_labels = []
        for label in form_labels:
            if label not in labels:
                errors.append(
                    f"{where}: form '{template}' applies label '{label}' which is not "
                    f"declared in the registry"
                )
        for label in labels:
            if label not in form_labels:
                errors.append(
                    f"{where}: registry declares label '{label}' which form "
                    f"'{template}' does not apply"
                )

        form_title = form.get("title")
        if form_title and title and form_title != title:
            errors.append(
                f"{where}: registry title prefix {title!r} does not match form "
                f"'{template}' title prefix {form_title!r}"
            )
        if title and title in seen_titles:
            errors.append(
                f"{where}: title prefix {title!r} is already used by type "
                f"'{seen_titles[title]}'"
            )
        elif title:
            seen_titles[title] = type_id

    for template in sorted(template_data):
        if template == "config.yml":
            continue
        if template not in registered_templates:
            errors.append(
                f"{rel_registry}: issue form '{template}' is not registered as a "
                f"first-class issue type"
            )

    return errors


def load_registry():
    """Load and parse the first-class issue type registry."""
    rel_registry = os.path.relpath(REGISTRY_PATH, REPO_ROOT)
    if yaml is None:
        return None, [f"PyYAML is required to validate the issue type registry: pip install pyyaml"]
    if not os.path.exists(REGISTRY_PATH):
        return None, [f"Issue type registry not found: {rel_registry}"]
    try:
        with open(REGISTRY_PATH, "r", encoding="utf-8") as f:
            data = yaml.safe_load(f)
    except Exception as e:
        return None, [f"{rel_registry}: Failed to parse YAML: {e}"]
    return data, []


def collect_template_data(template_files):
    """Parse every issue form into a {file name: parsed form} mapping."""
    data = {}
    for filepath in template_files:
        filename = os.path.basename(filepath)
        if filename == "config.yml":
            continue
        is_valid, parsed, _ = lint_yaml_file(filepath)
        if is_valid and isinstance(parsed, dict):
            data[filename] = parsed
    return data


def main():
    parser = argparse.ArgumentParser(
        description="Lint GitHub issue forms and the first-class issue type registry."
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="report problems without changing anything (default behaviour)",
    )
    parser.add_argument(
        "--registry-only",
        action="store_true",
        help="validate only the first-class issue type registry",
    )
    args = parser.parse_args()

    if not os.path.exists(TEMPLATE_DIR):
        print(f"ERROR: Issue template directory does not exist: {TEMPLATE_DIR}", file=sys.stderr)
        sys.exit(1)

    template_files = sorted(glob.glob(os.path.join(TEMPLATE_DIR, "*.yml")) + glob.glob(os.path.join(TEMPLATE_DIR, "*.yaml")))

    if not template_files:
        print(f"ERROR: No issue templates found in {TEMPLATE_DIR}", file=sys.stderr)
        sys.exit(1)

    all_errors = []
    passed_count = 0

    if not args.registry_only:
        print("=== GitHub Issue Template & Metadata Lint ===")
        for filepath in template_files:
            rel_path = os.path.relpath(filepath, REPO_ROOT)
            filename = os.path.basename(filepath)

            is_valid, data, parse_err = lint_yaml_file(filepath)
            if not is_valid:
                all_errors.append(f"{rel_path}: Failed to parse YAML: {parse_err}")
                continue

            if filename == "config.yml":
                file_errors = validate_config_yml(filepath, data)
            else:
                file_errors = validate_issue_form(filepath, data)

            if file_errors:
                all_errors.extend(file_errors)
            else:
                passed_count += 1
                print(f"  ✓ {rel_path} passed metadata linting")

        print()

    print("=== First-Class Issue Type Registry Lint ===")
    registry_data, registry_errors = load_registry()
    if registry_errors:
        all_errors.extend(registry_errors)
    else:
        template_data = collect_template_data(template_files)
        registry_issues = validate_registry(registry_data, template_data)
        if registry_issues:
            all_errors.extend(registry_issues)
        else:
            passed_count += 1
            print(
                f"  ✓ {os.path.relpath(REGISTRY_PATH, REPO_ROOT)}: "
                f"{len(registry_data.get('types', []))} first-class issue type(s) validated"
            )
    print()

    if all_errors:
        print(f"FAILED: {len(all_errors)} issue template linting error(s) found:")
        for err in all_errors:
            print(f"  ✗ {err}")
        sys.exit(1)

    print(f"SUMMARY: All {passed_count} issue template check(s) passed linting and metadata consistency checks.")
    sys.exit(0)


if __name__ == "__main__":
    main()
