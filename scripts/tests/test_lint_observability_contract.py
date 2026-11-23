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
"""Tests for the observability contract CI lint (issue #1481)."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "lint_observability_contract",
    Path(__file__).resolve().parents[1] / "ci" / "lint-observability-contract.py",
)
lint = importlib.util.module_from_spec(_SPEC)
assert _SPEC.loader is not None
_SPEC.loader.exec_module(lint)

SCHEMA = (
    Path(__file__).resolve().parents[2]
    / "schemas"
    / "observability"
    / "resource-attributes.v1.json"
)


class LintObservabilityContract(unittest.TestCase):
    def test_repo_scan_passes(self):
        self.assertEqual(lint.main([]), 0)

    def test_unknown_attribute_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "src"
            src.mkdir()
            (src / "rogue.rs").write_text(
                'let _ = KeyValue::new("k8s.unknown.attr", "nope");\n',
                encoding="utf-8",
            )
            violations = lint.lint(SCHEMA, [root])
            self.assertTrue(
                any("k8s.unknown.attr" in row for row in violations),
                violations,
            )

    def test_catalogued_attribute_is_allowed(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "src"
            src.mkdir()
            (src / "ok.rs").write_text(
                'let _ = KeyValue::new("k8s.pod.name", "pod");\n',
                encoding="utf-8",
            )
            self.assertEqual(lint.lint(SCHEMA, [root]), [])

    # ── #1960: fixtures, validator, collector drift ─────────────────────

    def _good(self):
        return {
            "service.name": "svc",
            "service.instance.id": "abc-123",
            "k8s.pod.name": "svc-0",
            "k8s.namespace.name": "ns",
            "k8s.node.name": "node-1",
            "stellar.observability.contract.version": "1.0.0",
        }

    def test_validator_names_the_offending_attribute(self):
        schema = lint.load_schema(SCHEMA)
        self.assertEqual(lint.validate_resource(self._good(), schema), [])
        r = self._good()
        del r["service.name"]
        del r["stellar.observability.contract.version"]  # stamped at ingest: not an error
        r["bogus.attr"] = "x"
        r["deployment.environment"] = "prod"
        r["k8s.pod.name"] = 5
        errs = " | ".join(lint.validate_resource(r, schema))
        self.assertIn("missing required attribute 'service.name'", errs)
        self.assertNotIn("contract.version", errs)
        self.assertIn("unknown attribute 'bogus.attr'", errs)
        self.assertIn("attribute 'deployment.environment'", errs)
        self.assertIn("attribute 'k8s.pod.name': expected string", errs)

    def test_repo_fixtures_pass_and_cover_each_failure_class(self):
        fixtures = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "observability"
        self.assertEqual(lint.lint_fixtures(SCHEMA, fixtures), [])
        names = {p.name for p in fixtures.glob("*.json")}
        self.assertTrue(any(n.startswith("invalid-missing-") for n in names))
        self.assertIn("valid-version-stamped-at-ingest.json", names)
        self.assertIn("invalid-unknown-attribute.json", names)
        self.assertIn("invalid-bad-environment.json", names)

    def test_fixture_with_wrong_expectation_fails(self):
        import json

        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "lies.json").write_text(
                json.dumps({"expect": "valid", "resource": {"bogus": "x"}}), encoding="utf-8"
            )
            (d / "also-lies.json").write_text(
                json.dumps({"expect": "invalid", "resource": self._good()}), encoding="utf-8"
            )
            out = lint.lint_fixtures(SCHEMA, d)
            self.assertEqual(len(out), 2, out)
            self.assertTrue(any("lies.json" in o and "bogus" in o for o in out))

    def test_empty_fixture_dir_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertTrue(lint.lint_fixtures(SCHEMA, Path(tmp)))

    def test_collector_template_matches_schema_required_set(self):
        template = (
            Path(__file__).resolve().parents[2]
            / "charts" / "stellar-operator" / "templates" / "otel-collector.yaml"
        )
        self.assertEqual(lint.lint_collector(SCHEMA, template), [])

    def test_collector_drift_is_detected(self):
        with tempfile.TemporaryDirectory() as tmp:
            t = Path(tmp) / "c.yaml"
            t.write_text(
                "- 'resource.attributes[\"service.name\"] == nil or resource.attributes[\"k8s.pod.name\"] == nil'\n"
                '- set(attributes["stellar.observability.contract.version"], "0.9.0") where x\n',
                encoding="utf-8",
            )
            out = " ".join(lint.lint_collector(SCHEMA, t))
            self.assertIn("missing", out)
            self.assertIn("0.9.0", out)


if __name__ == "__main__":
    unittest.main()
