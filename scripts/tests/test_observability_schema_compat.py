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
"""Tests for the observability schema compatibility gate (issue #1960)."""

from __future__ import annotations

import copy
import importlib.util
import json
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "schema_compat",
    Path(__file__).resolve().parents[1] / "ci" / "check-observability-schema-compat.py",
)
compat = importlib.util.module_from_spec(_SPEC)
assert _SPEC.loader is not None
_SPEC.loader.exec_module(compat)

SCHEMA = Path(__file__).resolve().parents[2] / "schemas" / "observability" / "resource-attributes.v1.json"
BASE = json.loads(SCHEMA.read_text(encoding="utf-8"))


def mutate(fn, version=None, compatible_with=None, min_compat=None):
    s = copy.deepcopy(BASE)
    fn(s)
    if version:
        s["contractVersion"] = version
    if compatible_with is not None:
        s["compatibleWith"] = compatible_with
    if min_compat:
        s["minCompatibleVersion"] = min_compat
    return s


class SchemaCompat(unittest.TestCase):
    def test_identical_schema_is_compatible(self):
        self.assertEqual(compat.check(BASE, BASE), [])

    def test_removing_an_attribute_requires_major_bump(self):
        new = mutate(lambda s: s["properties"].pop("host.name"), "1.1.0", ["1.0.0", "1.1.0"])
        errs = " ".join(compat.check(BASE, new))
        self.assertIn("MAJOR", errs)
        self.assertIn("host.name", errs)

    def test_major_bump_for_breaking_change_is_accepted_with_min_compat(self):
        new = mutate(lambda s: s["properties"].pop("host.name"), "2.0.0", ["2.0.0"], "2.0.0")
        self.assertEqual(compat.check(BASE, new), [])

    def test_major_bump_must_raise_min_compatible(self):
        new = mutate(lambda s: s["properties"].pop("host.name"), "2.0.0", ["2.0.0"])
        self.assertTrue(any("minCompatibleVersion" in e for e in compat.check(BASE, new)))

    def test_new_required_attribute_is_breaking(self):
        def f(s):
            s["properties"]["new.attr"] = {"type": "string"}
            s["required"].append("new.attr")

        errs = " ".join(compat.check(BASE, mutate(f, "1.1.0", ["1.0.0", "1.1.0"])))
        self.assertIn("required attribute 'new.attr'", errs)

    def test_optional_attribute_needs_version_bump_and_compat_list(self):
        def f(s):
            s["properties"]["new.attr"] = {"type": "string", "requirement": "optional"}

        self.assertTrue(any("not increased" in e for e in compat.check(BASE, mutate(f))))
        self.assertEqual(compat.check(BASE, mutate(f, "1.1.0", ["1.0.0", "1.1.0"])), [])
        self.assertTrue(any("compatibleWith" in e for e in compat.check(BASE, mutate(f, "1.1.0", ["1.1.0"]))))

    def test_patch_bump_cannot_add_attributes(self):
        def f(s):
            s["properties"]["new.attr"] = {"type": "string", "requirement": "optional"}

        errs = " ".join(compat.check(BASE, mutate(f, "1.0.1", ["1.0.0", "1.0.1"])))
        self.assertIn("MINOR", errs)

    def test_tightening_constraints_is_breaking(self):
        def tighten_enum(s):
            s["properties"]["deployment.environment"]["enum"] = ["production"]

        def lower_max(s):
            s["properties"]["service.name"]["maxLength"] = 10

        def retype(s):
            s["properties"]["service.version"]["type"] = "integer"

        for fn in (tighten_enum, lower_max, retype):
            errs = compat.check(BASE, mutate(fn, "1.1.0", ["1.0.0", "1.1.0"]))
            self.assertTrue(any("MAJOR" in e for e in errs), (fn.__name__, errs))

    def test_widening_enum_is_minor(self):
        def f(s):
            s["properties"]["deployment.environment"]["enum"].append("canary")

        self.assertEqual(compat.check(BASE, mutate(f, "1.1.0", ["1.0.0", "1.1.0"])), [])

    def test_version_cannot_go_backwards(self):
        old = mutate(lambda s: None, "1.2.0", ["1.2.0"])
        self.assertTrue(any("backwards" in e for e in compat.check(old, BASE)))


if __name__ == "__main__":
    unittest.main()
