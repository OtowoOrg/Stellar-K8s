#!/usr/bin/env python3
"""Collect criterion benchmark output into the repo's baseline JSON shape — Issue #1390.

`cargo bench` writes one `estimates.json` per benchmark under
`target/criterion/<group>/<id>/new/estimates.json`. This script walks that
tree and produces a single JSON file of the form

    {"metrics": {"<group>_<id>_ms": <mean_ms>, ...}}

which is the same shape used by `benchmarks/baselines/*.json` and understood
by `benchmarks/scripts/compare_benchmarks.py` / `.github/actions/compare-benchmarks`
(the same regression-comparison path the k6 suites already use).

Usage:
    python3 scripts/collect-criterion-results.py \\
        --criterion-dir target/criterion \\
        --output results/crd-benchmark.json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def sanitize(component: str) -> str:
    """Turn a criterion group/id path component into a metric-name segment."""
    s = component.replace("-", "_")
    s = re.sub(r"[^a-zA-Z0-9_]+", "_", s)
    s = re.sub(r"_+", "_", s).strip("_")
    return s.lower()


def collect(criterion_dir: Path) -> dict[str, float]:
    """Walk `criterion_dir` and build a flat {metric_name: mean_ms} dict."""
    metrics: dict[str, float] = {}

    for estimates_file in sorted(criterion_dir.glob("**/new/estimates.json")):
        # Path layout: <criterion_dir>/<group>/[<id>/]new/estimates.json
        rel_parts = estimates_file.relative_to(criterion_dir).parts[:-2]
        if not rel_parts:
            continue

        # Criterion also writes a "<group>/report" summary dir with no
        # estimates.json of its own, plus a per-group aggregate at
        # "<group>/new/estimates.json" when there's only one benchmark in
        # the group — both cases are handled by just sanitizing whatever
        # path components remain.
        name = "_".join(sanitize(part) for part in rel_parts if sanitize(part))
        if not name:
            continue

        with estimates_file.open(encoding="utf-8") as fh:
            estimates = json.load(fh)

        mean_ns = estimates.get("mean", {}).get("point_estimate")
        if mean_ns is None:
            continue

        metrics[f"{name}_ms"] = round(mean_ns / 1_000_000, 6)

    return metrics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--criterion-dir",
        type=Path,
        default=Path("target/criterion"),
        help="Directory criterion wrote its report tree into (default: target/criterion)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="Path to write the collected {\"metrics\": {...}} JSON to",
    )
    args = parser.parse_args(argv)

    if not args.criterion_dir.is_dir():
        print(f"error: criterion directory not found: {args.criterion_dir}", file=sys.stderr)
        return 1

    metrics = collect(args.criterion_dir)
    if not metrics:
        print(f"warning: no estimates.json files found under {args.criterion_dir}", file=sys.stderr)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as fh:
        json.dump({"metrics": metrics}, fh, indent=2, sort_keys=True)
        fh.write("\n")

    print(f"Collected {len(metrics)} metric(s) from {args.criterion_dir} -> {args.output}")
    for name, value in sorted(metrics.items()):
        print(f"  {name} = {value} ms")

    return 0


if __name__ == "__main__":
    sys.exit(main())
