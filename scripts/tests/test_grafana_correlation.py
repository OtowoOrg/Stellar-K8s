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
"""Grafana log<->trace correlation provisioning tests (issue #1960)."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
DS = ROOT / "config" / "monitoring" / "grafana" / "provisioning" / "datasources" / "prometheus.yaml"
SCHEMA = json.loads((ROOT / "schemas" / "observability" / "resource-attributes.v1.json").read_text())


def datasources():
    return {d["name"]: d for d in yaml.safe_load(DS.read_text())["datasources"]}


class GrafanaCorrelation(unittest.TestCase):
    def test_existing_datasources_are_preserved(self):
        ds = datasources()
        self.assertEqual(ds["Prometheus"]["url"], "http://prometheus:9090")
        self.assertTrue(ds["Prometheus"]["isDefault"])
        self.assertEqual(ds["Loki"]["url"], "http://loki:3100")

    def test_loki_derived_field_points_at_tempo(self):
        ds = datasources()
        field = ds["Loki"]["jsonData"]["derivedFields"][0]
        self.assertEqual(field["datasourceUid"], ds["Tempo"]["uid"])
        # Single click: the link is the captured trace id itself (internal link, no copy/paste).
        self.assertEqual(field["url"], "$${__value.raw}")

    def test_derived_field_regex_extracts_trace_id_from_common_log_formats(self):
        rx = re.compile(datasources()["Loki"]["jsonData"]["derivedFields"][0]["matcherRegex"])
        tid = "4bf92f3577b34da6a3ce929d0e0e4736"
        for line in (
            f'{{"level":"info","trace_id":"{tid}","msg":"x"}}',
            f"level=info trace_id={tid} msg=x",
            f'{{"traceId": "{tid}"}}',
        ):
            m = rx.search(line)
            self.assertIsNotNone(m, line)
            self.assertEqual(m.group(1), tid)
        self.assertIsNone(rx.search('{"trace_id":"short"}'))

    def test_tempo_links_back_to_loki_with_contract_attributes(self):
        ds = datasources()
        t2l = ds["Tempo"]["jsonData"]["tracesToLogsV2"]
        self.assertEqual(t2l["datasourceUid"], ds["Loki"]["uid"])
        self.assertTrue(t2l["filterByTraceID"])
        allowed = set(SCHEMA["properties"])
        for tag in t2l["tags"]:
            self.assertIn(tag["key"], allowed, f"tracesToLogs tag {tag['key']} is not a contract attribute")
        self.assertEqual(ds["Tempo"]["jsonData"]["tracesToMetrics"]["datasourceUid"], ds["Prometheus"]["uid"])

    def test_datasource_uids_are_unique_and_explicit(self):
        uids = [d["uid"] for d in datasources().values() if "uid" in d]
        self.assertEqual(len(uids), len(set(uids)))
        for name in ("Prometheus", "Loki", "Tempo"):
            self.assertIn("uid", datasources()[name])

    def test_schema_declares_the_log_to_trace_field(self):
        self.assertEqual(SCHEMA["x-grafanaCorrelation"]["logToTraceField"], "trace_id")


if __name__ == "__main__":
    unittest.main()
