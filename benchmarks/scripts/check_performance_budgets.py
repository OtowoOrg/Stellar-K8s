#!/usr/bin/env python3
"""Evaluate a k6 run against the declared performance budgets (issue #1336).

`benchmarks/scripts/compare_benchmarks.py` answers "did this get slower than
last time?" — a *relative* question, which cannot tell you whether the absolute
numbers were ever acceptable. Two 8% regressions in a row pass a 10% threshold
while doubling latency over a month.

This script answers the absolute question instead: does the run meet the SLO
targets in `benchmarks/performance-budgets.yaml`? Both gates run in CI; they
catch different failures.

Usage:
    check_performance_budgets.py --results results/benchmark-summary.json \\
                                 --suite operator
    check_performance_budgets.py --results r.json --suite webhook --format json
    check_performance_budgets.py --results r.json --suite operator \\
                                 --markdown "$GITHUB_STEP_SUMMARY" \\
                                 --history benchmarks/history

Exit codes: 0 = all blocking budgets met, 1 = a blocking budget was violated,
2 = bad invocation (missing file, unknown suite, malformed results).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Sequence

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("PyYAML is required: pip install -r requirements.txt")

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DEFAULT_BUDGETS = REPO_ROOT / "benchmarks" / "performance-budgets.yaml"

# How each comparison reads, and whether the measured value must be small.
COMPARISONS = {
    "lt": (lambda value, budget: value < budget, "<"),
    "lte": (lambda value, budget: value <= budget, "<="),
    "gt": (lambda value, budget: value > budget, ">"),
    "gte": (lambda value, budget: value >= budget, ">="),
}


@dataclass
class Budget:
    """One SLO target."""

    metric: str
    comparison: str
    budget: float
    unit: str = ""
    blocking: bool = True
    description: str = ""


@dataclass
class Result:
    """The verdict for one budget against one run."""

    budget: Budget
    value: float | None
    passed: bool
    missing: bool = False

    @property
    def symbol(self) -> str:
        if self.missing:
            return "?"
        return "PASS" if self.passed else "FAIL"

    def headroom_percent(self) -> float | None:
        """How much room is left before the budget is breached, as a percent.

        Negative means the budget is already exceeded. Reported so a run that
        passes at 99% of budget is visibly different from one at 40%.
        """
        if self.value is None or self.budget.budget == 0:
            return None
        if self.budget.comparison in ("lt", "lte"):
            return (self.budget.budget - self.value) / self.budget.budget * 100
        return (self.value - self.budget.budget) / self.budget.budget * 100

    def as_dict(self) -> dict:
        return {
            "metric": self.budget.metric,
            "comparison": self.budget.comparison,
            "budget": self.budget.budget,
            "unit": self.budget.unit,
            "blocking": self.budget.blocking,
            "value": self.value,
            "passed": self.passed,
            "missing": self.missing,
            "headroom_percent": self.headroom_percent(),
        }


@dataclass
class Report:
    """Every verdict for one suite."""

    suite: str
    results: list[Result] = field(default_factory=list)
    run_metadata: dict = field(default_factory=dict)

    @property
    def violations(self) -> list[Result]:
        return [r for r in self.results if not r.passed and not r.missing]

    @property
    def blocking_violations(self) -> list[Result]:
        return [r for r in self.violations if r.budget.blocking]

    @property
    def missing(self) -> list[Result]:
        return [r for r in self.results if r.missing]

    @property
    def passed(self) -> bool:
        """A missing blocking metric fails too.

        Silently passing because a metric vanished from the summary is exactly
        how a performance gate stops protecting anything.
        """
        return not self.blocking_violations and not [
            r for r in self.missing if r.budget.blocking
        ]

    def as_dict(self) -> dict:
        return {
            "suite": self.suite,
            "passed": self.passed,
            "evaluated": len(self.results),
            "violations": len(self.violations),
            "blocking_violations": len(self.blocking_violations),
            "missing_metrics": len(self.missing),
            "run": self.run_metadata,
            "results": [r.as_dict() for r in self.results],
        }


def load_budgets(path: Path, suite: str) -> tuple[list[Budget], str]:
    """Read the budgets declared for `suite`."""
    if not path.is_file():
        raise SystemExit(f"budgets file not found: {path}")
    document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    defaults = document.get("defaults") or {}
    suites = document.get("suites") or {}

    if suite not in suites:
        known = ", ".join(sorted(suites)) or "(none)"
        raise SystemExit(f"unknown suite '{suite}'; known suites: {known}")

    entry = suites[suite] or {}
    budgets = []
    for raw in entry.get("budgets") or []:
        comparison = raw.get("comparison", "lt")
        if comparison not in COMPARISONS:
            raise SystemExit(
                f"budget for '{raw.get('metric')}' uses unknown comparison "
                f"'{comparison}'; expected one of {', '.join(sorted(COMPARISONS))}"
            )
        budgets.append(
            Budget(
                metric=raw["metric"],
                comparison=comparison,
                budget=float(raw["budget"]),
                unit=raw.get("unit", ""),
                blocking=bool(raw.get("blocking", defaults.get("blocking", True))),
                description=raw.get("description", ""),
            )
        )
    if not budgets:
        raise SystemExit(f"suite '{suite}' declares no budgets")
    return budgets, entry.get("description", "")


def read_metric(document: Any, dotted_path: str) -> float | None:
    """Resolve a dotted path such as `metrics.http_req_duration.p95`."""
    node = document
    for part in dotted_path.split("."):
        if not isinstance(node, dict) or part not in node:
            return None
        node = node[part]
    if isinstance(node, bool) or not isinstance(node, (int, float)):
        return None
    return float(node)


def evaluate(results_document: dict, budgets: Sequence[Budget], suite: str) -> Report:
    """Check every budget against the run."""
    report = Report(
        suite=suite,
        run_metadata={
            key: results_document.get(key)
            for key in ("timestamp", "runId", "version", "gitSha")
            if results_document.get(key) is not None
        },
    )
    for budget in budgets:
        value = read_metric(results_document, budget.metric)
        if value is None:
            report.results.append(Result(budget=budget, value=None, passed=False, missing=True))
            continue
        predicate, _ = COMPARISONS[budget.comparison]
        report.results.append(
            Result(budget=budget, value=value, passed=predicate(value, budget.budget))
        )
    return report


def _format_value(value: float | None) -> str:
    if value is None:
        return "—"
    if abs(value) < 0.01 and value != 0:
        return f"{value:.5f}"
    return f"{value:,.2f}".rstrip("0").rstrip(".") if value % 1 else f"{value:,.0f}"


def render_markdown(report: Report, suite_description: str) -> str:
    """A CI job-summary table."""
    _, operators = {}, {k: v[1] for k, v in COMPARISONS.items()}
    lines: list[str] = []
    verdict = "✅ within budget" if report.passed else "❌ budget exceeded"
    lines.append(f"## Performance budgets — `{report.suite}` {verdict}")
    if suite_description:
        lines.append("")
        lines.append(f"_{suite_description.strip()}_")
    if report.run_metadata:
        meta = "  ".join(f"**{k}**: `{v}`" for k, v in report.run_metadata.items())
        lines.append("")
        lines.append(meta)
    lines.append("")
    lines.append("| | Metric | Measured | Budget | Headroom | Gate |")
    lines.append("|---|---|---:|---:|---:|---|")
    for result in report.results:
        budget = result.budget
        headroom = result.headroom_percent()
        headroom_text = "—" if headroom is None else f"{headroom:+.1f}%"
        icon = "❓" if result.missing else ("✅" if result.passed else "❌")
        unit = f" {budget.unit}" if budget.unit else ""
        lines.append(
            f"| {icon} | `{budget.metric}` | {_format_value(result.value)}{unit} "
            f"| {operators[budget.comparison]} {_format_value(budget.budget)}{unit} "
            f"| {headroom_text} | {'blocking' if budget.blocking else 'advisory'} |"
        )

    if report.missing:
        lines.append("")
        lines.append("> **Missing metrics** — not present in the run summary:")
        for result in report.missing:
            lines.append(f"> - `{result.budget.metric}`")

    if report.violations:
        lines.append("")
        lines.append("### Violations")
        for result in report.violations:
            budget = result.budget
            lines.append(
                f"- `{budget.metric}` = {_format_value(result.value)}{(' ' + budget.unit) if budget.unit else ''}"
                f", budget {operators[budget.comparison]} {_format_value(budget.budget)}"
                f" ({'blocking' if budget.blocking else 'advisory'})"
            )
            if budget.description:
                lines.append(f"  - {budget.description.strip()}")
    return "\n".join(lines) + "\n"


def render_text(report: Report) -> str:
    operators = {k: v[1] for k, v in COMPARISONS.items()}
    lines = [f"→ Performance budgets for suite '{report.suite}'", ""]
    for result in report.results:
        budget = result.budget
        unit = f" {budget.unit}" if budget.unit else ""
        lines.append(
            f"  [{result.symbol:>4}] {budget.metric}: "
            f"{_format_value(result.value)}{unit} "
            f"(budget {operators[budget.comparison]} {_format_value(budget.budget)}{unit}"
            f", {'blocking' if budget.blocking else 'advisory'})"
        )
    lines.append("")
    lines.append("━" * 60)
    lines.append(
        f"Budgets: {len(report.results)} evaluated, "
        f"{len(report.violations)} violated "
        f"({len(report.blocking_violations)} blocking), "
        f"{len(report.missing)} missing"
    )
    lines.append("━" * 60)
    lines.append("")
    lines.append("✅ All blocking budgets met" if report.passed else "❌ Performance budget FAILED")
    return "\n".join(lines)


def append_history(report: Report, directory: Path) -> Path:
    """Append this run to a per-suite JSONL history.

    One line per run keeps the series appendable from CI without a database,
    and greppable/plottable afterwards.
    """
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / f"{report.suite}.jsonl"
    entry = {
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "passed": report.passed,
        "run": report.run_metadata,
        "metrics": {
            r.budget.metric: r.value for r in report.results if r.value is not None
        },
    }
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(entry) + "\n")
    return path


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Evaluate k6 results against performance budgets")
    parser.add_argument("--results", required=True, help="k6 handleSummary JSON")
    parser.add_argument("--suite", required=True, help="suite name from the budgets file")
    parser.add_argument("--budgets", default=str(DEFAULT_BUDGETS))
    parser.add_argument("--format", choices=("text", "json", "markdown"), default="text")
    parser.add_argument("--markdown", help="also write a markdown report to this path (append)")
    parser.add_argument("--history", help="append this run to a JSONL history in this directory")
    parser.add_argument(
        "--warn-only",
        action="store_true",
        help="report violations but always exit 0",
    )
    args = parser.parse_args(argv)

    results_path = Path(args.results)
    if not results_path.is_file():
        raise SystemExit(f"results file not found: {results_path}")
    try:
        document = json.loads(results_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise SystemExit(f"results file is not valid JSON: {exc}") from exc
    if not isinstance(document, dict):
        raise SystemExit("results file must contain a JSON object")

    budgets, suite_description = load_budgets(Path(args.budgets), args.suite)
    report = evaluate(document, budgets, args.suite)

    if args.format == "json":
        print(json.dumps(report.as_dict(), indent=2))
    elif args.format == "markdown":
        print(render_markdown(report, suite_description), end="")
    else:
        print(render_text(report))

    if args.markdown:
        with open(args.markdown, "a", encoding="utf-8") as handle:
            handle.write(render_markdown(report, suite_description))

    if args.history:
        path = append_history(report, Path(args.history))
        print(f"\nHistory appended to {path}", file=sys.stderr)

    if os.environ.get("GITHUB_ACTIONS") == "true":
        for result in report.blocking_violations:
            print(
                f"::error title=Performance budget::{result.budget.metric} "
                f"= {_format_value(result.value)} violates budget {result.budget.budget}"
            )

    if args.warn_only:
        return 0
    return 0 if report.passed else 1


if __name__ == "__main__":
    sys.exit(main())
