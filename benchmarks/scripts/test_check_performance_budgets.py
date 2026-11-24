#!/usr/bin/env python3
"""Unit tests for benchmarks/scripts/check_performance_budgets.py (issue #1336).

The evaluator is a merge gate, so the cases that matter most are the ones where
it could silently pass: a metric missing from the summary, an advisory budget
being mistaken for a blocking one, or a comparison direction inverted for
throughput-style metrics.
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_ROOT = Path(__file__).resolve().parent
_SPEC = importlib.util.spec_from_file_location(
    "check_performance_budgets", _ROOT / "check_performance_budgets.py"
)
budgets_mod = importlib.util.module_from_spec(_SPEC)
sys.modules["check_performance_budgets"] = budgets_mod
_SPEC.loader.exec_module(budgets_mod)

REPO_ROOT = _ROOT.parent.parent
BUDGETS_FILE = REPO_ROOT / "benchmarks" / "performance-budgets.yaml"


def budget(metric, comparison="lt", value=100.0, blocking=True):
    return budgets_mod.Budget(
        metric=metric, comparison=comparison, budget=value, blocking=blocking
    )


def run(document, budget_list, suite="operator"):
    return budgets_mod.evaluate(document, budget_list, suite)


class MetricLookupTest(unittest.TestCase):
    DOC = {"metrics": {"http_req_duration": {"p95": 300}}, "error_rate": 0.01}

    def test_dotted_path_resolves(self):
        self.assertEqual(
            budgets_mod.read_metric(self.DOC, "metrics.http_req_duration.p95"), 300.0
        )

    def test_top_level_path_resolves(self):
        self.assertEqual(budgets_mod.read_metric(self.DOC, "error_rate"), 0.01)

    def test_missing_path_returns_none(self):
        self.assertIsNone(budgets_mod.read_metric(self.DOC, "metrics.nope.p95"))

    def test_partial_path_into_a_scalar_returns_none(self):
        self.assertIsNone(budgets_mod.read_metric(self.DOC, "error_rate.p95"))

    def test_non_numeric_value_returns_none(self):
        self.assertIsNone(budgets_mod.read_metric({"a": "fast"}, "a"))

    def test_booleans_are_not_treated_as_numbers(self):
        # bool is a subclass of int; a `true` must not read as 1.0.
        self.assertIsNone(budgets_mod.read_metric({"a": True}, "a"))

    def test_zero_is_a_valid_measurement(self):
        self.assertEqual(budgets_mod.read_metric({"a": 0}, "a"), 0.0)


class ComparisonTest(unittest.TestCase):
    def test_lt_passes_below_budget(self):
        report = run({"a": 99}, [budget("a", "lt", 100)])
        self.assertTrue(report.passed)

    def test_lt_fails_at_the_budget(self):
        self.assertFalse(run({"a": 100}, [budget("a", "lt", 100)]).passed)

    def test_lte_passes_at_the_budget(self):
        self.assertTrue(run({"a": 100}, [budget("a", "lte", 100)]).passed)

    def test_gte_enforces_a_throughput_floor(self):
        # Direction matters: for req/s, bigger is better.
        self.assertTrue(run({"a": 150}, [budget("a", "gte", 100)]).passed)
        self.assertFalse(run({"a": 42}, [budget("a", "gte", 100)]).passed)

    def test_gt_is_strict(self):
        self.assertFalse(run({"a": 100}, [budget("a", "gt", 100)]).passed)

    def test_unknown_comparison_is_rejected_at_load_time(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "b.yaml"
            path.write_text(
                "suites:\n  s:\n    budgets:\n"
                "      - metric: a\n        comparison: approximately\n        budget: 1\n"
            )
            with self.assertRaises(SystemExit):
                budgets_mod.load_budgets(path, "s")


class BlockingTest(unittest.TestCase):
    def test_a_blocking_violation_fails_the_report(self):
        report = run({"a": 500}, [budget("a", "lt", 100, blocking=True)])
        self.assertFalse(report.passed)
        self.assertEqual(len(report.blocking_violations), 1)

    def test_an_advisory_violation_is_reported_but_passes(self):
        report = run({"a": 500}, [budget("a", "lt", 100, blocking=False)])
        self.assertTrue(report.passed)
        self.assertEqual(len(report.violations), 1)
        self.assertEqual(len(report.blocking_violations), 0)

    def test_a_missing_blocking_metric_fails(self):
        # The important one: a metric that vanishes from the summary must not
        # quietly turn the gate into a no-op.
        report = run({}, [budget("gone", "lt", 100, blocking=True)])
        self.assertFalse(report.passed)
        self.assertEqual(len(report.missing), 1)
        self.assertEqual(len(report.violations), 0, "missing is not a violation")

    def test_a_missing_advisory_metric_passes(self):
        self.assertTrue(run({}, [budget("gone", "lt", 100, blocking=False)]).passed)

    def test_mixed_results_fail_on_the_blocking_one(self):
        report = run(
            {"a": 500, "b": 1},
            [budget("a", "lt", 100, blocking=True), budget("b", "lt", 100, blocking=False)],
        )
        self.assertFalse(report.passed)


class HeadroomTest(unittest.TestCase):
    def test_headroom_is_positive_when_under_a_ceiling(self):
        report = run({"a": 60}, [budget("a", "lt", 100)])
        self.assertAlmostEqual(report.results[0].headroom_percent(), 40.0)

    def test_headroom_is_negative_when_over_a_ceiling(self):
        report = run({"a": 140}, [budget("a", "lt", 100)])
        self.assertAlmostEqual(report.results[0].headroom_percent(), -40.0)

    def test_headroom_inverts_for_a_floor(self):
        report = run({"a": 150}, [budget("a", "gte", 100)])
        self.assertAlmostEqual(report.results[0].headroom_percent(), 50.0)

    def test_headroom_is_none_for_a_missing_metric(self):
        report = run({}, [budget("gone")])
        self.assertIsNone(report.results[0].headroom_percent())


class BudgetsFileTest(unittest.TestCase):
    """The repository's own budgets must stay loadable and sane."""

    def test_operator_suite_loads(self):
        loaded, description = budgets_mod.load_budgets(BUDGETS_FILE, "operator")
        self.assertTrue(loaded)
        self.assertTrue(description)

    def test_webhook_suite_loads(self):
        loaded, _ = budgets_mod.load_budgets(BUDGETS_FILE, "webhook")
        self.assertTrue(loaded)

    def test_an_unknown_suite_exits(self):
        with self.assertRaises(SystemExit):
            budgets_mod.load_budgets(BUDGETS_FILE, "no-such-suite")

    def test_every_budget_declares_a_description(self):
        for suite in ("operator", "webhook"):
            loaded, _ = budgets_mod.load_budgets(BUDGETS_FILE, suite)
            for entry in loaded:
                self.assertTrue(
                    entry.description.strip(),
                    f"{suite}/{entry.metric} has no description explaining the target",
                )

    def test_webhook_latency_budget_is_tighter_than_the_operator_one(self):
        # The webhook sits in the API server's synchronous admission path, so
        # its budget must never be loosened to match the operator's.
        webhook, _ = budgets_mod.load_budgets(BUDGETS_FILE, "webhook")
        operator, _ = budgets_mod.load_budgets(BUDGETS_FILE, "operator")
        webhook_p99 = next(b for b in webhook if b.metric.endswith("validation_p99"))
        operator_p99 = next(
            b for b in operator if b.metric == "metrics.http_req_duration.p99"
        )
        self.assertLess(webhook_p99.budget, operator_p99.budget)

    def test_error_rate_budgets_are_ratios_not_percentages(self):
        for suite in ("operator", "webhook"):
            loaded, _ = budgets_mod.load_budgets(BUDGETS_FILE, suite)
            for entry in loaded:
                if entry.metric.endswith("error_rate"):
                    self.assertLess(
                        entry.budget, 1.0, f"{suite}/{entry.metric} looks like a percentage"
                    )


class ReportingTest(unittest.TestCase):
    def _report(self):
        return run(
            {"timestamp": "2026-01-01T00:00:00Z", "a": 500, "b": 10},
            [budget("a", "lt", 100), budget("b", "lt", 100), budget("gone")],
        )

    def test_markdown_marks_pass_fail_and_missing(self):
        markdown = budgets_mod.render_markdown(self._report(), "suite description")
        self.assertIn("budget exceeded", markdown)
        self.assertIn("suite description", markdown)
        self.assertIn("❌", markdown)
        self.assertIn("✅", markdown)
        self.assertIn("Missing metrics", markdown)
        self.assertIn("### Violations", markdown)

    def test_markdown_reports_a_clean_run_as_within_budget(self):
        markdown = budgets_mod.render_markdown(run({"a": 1}, [budget("a", "lt", 100)]), "")
        self.assertIn("within budget", markdown)

    def test_text_report_lists_every_budget(self):
        text = budgets_mod.render_text(self._report())
        self.assertIn("PASS", text)
        self.assertIn("FAIL", text)
        self.assertIn("3 evaluated", text)

    def test_json_report_round_trips(self):
        payload = json.loads(json.dumps(self._report().as_dict()))
        self.assertEqual(payload["evaluated"], 3)
        self.assertEqual(payload["blocking_violations"], 1)
        self.assertEqual(payload["missing_metrics"], 1)
        self.assertFalse(payload["passed"])

    def test_run_metadata_is_carried_into_the_report(self):
        report = run({"timestamp": "T", "runId": "r", "a": 1}, [budget("a", "lt", 100)])
        self.assertEqual(report.run_metadata["timestamp"], "T")
        self.assertEqual(report.run_metadata["runId"], "r")


class HistoryTest(unittest.TestCase):
    def test_history_appends_one_line_per_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            for _ in range(3):
                report = run({"a": 5}, [budget("a", "lt", 100)])
                path = budgets_mod.append_history(report, directory)
            lines = path.read_text().strip().splitlines()
            self.assertEqual(len(lines), 3)
            entry = json.loads(lines[0])
            self.assertTrue(entry["passed"])
            self.assertEqual(entry["metrics"]["a"], 5.0)
            self.assertIn("recorded_at", entry)

    def test_history_is_kept_per_suite(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            budgets_mod.append_history(run({"a": 1}, [budget("a")], "operator"), directory)
            budgets_mod.append_history(run({"a": 1}, [budget("a")], "webhook"), directory)
            self.assertTrue((directory / "operator.jsonl").is_file())
            self.assertTrue((directory / "webhook.jsonl").is_file())


class CliTest(unittest.TestCase):
    def _results_file(self, tmp: Path, document: dict) -> Path:
        path = tmp / "results.json"
        path.write_text(json.dumps(document))
        return path

    PASSING = {
        "metrics": {
            "tps": {"avg": 150},
            "http_req_duration": {"p95": 300, "p99": 800},
            "reconciliation_duration": {"p95": 2000, "p99": 4000},
            "api_latency": {"p95": 120},
            "error_rate": 0.002,
        }
    }

    def test_a_passing_run_exits_zero(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self._results_file(Path(tmp), self.PASSING)
            code = budgets_mod.main([
                "--results", str(path), "--suite", "operator",
                "--budgets", str(BUDGETS_FILE), "--format", "json",
            ])
            self.assertEqual(code, 0)

    def test_a_violating_run_exits_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            document = json.loads(json.dumps(self.PASSING))
            document["metrics"]["error_rate"] = 0.5
            path = self._results_file(Path(tmp), document)
            code = budgets_mod.main([
                "--results", str(path), "--suite", "operator",
                "--budgets", str(BUDGETS_FILE), "--format", "json",
            ])
            self.assertEqual(code, 1)

    def test_warn_only_exits_zero_despite_violations(self):
        with tempfile.TemporaryDirectory() as tmp:
            document = json.loads(json.dumps(self.PASSING))
            document["metrics"]["error_rate"] = 0.5
            path = self._results_file(Path(tmp), document)
            code = budgets_mod.main([
                "--results", str(path), "--suite", "operator",
                "--budgets", str(BUDGETS_FILE), "--format", "json", "--warn-only",
            ])
            self.assertEqual(code, 0)

    def test_a_missing_results_file_exits_two(self):
        with self.assertRaises(SystemExit) as caught:
            budgets_mod.main(["--results", "/nonexistent.json", "--suite", "operator"])
        self.assertNotEqual(caught.exception.code, 0)

    def test_malformed_results_json_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "bad.json"
            path.write_text("{not json")
            with self.assertRaises(SystemExit):
                budgets_mod.main([
                    "--results", str(path), "--suite", "operator",
                    "--budgets", str(BUDGETS_FILE),
                ])

    def test_markdown_is_appended_to_the_named_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            path = self._results_file(tmp_path, self.PASSING)
            summary = tmp_path / "summary.md"
            budgets_mod.main([
                "--results", str(path), "--suite", "operator",
                "--budgets", str(BUDGETS_FILE), "--format", "json",
                "--markdown", str(summary),
            ])
            self.assertIn("Performance budgets", summary.read_text())


if __name__ == "__main__":
    unittest.main()
