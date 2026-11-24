#!/usr/bin/env python3
"""Unit tests for scripts/crd_migration_lint.py (issue #1065)."""

import importlib.util
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "crd_migration_lint",
    Path(__file__).resolve().parent.parent / "crd_migration_lint.py",
)
lint = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(lint)


def _crd(properties, required=None, version="v1", served=True):
    return {
        "metadata": {"name": "widgets.stellar.example.com"},
        "spec": {
            "versions": [
                {
                    "name": version,
                    "served": served,
                    "schema": {
                        "openAPIV3Schema": {
                            "type": "object",
                            "properties": properties,
                            "required": required or [],
                        }
                    },
                }
            ]
        },
    }


class CompareCrdsTest(unittest.TestCase):
    def test_identical_crds_pass(self):
        crd = _crd({"replicas": {"type": "integer"}})
        self.assertEqual(lint.compare_crds(crd, crd), [])

    def test_added_optional_field_passes(self):
        old = _crd({"replicas": {"type": "integer"}})
        new = _crd({"replicas": {"type": "integer"}, "image": {"type": "string"}})
        self.assertEqual(lint.compare_crds(old, new), [])

    def test_removed_property_flagged(self):
        old = _crd({"replicas": {"type": "integer"}})
        new = _crd({})
        problems = lint.compare_crds(old, new)
        self.assertTrue(any("removed" in p for p in problems))

    def test_type_change_flagged(self):
        old = _crd({"replicas": {"type": "integer"}})
        new = _crd({"replicas": {"type": "string"}})
        problems = lint.compare_crds(old, new)
        self.assertTrue(any("changed type" in p for p in problems))

    def test_newly_required_existing_field_flagged(self):
        old = _crd({"replicas": {"type": "integer"}})
        new = _crd({"replicas": {"type": "integer"}}, required=["replicas"])
        problems = lint.compare_crds(old, new)
        self.assertTrue(any("became required" in p for p in problems))

    def test_removed_served_version_flagged(self):
        old = _crd({"replicas": {"type": "integer"}}, version="v1alpha1")
        new = _crd({"replicas": {"type": "integer"}}, version="v1")
        problems = lint.compare_crds(old, new)
        self.assertTrue(any("served version 'v1alpha1' was removed" in p for p in problems))

    def test_nested_property_removal_flagged(self):
        old = _crd({"spec": {"type": "object", "properties": {"size": {"type": "integer"}}}})
        new = _crd({"spec": {"type": "object", "properties": {}}})
        problems = lint.compare_crds(old, new)
        self.assertTrue(any("spec.size" in p for p in problems))


if __name__ == "__main__":
    unittest.main()
