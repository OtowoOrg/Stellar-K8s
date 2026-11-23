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
"""Render the OTel collector template and check contract/dead-letter wiring (issue #1960)."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
CHART = ROOT / "charts" / "stellar-operator"


def render(*sets: str) -> list[dict]:
    # Render only the collector template: other chart templates are out of scope here.
    with tempfile.TemporaryDirectory() as tmp:
        c = Path(tmp) / "chart"
        (c / "templates").mkdir(parents=True)
        for f in ("Chart.yaml", "values.yaml"):
            shutil.copy(CHART / f, c / f)
        for f in ("_helpers.tpl", "otel-collector.yaml"):
            shutil.copy(CHART / "templates" / f, c / "templates" / f)
        args = ["helm", "template", "t", str(c), "--set", "otel.collector.enabled=true"]
        for s in sets:
            args += ["--set", s]
        out = subprocess.run(args, check=True, capture_output=True, text=True).stdout
    return [d for d in yaml.safe_load_all(out) if d]


@unittest.skipUnless(shutil.which("helm"), "helm not installed")
class CollectorContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        docs = render("otel.collector.config.loki.enabled=true", "otel.collector.config.tempo.enabled=true")
        cls.docs = docs
        cm = next(d for d in docs if d["kind"] == "ConfigMap")
        cls.cfg = yaml.safe_load(cm["data"]["config.yaml"])

    def test_default_values_render(self):
        render()  # regression: loki/elasticsearch defaults used to be missing

    def test_every_pipeline_reference_resolves(self):
        cfg = self.cfg
        for name, p in cfg["service"]["pipelines"].items():
            for r in p.get("receivers", []):
                self.assertTrue(r in cfg["receivers"] or r in cfg.get("connectors", {}), (name, r))
            for e in p["exporters"]:
                self.assertTrue(e in cfg["exporters"] or e in cfg.get("connectors", {}), (name, e))
            for pr in p.get("processors", []):
                self.assertIn(pr, cfg["processors"], (name, pr))

    def test_invalid_signals_go_to_dead_letter_and_are_counted(self):
        pl = self.cfg["service"]["pipelines"]
        for sig in ("traces", "metrics", "logs"):
            dl = pl[f"{sig}/deadletter"]
            self.assertIn("filter/keep_invalid", dl["processors"])
            self.assertIn("file/deadletter", dl["exporters"])
            self.assertIn("count/deadletter", dl["exporters"], f"{sig} dead letters are not counted")
            self.assertNotIn("file/deadletter", pl[sig]["exporters"])
            self.assertIn("filter/drop_invalid", pl[sig]["processors"])
        counts = pl["metrics/deadletter-counts"]
        self.assertEqual(counts["receivers"], ["count/deadletter"])
        self.assertEqual(counts["exporters"], ["prometheus/deadletter"])

    def test_dead_letter_metrics_port_exposed_on_pod_and_service(self):
        dep = next(d for d in self.docs if d["kind"] == "Deployment")
        ports = {p["name"]: p["containerPort"] for p in dep["spec"]["template"]["spec"]["containers"][0]["ports"]}
        self.assertEqual(ports["dl-metrics"], 8889)
        self.assertTrue(all(len(n) <= 15 for n in ports), "container port names must be <= 15 chars")
        svc = next(d for d in self.docs if d["kind"] == "Service")
        self.assertIn("dl-metrics", [p["name"] for p in svc["spec"]["ports"]])

    def test_loki_exporter_is_in_the_logs_pipeline_when_enabled(self):
        self.assertIn("loki", self.cfg["service"]["pipelines"]["logs"]["exporters"])
        self.assertIn("otlp/tempo", self.cfg["service"]["pipelines"]["traces"]["exporters"])


if __name__ == "__main__":
    unittest.main()
