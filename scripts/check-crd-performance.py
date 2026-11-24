#!/usr/bin/env python3
"""CRD performance regression detection — Issue #1287.

Compares current CRD operation benchmark results against a baseline and
fails when performance regresses by more than the configured threshold.

Usage:
    python3 scripts/check-crd-performance.py \\
        --current results/crd-benchmark.json \\
        --baseline benchmarks/baselines/crd-performance-v0.1.0.json \\
        --threshold 10

Exit codes:
    0  No regression detected
    1  Performance regression exceeds threshold
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_BASELINE = REPO_ROOT / "benchmarks" / "baselines" / "crd-performance-v0.1.0.json"


def load_json(path: Path) -> dict[str, Any]:
    """Load a JSON file."""
    if not path.is_file():
        raise SystemExit(f"File not found: {path}")
    with path.open(encoding="utf-8") as fh:
        return json.load(fh)


def detect_regression(
    current: dict[str, Any],
    baseline: dict[str, Any],
    threshold: float = 10.0,
) -> list[dict[str, Any]]:
    """Compare current results against baseline, return list of regressions."""
    regressions = []
    current_metrics = current.get("metrics", {})
    baseline_metrics = baseline.get("metrics", {})

    for metric_name, current_value in current_metrics.items():
        if metric_name not in baseline_metrics:
            continue

        baseline_value = baseline_metrics[metric_name]
        if not isinstance(current_value, (int, float)) or not isinstance(
            baseline_value, (int, float)
        ):
            continue
        if baseline_value == 0:
            continue

        change_pct = ((current_value - baseline_value) / baseline_value) * 100

        # For latency metrics, increase is bad; for throughput, decrease is bad
        is_latency = any(
            x in metric_name.lower()
            for x in ["latency", "duration", "time", "p95", "p99", "p50"]
        )

        is_regression = False
        if is_latency:
            is_regression = change_pct > threshold
        else:
            is_regression = change_pct < -threshold

        if is_regression:
            regressions.append(
                {
                    "metric": metric_name,
                    "current": current_value,
                    "baseline": baseline_value,
                    "change_pct": round(change_pct, 2),
                    "threshold": threshold,
                    "direction": "increase" if is_latency else "decrease",
                }
            )

    return regressions


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="CRD performance regression detection"
    )
    parser.add_argument(
        "--current",
        type=Path,
        required=True,
        help="Current benchmark results JSON",
    )
    parser.add_argument(
        "--baseline",
        type=Path,
        default=DEFAULT_BASELINE,
        help="Baseline benchmark JSON",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=10.0,
        help="Regression threshold percentage (default: 10)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Output report JSON",
    )
    args = parser.parse_args(argv)

    current = load_json(args.current)
    baseline = load_json(args.baseline)

    regressions = detect_regression(current, baseline, args.threshold)

    report = {
        "overall_passed": len(regressions) == 0,
        "threshold_percent": args.threshold,
        "regressions": regressions,
        "summary": (
            f"No regressions detected (threshold: {args.threshold}%)"
            if not regressions
            else f"{len(regressions)} regression(s) exceeding {args.threshold}% threshold"
        ),
    }

    if args.output:
        with args.output.open("w", encoding="utf-8") as fh:
            json.dump(report, fh, indent=2)
        print(f"Report written to {args.output}")

    if regressions:
        print(f"\nCRD Performance Regression DETECTED ({len(regressions)} metric(s)):")
        print(f"{'='*70}")
        for reg in regressions:
            print(
                f"  ✗ {reg['metric']}: {reg['baseline']:.1f} -> {reg['current']:.1f} "
                f"({reg['change_pct']:+.1f}% {reg['direction']}, threshold: ±{reg['threshold']}%)"
            )
        print(f"{'='*70}")
        return 1

    print(f"✓ CRD performance within {args.threshold}% threshold of baseline")
    return 0


if __name__ == "__main__":
    sys.exit(main())
