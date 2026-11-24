#!/usr/bin/env python3
"""sort-manifests.py — Deterministic YAML manifest sorter for Stellar-K8s pipelines.

Reads a multi-document YAML stream from stdin (or a file argument), then
emits each document in a stable, canonical order so that generated manifests
produce identical output on every run regardless of hash-map iteration order
or map key insertion order in the upstream tool (helm template, crdgen, etc.).

Sorting rules
-------------
1. Documents are ordered by (kind, metadata.namespace, metadata.name).
2. Within each document, **all mapping keys are sorted recursively** so that
   the output is byte-for-byte identical for equivalent inputs — eliminating
   spurious diffs in PRs and making git blame meaningful.
3. Null documents (empty YAML blocks separated by ---) are silently dropped.

Usage
-----
    # via Makefile targets
    make bundle-render
    make crd-gen

    # direct pipe
    helm template ... | python3 scripts/sort-manifests.py

    # file argument
    python3 scripts/sort-manifests.py rendered/manifests.yaml

The script requires only the Python standard library (pyyaml is bundled with
most CI runners; install with `pip install pyyaml` if missing).
"""

from __future__ import annotations

import sys
from typing import Any

try:
    import yaml
except ImportError:
    print(
        "error: PyYAML is required. Install with: pip install pyyaml",
        file=sys.stderr,
    )
    sys.exit(1)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _sort_keys_recursive(obj: Any) -> Any:
    """Return *obj* with all nested mapping keys sorted alphabetically.

    Lists are kept in insertion order (preserving ordered sequences such as
    container ports and environment variables); only mappings are sorted.
    """
    if isinstance(obj, dict):
        return {k: _sort_keys_recursive(obj[k]) for k in sorted(obj)}
    if isinstance(obj, list):
        return [_sort_keys_recursive(item) for item in obj]
    return obj


def _doc_sort_key(doc: dict) -> tuple[str, str, str]:
    """Return a stable three-tuple sort key for a Kubernetes manifest.

    Ordering: kind → namespace → name.  Missing fields sort to the empty
    string so that incomplete manifests don't crash the sort.
    """
    kind = doc.get("kind", "")
    metadata = doc.get("metadata") or {}
    namespace = metadata.get("namespace", "")
    name = metadata.get("name", "")
    return (kind, namespace, name)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> None:
    # Read from a file argument or stdin.
    if len(sys.argv) > 1:
        with open(sys.argv[1], encoding="utf-8") as fh:
            raw = fh.read()
    else:
        raw = sys.stdin.read()

    # Parse all documents; drop nulls (empty --- blocks).
    docs = [d for d in yaml.safe_load_all(raw) if d is not None]

    # Sort documents by (kind, namespace, name).
    docs.sort(key=_doc_sort_key)

    # Recursively sort all mapping keys and emit.
    sorted_docs = [_sort_keys_recursive(d) for d in docs]

    output = yaml.dump_all(
        sorted_docs,
        default_flow_style=False,
        allow_unicode=True,
        sort_keys=True,   # belt-and-suspenders: also sorts at the YAML emitter level
        explicit_start=True,
        width=10**9,      # avoid spurious wrapping diffs vs committed manifests
    )
    sys.stdout.write(output)


if __name__ == "__main__":
    main()
