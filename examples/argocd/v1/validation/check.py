#!/usr/bin/env python3
"""Static checks for the ArgoCD v1 golden path.

This does not contact a cluster. It verifies the repository's declarative inputs
are internally consistent before a real ArgoCD sync is recorded.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[4]
BASE = ROOT / "examples" / "argocd" / "v1"

required = [
    BASE / "bootstrap.yaml",
    BASE / "apps" / "kustomization.yaml",
    BASE / "apps" / "platform-storage.yaml",
    BASE / "apps" / "stellar-operator.yaml",
    BASE / "apps" / "testnet-validator.yaml",
    BASE / "apps" / "soroban-rpc.yaml",
    BASE / "platform" / "storage.yaml",
    BASE / "node-chart" / "Chart.yaml",
    BASE / "node-chart" / "templates" / "stellarnode.yaml",
    BASE / "node-chart" / "values-validator.yaml",
    BASE / "node-chart" / "values-soroban-rpc.yaml",
]

missing = [str(path.relative_to(ROOT)) for path in required if not path.is_file()]
if missing:
    print("missing required files:", ", ".join(missing), file=sys.stderr)
    sys.exit(1)

text = "\n".join(path.read_text(encoding="utf-8") for path in required)
checks = {
    "ArgoCD finalizer": "resources-finalizer.argocd.argoproj.io" in text,
    "automated prune": "prune: true" in text,
    "self heal": "selfHeal: true" in text,
    "foreground pruning": "PrunePropagationPolicy=foreground" in text,
    "server side apply": "ServerSideApply=true" in text,
    "namespace creation": "CreateNamespace=true" in text,
    "version label": "stellar.org/gitops-version: v1" in text,
    "validator profile": "nodeType: Validator" in (BASE / "node-chart" / "values-validator.yaml").read_text(encoding="utf-8"),
    "soroban profile": "nodeType: SorobanRpc" in (BASE / "node-chart" / "values-soroban-rpc.yaml").read_text(encoding="utf-8"),
    "placeholder instead of secret": "REPLACE_WITH_A_TESTNET_VALIDATOR_SECRET_KEY" in text,
}

failed = [name for name, passed in checks.items() if not passed]
if failed:
    print("failed checks:", ", ".join(failed), file=sys.stderr)
    sys.exit(1)

applications = sorted((BASE / "apps").glob("*.yaml"))
applications = [path for path in applications if path.name != "kustomization.yaml"]
for path in applications:
    content = path.read_text(encoding="utf-8")
    if "kind: Application" not in content:
        print(f"{path} is not an ArgoCD Application", file=sys.stderr)
        sys.exit(1)
    if path.name == "stellar-operator.yaml":
        expected_source = "path: charts/stellar-operator"
    elif path.name == "platform-storage.yaml":
        expected_source = "path: examples/argocd/v1/platform"
    else:
        expected_source = "path: examples/argocd/v1/node-chart"
    if expected_source not in content:
        print(f"{path} does not target {expected_source}", file=sys.stderr)
        sys.exit(1)

print(f"validated {len(required)} golden-path files and {len(applications)} ArgoCD Applications")
