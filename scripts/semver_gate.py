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
"""Semantic versioning gate for charts, images and CRDs (issue #1523).

Subcommands:

  required-bump [--base REF]
      Print the minimum bump (major | minor | none) the CRD API diff between
      REF and the working tree requires. Breaking changes (as detected by
      scripts/crd_migration_lint.py, plus removed CRDs) require `major`;
      new CRDs or served versions require `minor`.

  check --version X [--base REF]
      Block a release unless:
        * Chart.yaml `version` and `appVersion` both equal X
        * values.yaml `image.tag` is empty (defaults to appVersion) or X
        * the chart's CRD served/storage versions match config/crd
        * the bump from the chart version at REF to X is at least the
          required bump computed from the CRD diff

REF defaults to the latest `chart-v*` tag. Exit 0 = pass, 1 = blocked.
"""

from __future__ import annotations

import argparse
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parent.parent
CHART_DIR = "charts/stellar-operator"
CRD_DIR = "config/crd"
CHART_CRD = f"{CHART_DIR}/templates/crd.yaml"
SEMVER = re.compile(r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$")
LEVELS = ["none", "patch", "minor", "major"]

_spec = importlib.util.spec_from_file_location(
    "crd_migration_lint", Path(__file__).resolve().parent / "crd_migration_lint.py"
)
lint = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(lint)


def parse_semver(v: str):
    m = SEMVER.match(v or "")
    return tuple(int(x) for x in m.groups()) if m else None


def bump_level(old: str, new: str) -> str:
    """Level of the bump from `old` to `new`; `invalid` if not an increase."""
    o, n = parse_semver(old), parse_semver(new)
    if o is None or n is None or n <= o:
        return "invalid"
    if n[0] > o[0]:
        return "major"
    if n[1] > o[1]:
        return "minor"
    return "patch"


def git(*args: str, root: Path = REPO_ROOT) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], capture_output=True, text=True, cwd=root)


def latest_chart_tag(root: Path = REPO_ROOT) -> str | None:
    """Highest released chart tag (by version, not reachability: release
    bump commits are not always ancestors of the release branch)."""
    p = git("tag", "--list", "chart-v*", "--sort=-v:refname", root=root)
    tags = p.stdout.split()
    return tags[0] if tags else None


def crd_impact(old_crds: dict, new_crds: dict) -> tuple[str, list[str]]:
    """Compare {path: crd_doc} maps. Returns (required level, reasons)."""
    breaking, additive = [], []
    for path, old in old_crds.items():
        new = new_crds.get(path)
        if new is None:
            breaking.append(f"{path}: CRD was removed")
            continue
        breaking.extend(lint.compare_crds(old, new))
        old_versions = {v["name"] for v in old.get("spec", {}).get("versions", [])}
        for v in new.get("spec", {}).get("versions", []):
            if v["name"] not in old_versions and v.get("served"):
                additive.append(f"{path}: new served version '{v['name']}'")
    for path in new_crds.keys() - old_crds.keys():
        additive.append(f"{path}: new CRD")
    if breaking:
        return "major", breaking
    if additive:
        return "minor", additive
    return "none", []


def load_crds_at(ref: str | None, root: Path = REPO_ROOT) -> dict:
    """CRD docs under config/crd at `ref` (None = working tree)."""
    out = {}
    if ref is None:
        for f in sorted((root / CRD_DIR).glob("*.yaml")):
            doc = lint._first_crd_doc(f.read_text())
            if doc:
                out[f.relative_to(root).as_posix()] = doc
        return out
    listing = git("ls-tree", "--name-only", f"{ref}:{CRD_DIR}", root=root)
    for name in listing.stdout.split():
        if name.endswith(".yaml"):
            rel = f"{CRD_DIR}/{name}"
            doc = lint._load_at_ref(ref, rel, root)
            if doc:
                out[rel] = doc
    return out


def _versions(crd: dict) -> list[tuple[str, bool, bool]]:
    return sorted(
        (v["name"], bool(v.get("served")), bool(v.get("storage")))
        for v in crd.get("spec", {}).get("versions", [])
    )


def _chart_crd_versions(text: str) -> dict:
    """{crd name: versions} from the chart CRD template (Helm directives stripped)."""
    cleaned = "\n".join(
        line for line in text.splitlines() if not re.search(r"\{\{-?\s", line)
    )
    out = {}
    for doc in yaml.safe_load_all(cleaned):
        if isinstance(doc, dict) and doc.get("kind") == "CustomResourceDefinition":
            out[doc["metadata"]["name"]] = _versions(doc)
    return out


def consistency_errors(version: str, root: Path = REPO_ROOT) -> list[str]:
    errors = []
    if parse_semver(version) is None:
        return [f"release version '{version}' is not valid semver (MAJOR.MINOR.PATCH)"]
    chart = yaml.safe_load((root / CHART_DIR / "Chart.yaml").read_text())
    for key in ("version", "appVersion"):
        if str(chart.get(key)) != version:
            errors.append(
                f"Chart.yaml {key} is '{chart.get(key)}', expected '{version}'. "
                f"Fix: set {key}: \"{version}\" in {CHART_DIR}/Chart.yaml."
            )
    values = yaml.safe_load((root / CHART_DIR / "values.yaml").read_text())
    tag = str((values.get("image") or {}).get("tag") or "")
    if tag not in ("", version):
        errors.append(
            f"values.yaml image.tag is '{tag}', expected '' or '{version}'. "
            "Fix: leave image.tag empty so it defaults to appVersion."
        )
    chart_crd = root / CHART_CRD
    if chart_crd.exists():
        config = {d["metadata"]["name"]: _versions(d) for d in load_crds_at(None, root).values()}
        for name, versions in _chart_crd_versions(chart_crd.read_text()).items():
            if name in config and config[name] != versions:
                errors.append(
                    f"{CHART_CRD} serves {versions} for {name} but {CRD_DIR} has "
                    f"{config[name]}. Fix: regenerate the chart CRD from {CRD_DIR}."
                )
    return errors


def cmd_required_bump(args) -> int:
    root = Path(args.root)
    base = args.base or latest_chart_tag(root)
    if base is None:
        print("none")
        return 0
    level, reasons = crd_impact(load_crds_at(base, root), load_crds_at(None, root))
    for r in reasons:
        print(f"  {r}", file=sys.stderr)
    print(level)
    return 0


def cmd_check(args) -> int:
    root = Path(args.root)
    errors = consistency_errors(args.version, root)
    base = args.base or latest_chart_tag(root)
    if base is not None:
        base_chart = git("show", f"{base}:{CHART_DIR}/Chart.yaml", root=root)
        base_version = (
            str(yaml.safe_load(base_chart.stdout).get("version"))
            if base_chart.returncode == 0
            else None
        )
        required, reasons = crd_impact(load_crds_at(base, root), load_crds_at(None, root))
        actual = bump_level(base_version, args.version) if base_version else "major"
        if actual == "invalid":
            errors.append(
                f"version {args.version} is not greater than {base_version} released at {base}."
            )
        elif LEVELS.index(actual) < LEVELS.index(required):
            errors.append(
                f"CRD API changes since {base} require a {required} bump, but "
                f"{base_version} -> {args.version} is a {actual} bump:\n"
                + "\n".join(f"      - {r}" for r in reasons)
                + (
                    "\n    Fix: release as a major version (commit with `feat!:` or a "
                    "`BREAKING CHANGE:` footer, or run the Helm release with "
                    "bump_override=major), or restore compatibility (keep the old "
                    "served version / field)."
                    if required == "major"
                    else "\n    Fix: release as at least a minor version."
                )
            )
    if errors:
        print(f"✗ Release {args.version} blocked by semver gate:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    print(f"✓ Release {args.version} passes the semver gate")
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=str(REPO_ROOT), help=argparse.SUPPRESS)
    sub = parser.add_subparsers(dest="cmd", required=True)
    rb = sub.add_parser("required-bump")
    rb.add_argument("--base")
    rb.set_defaults(func=cmd_required_bump)
    ck = sub.add_parser("check")
    ck.add_argument("--version", required=True)
    ck.add_argument("--base")
    ck.set_defaults(func=cmd_check)
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
