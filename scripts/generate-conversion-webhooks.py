#!/usr/bin/env python3
"""Generate CRD conversion webhook manifests from config/schema-migrations.json."""

import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CONFIG_PATH = REPO_ROOT / "config" / "schema-migrations.json"
OUTPUT_PATH = REPO_ROOT / "target" / "generated" / "conversion-webhooks.yaml"

HEADER = """# Copyright 2024 Stellar-K8s Contributors
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
"""


def path_exists(graph, source, target):
    if source == target:
        return True
    adjacency = {}
    for migration in graph["migrations"].values():
        adjacency.setdefault(migration["from_version"], []).append(migration["to_version"])
    queue = [source]
    visited = {source}
    while queue:
        current = queue.pop(0)
        for nxt in adjacency.get(current, []):
            if nxt == target:
                return True
            if nxt not in visited:
                visited.add(nxt)
                queue.append(nxt)
    return False


def validate(config):
    errors = []
    webhook = config.get("webhook", {})
    for key in ("name", "namespace", "path", "port"):
        if key not in webhook:
            errors.append(f"webhook.{key} is required")
    for graph in config.get("graphs", []):
        kind = graph.get("crd_kind", "<unknown>")
        versions = graph.get("supported_versions", [])
        storage = graph.get("current_storage_version")
        if storage not in versions:
            errors.append(f"{kind}: storage version {storage} is not supported")
        for migration in graph.get("migrations", {}).values():
            for field in ("from_version", "to_version"):
                if migration.get(field) not in versions:
                    errors.append(f"{kind}: {field} {migration.get(field)} is not supported")
        for version in versions:
            if not path_exists(graph, version, storage):
                errors.append(f"{kind}: no conversion path from {version} to {storage}")
            if not path_exists(graph, storage, version):
                errors.append(f"{kind}: no reverse conversion path from {storage} to {version}")
    return errors


def render(config):
    webhook = config["webhook"]
    items = []
    for graph in config["graphs"]:
        items.append(
            {
                "apiVersion": "apiextensions.k8s.io/v1",
                "kind": "CustomResourceDefinition",
                "metadata": {
                    "name": f"{graph['crd_kind'].lower()}s.stellar.org"
                },
                "spec": {
                    "conversion": {
                        "strategy": "Webhook",
                        "webhook": {
                            "clientConfig": {
                                "service": {
                                    "name": webhook["name"],
                                    "namespace": webhook["namespace"],
                                    "path": webhook["path"],
                                    "port": webhook["port"],
                                }
                            },
                            "conversionReviewVersions": ["v1", "v1beta1"],
                        }
                    }
                },
            }
        )
    manifest = {"apiVersion": "v1", "kind": "List", "items": items}
    return HEADER + json.dumps(manifest, indent=2, sort_keys=True) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=CONFIG_PATH)
    parser.add_argument("--output", type=Path, default=OUTPUT_PATH)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    config = json.loads(args.config.read_text(encoding="utf-8"))
    errors = validate(config)
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1

    rendered = render(config)
    if args.check:
        if not args.output.exists() or args.output.read_text(encoding="utf-8") != rendered:
            print("conversion webhook manifest is out of date", file=sys.stderr)
            return 1
        print("conversion webhook manifest is current")
        return 0

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    print(f"wrote {args.output.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
