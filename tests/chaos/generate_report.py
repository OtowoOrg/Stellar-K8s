#!/usr/bin/env python3
"""
Chaos Engineering Resilience Report Generator
==============================================
Parses operator logs and experiment metadata from a chaos test run,
computes resilience scores, and emits a structured Markdown report
plus a machine-readable JSON summary.

Usage:
    python3 generate_report.py --results-dir tests/chaos/results/<run-id>
    python3 generate_report.py --results-dir tests/chaos/results/<run-id> \
                               --output-format both   # markdown + json
    python3 generate_report.py --help
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional


# ── Experiment metadata ────────────────────────────────────────────────────

EXPERIMENT_META: dict[str, dict] = {
    "01": {
        "name": "Operator Pod Kill",
        "category": "pod-failure",
        "description": "Operator pod receives SIGKILL during active reconciliation.",
        "slo_recovery_secs": 180,
        "severity": "critical",
    },
    "02": {
        "name": "Network Partition",
        "category": "network",
        "description": "Full bidirectional partition between operator and K8s API.",
        "slo_recovery_secs": 180,
        "severity": "critical",
    },
    "03": {
        "name": "API High Latency",
        "category": "network",
        "description": "2 s delay + 500 ms jitter on every API call.",
        "slo_recovery_secs": 600,
        "severity": "high",
    },
    "04": {
        "name": "Validator Peer Partition",
        "category": "network",
        "description": "Validator pods partitioned from each other (SCP peer ports).",
        "slo_recovery_secs": 300,
        "severity": "high",
    },
    "05": {
        "name": "Disk Fill",
        "category": "storage",
        "description": "Operator pod disk filled to capacity.",
        "slo_recovery_secs": 120,
        "severity": "medium",
    },
    "06": {
        "name": "CPU Stress",
        "category": "resource-exhaustion",
        "description": "4 CPU workers at 90% load on the operator pod.",
        "slo_recovery_secs": 300,
        "severity": "medium",
    },
    "07": {
        "name": "Memory Pressure",
        "category": "resource-exhaustion",
        "description": "256 MB memory consumed on the operator pod.",
        "slo_recovery_secs": 300,
        "severity": "medium",
    },
    "08": {
        "name": "Validator Pod Kill",
        "category": "pod-failure",
        "description": "Validator pods killed repeatedly while operator manages them.",
        "slo_recovery_secs": 300,
        "severity": "high",
    },
    "09": {
        "name": "Cascading Failure",
        "category": "cascading",
        "description": "Simultaneous pod kill + network partition (worst-case).",
        "slo_recovery_secs": 600,
        "severity": "critical",
    },
    "10": {
        "name": "I/O Stress",
        "category": "storage",
        "description": "Validator pod storage saturated with concurrent I/O workers.",
        "slo_recovery_secs": 300,
        "severity": "medium",
    },
}

# ── Log pattern matchers ───────────────────────────────────────────────────

PANIC_RE = re.compile(r"\bpanic\b|\bPANIC\b", re.IGNORECASE)
ERROR_RE = re.compile(r"\bERROR\b|\blevel=error\b", re.IGNORECASE)
WARN_RE = re.compile(r"\bWARN\b|\blevel=warn\b", re.IGNORECASE)
RECONCILE_RE = re.compile(r"Reconcil(ing|ed|iation)", re.IGNORECASE)
RECOVERY_RE = re.compile(r"(Applied|Ready|converged|recovered|requeued)", re.IGNORECASE)
CRASH_LOOP_RE = re.compile(r"CrashLoopBackOff", re.IGNORECASE)
OOM_RE = re.compile(r"OOMKill|out of memory", re.IGNORECASE)
TIMEOUT_RE = re.compile(r"(context deadline exceeded|timeout|timed out)", re.IGNORECASE)
DUPLICATE_RE = re.compile(r"(already exists|duplicate|AlreadyExists)", re.IGNORECASE)
STUCK_FINALIZER_RE = re.compile(r"(stuck finalizer|finalizer.*not removed)", re.IGNORECASE)


# ── Data classes ──────────────────────────────────────────────────────────

@dataclass
class LogMetrics:
    total_lines: int = 0
    error_count: int = 0
    warn_count: int = 0
    panic_count: int = 0
    reconcile_count: int = 0
    recovery_count: int = 0
    crash_loop_count: int = 0
    oom_count: int = 0
    timeout_count: int = 0
    duplicate_count: int = 0
    stuck_finalizer_count: int = 0
    sample_errors: list[str] = field(default_factory=list)
    sample_recoveries: list[str] = field(default_factory=list)


@dataclass
class ExperimentResult:
    experiment_id: str
    name: str
    category: str
    description: str
    severity: str
    slo_recovery_secs: int
    log_metrics: LogMetrics
    recovery_time_secs: Optional[int]
    passed: bool
    score: float          # 0.0 – 100.0
    failure_reasons: list[str] = field(default_factory=list)
    warnings: list[str] = field(default_factory=list)


@dataclass
class ReportSummary:
    run_id: str
    generated_at: str
    git_sha: str
    total_experiments: int
    passed: int
    failed: int
    warned: int
    overall_score: float
    results: list[ExperimentResult]


# ── Log parsing ───────────────────────────────────────────────────────────

def parse_log_file(log_path: Path) -> LogMetrics:
    m = LogMetrics()
    if not log_path.exists():
        return m

    with log_path.open(errors="replace") as fh:
        for line in fh:
            m.total_lines += 1
            if PANIC_RE.search(line):
                m.panic_count += 1
            if ERROR_RE.search(line):
                m.error_count += 1
                if len(m.sample_errors) < 5:
                    m.sample_errors.append(line.rstrip())
            if WARN_RE.search(line):
                m.warn_count += 1
            if RECONCILE_RE.search(line):
                m.reconcile_count += 1
            if RECOVERY_RE.search(line):
                m.recovery_count += 1
                if len(m.sample_recoveries) < 3:
                    m.sample_recoveries.append(line.rstrip())
            if CRASH_LOOP_RE.search(line):
                m.crash_loop_count += 1
            if OOM_RE.search(line):
                m.oom_count += 1
            if TIMEOUT_RE.search(line):
                m.timeout_count += 1
            if DUPLICATE_RE.search(line):
                m.duplicate_count += 1
            if STUCK_FINALIZER_RE.search(line):
                m.stuck_finalizer_count += 1

    return m


def parse_recovery_time(results_dir: Path, exp_id: str) -> Optional[int]:
    """Read recovery_time_secs from a sidecar JSON file if present."""
    meta_file = results_dir / f"exp{exp_id}-meta.json"
    if meta_file.exists():
        try:
            data = json.loads(meta_file.read_text())
            return data.get("recovery_time_secs")
        except (json.JSONDecodeError, KeyError):
            pass
    return None


# ── Scoring ───────────────────────────────────────────────────────────────

def score_experiment(m: LogMetrics, meta: dict, recovery_secs: Optional[int]) -> tuple[float, list[str], list[str]]:
    """
    Returns (score 0-100, failure_reasons, warnings).

    Scoring rubric:
      - Panic:              -50 pts each (hard failure)
      - CrashLoopBackOff:  -30 pts
      - OOMKill:           -30 pts
      - Duplicate resource: -20 pts
      - Stuck finalizer:   -20 pts
      - No reconciliation: -20 pts (operator stopped working)
      - SLO breach:        -15 pts
      - High error rate:   -10 pts (>10% of log lines)
    """
    score = 100.0
    failures: list[str] = []
    warnings: list[str] = []

    if m.panic_count > 0:
        score -= 50 * min(m.panic_count, 2)
        failures.append(f"Operator panicked {m.panic_count} time(s)")

    if m.crash_loop_count > 0:
        score -= 30
        failures.append("CrashLoopBackOff detected")

    if m.oom_count > 0:
        score -= 30
        failures.append(f"OOMKill detected ({m.oom_count} occurrence(s))")

    if m.duplicate_count > 0:
        score -= 20
        failures.append(f"Duplicate resource creation detected ({m.duplicate_count} occurrence(s))")

    if m.stuck_finalizer_count > 0:
        score -= 20
        failures.append(f"Stuck finalizer detected ({m.stuck_finalizer_count} occurrence(s))")

    if m.total_lines > 0 and m.reconcile_count == 0:
        score -= 20
        failures.append("No reconciliation activity detected — operator may have stopped")

    slo = meta["slo_recovery_secs"]
    if recovery_secs is not None and recovery_secs > slo:
        score -= 15
        warnings.append(f"Recovery time {recovery_secs}s exceeded SLO of {slo}s")
    elif recovery_secs is None and m.recovery_count == 0:
        score -= 10
        warnings.append("No recovery activity detected in logs")

    if m.total_lines > 0:
        error_rate = m.error_count / m.total_lines
        if error_rate > 0.10:
            score -= 10
            warnings.append(f"High error rate: {error_rate:.1%} of log lines are errors")

    if m.timeout_count > 50:
        warnings.append(f"High timeout count: {m.timeout_count} — possible API server overload")

    return max(0.0, score), failures, warnings


# ── Report building ───────────────────────────────────────────────────────

def build_results(results_dir: Path) -> list[ExperimentResult]:
    results: list[ExperimentResult] = []

    for exp_id, meta in sorted(EXPERIMENT_META.items()):
        log_file = results_dir / f"exp{exp_id}-operator-logs.txt"
        if not log_file.exists():
            # Also try the older naming convention
            log_file = results_dir / f"{exp_id}-operator-logs.txt"
        if not log_file.exists():
            continue

        lm = parse_log_file(log_file)
        recovery_secs = parse_recovery_time(results_dir, exp_id)
        score, failures, warnings = score_experiment(lm, meta, recovery_secs)

        results.append(ExperimentResult(
            experiment_id=exp_id,
            name=meta["name"],
            category=meta["category"],
            description=meta["description"],
            severity=meta["severity"],
            slo_recovery_secs=meta["slo_recovery_secs"],
            log_metrics=lm,
            recovery_time_secs=recovery_secs,
            passed=len(failures) == 0,
            score=score,
            failure_reasons=failures,
            warnings=warnings,
        ))

    return results


def overall_score(results: list[ExperimentResult]) -> float:
    if not results:
        return 0.0
    # Weight critical experiments more heavily
    weight_map = {"critical": 3, "high": 2, "medium": 1}
    total_weight = sum(weight_map.get(r.severity, 1) for r in results)
    weighted_sum = sum(r.score * weight_map.get(r.severity, 1) for r in results)
    return weighted_sum / total_weight if total_weight else 0.0


# ── Markdown rendering ────────────────────────────────────────────────────

SCORE_EMOJI = {
    (90, 101): "🟢",
    (70, 90):  "🟡",
    (0,  70):  "🔴",
}

def score_emoji(score: float) -> str:
    for (lo, hi), emoji in SCORE_EMOJI.items():
        if lo <= score < hi:
            return emoji
    return "⚪"


def render_markdown(summary: ReportSummary) -> str:
    lines: list[str] = []
    a = lines.append

    a("# 🔥 Chaos Engineering Resilience Report")
    a("")
    a(f"**Generated:** {summary.generated_at}  ")
    a(f"**Run ID:** `{summary.run_id}`  ")
    a(f"**Commit:** `{summary.git_sha}`  ")
    a("")

    # ── Executive summary ──────────────────────────────────────────────
    emoji = score_emoji(summary.overall_score)
    a("## Executive Summary")
    a("")
    a(f"| Metric | Value |")
    a(f"|--------|-------|")
    a(f"| Overall Resilience Score | {emoji} **{summary.overall_score:.1f} / 100** |")
    a(f"| Experiments Run | {summary.total_experiments} |")
    a(f"| Passed | ✅ {summary.passed} |")
    a(f"| Failed | ❌ {summary.failed} |")
    a(f"| Warnings | ⚠️ {summary.warned} |")
    a("")

    # ── Score interpretation ───────────────────────────────────────────
    if summary.overall_score >= 90:
        verdict = "🟢 **EXCELLENT** — The operator is highly resilient. All critical experiments passed."
    elif summary.overall_score >= 70:
        verdict = "🟡 **GOOD** — The operator handles most failures gracefully. Review warnings."
    elif summary.overall_score >= 50:
        verdict = "🟠 **FAIR** — Several resilience gaps detected. Remediation recommended."
    else:
        verdict = "🔴 **POOR** — Critical resilience failures detected. Immediate action required."

    a(f"**Verdict:** {verdict}")
    a("")

    # ── Results table ──────────────────────────────────────────────────
    a("## Experiment Results")
    a("")
    a("| # | Experiment | Category | Severity | Score | Status |")
    a("|---|-----------|----------|----------|-------|--------|")
    for r in summary.results:
        status = "✅ PASS" if r.passed else "❌ FAIL"
        if r.passed and r.warnings:
            status = "⚠️ WARN"
        a(f"| {r.experiment_id} | {r.name} | {r.category} | {r.severity} "
          f"| {score_emoji(r.score)} {r.score:.0f} | {status} |")
    a("")

    # ── Per-experiment detail ──────────────────────────────────────────
    a("## Detailed Results")
    a("")
    for r in summary.results:
        status_icon = "✅" if r.passed else "❌"
        if r.passed and r.warnings:
            status_icon = "⚠️"
        a(f"### {status_icon} Experiment {r.experiment_id}: {r.name}")
        a("")
        a(f"> {r.description}")
        a("")
        a(f"**Score:** {score_emoji(r.score)} {r.score:.1f}/100 | "
          f"**Severity:** {r.severity} | "
          f"**SLO:** {r.slo_recovery_secs}s recovery")
        a("")

        lm = r.log_metrics
        a("#### Log Metrics")
        a("")
        a("| Metric | Count |")
        a("|--------|-------|")
        a(f"| Total log lines | {lm.total_lines:,} |")
        a(f"| Errors | {lm.error_count:,} |")
        a(f"| Warnings | {lm.warn_count:,} |")
        a(f"| Panics | {lm.panic_count} |")
        a(f"| Reconciliation events | {lm.reconcile_count:,} |")
        a(f"| Recovery events | {lm.recovery_count:,} |")
        a(f"| Timeouts | {lm.timeout_count:,} |")
        a(f"| OOMKill events | {lm.oom_count} |")
        a(f"| Duplicate resources | {lm.duplicate_count} |")
        a(f"| Stuck finalizers | {lm.stuck_finalizer_count} |")
        if r.recovery_time_secs is not None:
            slo_met = "✅" if r.recovery_time_secs <= r.slo_recovery_secs else "❌"
            a(f"| Recovery time | {r.recovery_time_secs}s {slo_met} |")
        a("")

        if r.failure_reasons:
            a("#### ❌ Failure Reasons")
            a("")
            for reason in r.failure_reasons:
                a(f"- {reason}")
            a("")

        if r.warnings:
            a("#### ⚠️ Warnings")
            a("")
            for w in r.warnings:
                a(f"- {w}")
            a("")

        if lm.sample_errors:
            a("#### Sample Error Lines")
            a("")
            a("```")
            for line in lm.sample_errors:
                a(line[:200])
            a("```")
            a("")

        if lm.sample_recoveries:
            a("#### Sample Recovery Lines")
            a("")
            a("```")
            for line in lm.sample_recoveries:
                a(line[:200])
            a("```")
            a("")

    # ── Recommendations ────────────────────────────────────────────────
    a("## Recommendations")
    a("")
    failed = [r for r in summary.results if not r.passed]
    warned = [r for r in summary.results if r.passed and r.warnings]

    if not failed and not warned:
        a("✅ All experiments passed without issues. Continue running chaos tests regularly.")
    else:
        if failed:
            a("### Critical Actions Required")
            a("")
            for r in failed:
                a(f"**{r.name}:**")
                for reason in r.failure_reasons:
                    a(f"  - {reason}")
            a("")

        if warned:
            a("### Improvements Recommended")
            a("")
            for r in warned:
                a(f"**{r.name}:**")
                for w in r.warnings:
                    a(f"  - {w}")
            a("")

    a("### General Best Practices")
    a("")
    a("- Run chaos tests on every release candidate, not just nightly")
    a("- Add circuit breakers for all external dependencies (K8s API, DB, Kafka)")
    a("- Ensure all reconciliation paths have idempotency guarantees")
    a("- Set resource limits on the operator pod to prevent OOMKill")
    a("- Use exponential backoff with jitter for all retry loops")
    a("- Instrument recovery time as a Prometheus metric for SLO tracking")
    a("")
    a("---")
    a(f"*Generated by Stellar-K8s Chaos Engineering Suite — {summary.generated_at}*")

    return "\n".join(lines)


# ── JSON rendering ────────────────────────────────────────────────────────

def render_json(summary: ReportSummary) -> str:
    def _serialise(obj):
        if isinstance(obj, ExperimentResult):
            d = asdict(obj)
            d["log_metrics"] = asdict(obj.log_metrics)
            return d
        raise TypeError(f"Not serialisable: {type(obj)}")

    data = {
        "run_id": summary.run_id,
        "generated_at": summary.generated_at,
        "git_sha": summary.git_sha,
        "overall_score": round(summary.overall_score, 2),
        "total_experiments": summary.total_experiments,
        "passed": summary.passed,
        "failed": summary.failed,
        "warned": summary.warned,
        "results": [asdict(r) for r in summary.results],
    }
    return json.dumps(data, indent=2, default=str)


# ── GitHub Actions step summary ───────────────────────────────────────────

def write_github_summary(summary: ReportSummary, output_path: Optional[str]) -> None:
    """Write a compact summary to $GITHUB_STEP_SUMMARY if running in CI."""
    summary_file = output_path or os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_file:
        return

    emoji = score_emoji(summary.overall_score)
    lines = [
        "## 🔥 Chaos Engineering Results",
        "",
        f"**Overall Score:** {emoji} {summary.overall_score:.1f}/100  ",
        f"**Commit:** `{summary.git_sha}`",
        "",
        "| # | Experiment | Score | Status |",
        "|---|-----------|-------|--------|",
    ]
    for r in summary.results:
        status = "✅ PASS" if r.passed else "❌ FAIL"
        if r.passed and r.warnings:
            status = "⚠️ WARN"
        lines.append(f"| {r.experiment_id} | {r.name} | {score_emoji(r.score)} {r.score:.0f} | {status} |")

    with open(summary_file, "a") as fh:
        fh.write("\n".join(lines) + "\n")


# ── CLI ───────────────────────────────────────────────────────────────────

def get_git_sha() -> str:
    try:
        import subprocess
        return subprocess.check_output(
            ["git", "rev-parse", "--short", "HEAD"],
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except Exception:
        return os.environ.get("GITHUB_SHA", "unknown")[:12]


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate a chaos engineering resilience report.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "--results-dir",
        required=True,
        help="Directory containing experiment log files (exp01-operator-logs.txt, etc.)",
    )
    parser.add_argument(
        "--output-format",
        choices=["markdown", "json", "both"],
        default="both",
        help="Output format (default: both)",
    )
    parser.add_argument(
        "--run-id",
        default=None,
        help="Run identifier (defaults to results directory name)",
    )
    parser.add_argument(
        "--github-summary",
        default=None,
        help="Path to write GitHub Actions step summary (defaults to $GITHUB_STEP_SUMMARY)",
    )
    args = parser.parse_args()

    results_dir = Path(args.results_dir)
    if not results_dir.is_dir():
        print(f"ERROR: Results directory not found: {results_dir}", file=sys.stderr)
        return 1

    run_id = args.run_id or results_dir.name
    git_sha = get_git_sha()
    generated_at = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S UTC")

    print(f"Parsing results from: {results_dir}")
    experiment_results = build_results(results_dir)

    if not experiment_results:
        print("WARNING: No experiment log files found. "
              "Expected files named exp01-operator-logs.txt, etc.", file=sys.stderr)
        return 1

    passed = sum(1 for r in experiment_results if r.passed)
    failed = sum(1 for r in experiment_results if not r.passed)
    warned = sum(1 for r in experiment_results if r.passed and r.warnings)

    summary = ReportSummary(
        run_id=run_id,
        generated_at=generated_at,
        git_sha=git_sha,
        total_experiments=len(experiment_results),
        passed=passed,
        failed=failed,
        warned=warned,
        overall_score=overall_score(experiment_results),
        results=experiment_results,
    )

    # Write outputs
    if args.output_format in ("markdown", "both"):
        md_path = results_dir / "resilience-report.md"
        md_path.write_text(render_markdown(summary))
        print(f"Markdown report: {md_path}")

    if args.output_format in ("json", "both"):
        json_path = results_dir / "resilience-report.json"
        json_path.write_text(render_json(summary))
        print(f"JSON report:     {json_path}")

    write_github_summary(summary, args.github_summary)

    # Print summary to stdout
    emoji = score_emoji(summary.overall_score)
    print(f"\n{emoji} Overall Resilience Score: {summary.overall_score:.1f}/100")
    print(f"   Passed: {passed}  Failed: {failed}  Warned: {warned}")

    # Exit non-zero if any experiment failed
    return 1 if failed > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
