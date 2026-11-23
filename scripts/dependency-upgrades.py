#!/usr/bin/env python3
# Copyright 2026 Stellar-K8s Contributors
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
"""List the direct dependencies whose locked version changed against a base ref.

Prints one `name from to` line per upgraded direct dependency, for the
dependency upgrade gate (#1999). A crate is direct when `Cargo.toml` declares it
in `[dependencies]`, `[dev-dependencies]` or `[build-dependencies]` (including
the `target.*` variants). Transitive-only changes are not gated: nothing in this
repository calls them.

Crates locked at more than one version on either side are skipped with a
warning, because the lockfile alone cannot say which copy changed.

Usage: scripts/dependency-upgrades.py <base-ref> [--lock Cargo.lock] [--manifest Cargo.toml]
"""

import argparse
import subprocess
import sys
import tomllib
from pathlib import Path

SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def locked_versions(lock_text):
    """Map crate name -> sorted list of locked versions."""
    versions = {}
    for package in tomllib.loads(lock_text).get("package", []):
        versions.setdefault(package["name"], []).append(package["version"])
    return {name: sorted(found) for name, found in versions.items()}


def direct_dependencies(manifest_text):
    """Names of the crates the manifest declares, resolving `package = "..."` renames."""
    manifest = tomllib.loads(manifest_text)
    tables = [manifest] + list(manifest.get("target", {}).values())
    names = set()
    for table in tables:
        for section in SECTIONS:
            for key, spec in table.get(section, {}).items():
                names.add(spec.get("package", key) if isinstance(spec, dict) else key)
    return names


def upgrades(base_versions, head_versions, direct):
    """(name, from, to) for each direct dependency whose single locked version changed."""
    changed, skipped = [], []
    for name in sorted(direct):
        before, after = base_versions.get(name), head_versions.get(name)
        if not before or not after or before == after:
            continue
        if len(before) != 1 or len(after) != 1:
            skipped.append(name)
            continue
        changed.append((name, before[0], after[0]))
    return changed, skipped


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("base_ref", help="git ref to compare against, e.g. origin/main")
    parser.add_argument("--lock", default="Cargo.lock")
    parser.add_argument("--manifest", default="Cargo.toml")
    args = parser.parse_args()

    base_lock = subprocess.run(
        ["git", "show", f"{args.base_ref}:{args.lock}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if base_lock.returncode != 0:
        print(f"cannot read {args.lock} at {args.base_ref}: {base_lock.stderr.strip()}", file=sys.stderr)
        return 2

    head_lock = Path(args.lock).read_text(encoding="utf-8")
    manifest = Path(args.manifest).read_text(encoding="utf-8")

    changed, skipped = upgrades(
        locked_versions(base_lock.stdout),
        locked_versions(head_lock),
        direct_dependencies(manifest),
    )
    for name in skipped:
        print(f"warning: {name} is locked at several versions; not gated", file=sys.stderr)
    for name, before, after in changed:
        print(f"{name} {before} {after}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
