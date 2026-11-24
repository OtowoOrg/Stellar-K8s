#!/usr/bin/env python3
"""Unit tests for scripts/validate-yaml-manifests.py (issue #1044).

Covers each validation layer independently, plus the two mechanisms that
keep the gate from rotting: negative fixtures must keep failing, and a
known-deviation waiver that stops matching must be reported.
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "validate_yaml_manifests",
    Path(__file__).resolve().parent.parent / "validate-yaml-manifests.py",
)
val = importlib.util.module_from_spec(_SPEC)
sys.modules["validate_yaml_manifests"] = val
_SPEC.loader.exec_module(val)


def write(tmp: Path, name: str, content: str) -> Path:
    path = tmp / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    return path


def layers(report) -> set:
    return {i.layer for i in report.issues}


def messages(report) -> str:
    return "\n".join(i.message for i in report.issues)


class SyntaxLayerTest(unittest.TestCase):
    def test_valid_yaml_produces_no_syntax_issues(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "ok.yaml", "a: 1\nb: two\n")
            _, issues = val.load_documents(path)
            self.assertEqual(issues, [])

    def test_duplicate_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "dup.yaml", "a: 1\nb: 2\na: 3\n")
            _, issues = val.load_documents(path)
            self.assertTrue(issues)
            self.assertIn("duplicate key", issues[0].message)

    def test_tab_indentation_is_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "tab.yaml", "a:\n\tb: 1\n")
            _, issues = val.load_documents(path)
            self.assertTrue(any("tab" in i.message for i in issues))

    def test_parse_error_records_a_line_number(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "bad.yaml", "a: 1\nb: [unclosed\n")
            _, issues = val.load_documents(path)
            self.assertTrue(issues)
            self.assertIsNotNone(issues[0].line)

    def test_multi_document_files_are_all_returned(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "multi.yaml", "a: 1\n---\nb: 2\n---\nc: 3\n")
            docs, issues = val.load_documents(path)
            self.assertEqual(issues, [])
            self.assertEqual(len(docs), 3)


class StructureLayerTest(unittest.TestCase):
    BASE = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "good-name"}}

    def check(self, doc):
        return val.validate_structure("f.yaml", doc, 0)

    def test_well_formed_object_passes(self):
        self.assertEqual(self.check(self.BASE), [])

    def test_missing_metadata_is_reported(self):
        self.assertTrue(self.check({"apiVersion": "v1", "kind": "ConfigMap"}))

    def test_missing_name_is_reported(self):
        doc = {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {}}
        self.assertIn("metadata.name", self.check(doc)[0].message)

    def test_generate_name_satisfies_the_name_requirement(self):
        doc = {"apiVersion": "v1", "kind": "Pod", "metadata": {"generateName": "p-"}}
        self.assertEqual(self.check(doc), [])

    def test_uppercase_name_is_rejected(self):
        doc = {**self.BASE, "metadata": {"name": "BadName"}}
        self.assertIn("DNS-1123", self.check(doc)[0].message)

    def test_overlong_name_is_rejected(self):
        doc = {**self.BASE, "metadata": {"name": "a" * 254}}
        self.assertIn("253", self.check(doc)[0].message)

    def test_malformed_api_version_is_rejected(self):
        self.assertTrue(self.check({**self.BASE, "apiVersion": "not a version"}))

    def test_grouped_api_version_is_accepted(self):
        self.assertEqual(self.check({**self.BASE, "apiVersion": "stellar.org/v1alpha1"}), [])

    def test_lowercase_kind_is_rejected(self):
        self.assertTrue(self.check({**self.BASE, "kind": "configMap"}))

    def test_invalid_namespace_is_rejected(self):
        doc = {**self.BASE, "metadata": {"name": "n", "namespace": "Bad_NS"}}
        self.assertTrue(self.check(doc))

    def test_non_string_label_value_is_rejected(self):
        doc = {**self.BASE, "metadata": {"name": "n", "labels": {"tier": 3}}}
        self.assertIn("must be a string", self.check(doc)[0].message)

    def test_invalid_label_value_is_rejected(self):
        doc = {**self.BASE, "metadata": {"name": "n", "labels": {"tier": "has spaces"}}}
        self.assertTrue(self.check(doc))

    def test_prefixed_label_key_is_accepted(self):
        doc = {**self.BASE, "metadata": {"name": "n", "labels": {"stellar.org/role": "validator"}}}
        self.assertEqual(self.check(doc), [])

    def test_annotations_may_hold_free_form_strings(self):
        doc = {**self.BASE, "metadata": {"name": "n", "annotations": {"a/b": "any value here!"}}}
        self.assertEqual(self.check(doc), [])


class OpenApiConversionTest(unittest.TestCase):
    def test_int_or_string_becomes_a_union_type(self):
        out = val.openapi_to_jsonschema({"x-kubernetes-int-or-string": True})
        self.assertEqual(out["type"], ["integer", "string"])

    def test_preserve_unknown_fields_opens_the_object(self):
        out = val.openapi_to_jsonschema({"type": "object", "x-kubernetes-preserve-unknown-fields": True})
        self.assertTrue(out["additionalProperties"])

    def test_list_type_extensions_are_dropped(self):
        out = val.openapi_to_jsonschema({"type": "array", "x-kubernetes-list-type": "map"})
        self.assertNotIn("x-kubernetes-list-type", out)

    def test_nullable_widens_the_type(self):
        out = val.openapi_to_jsonschema({"type": "string", "nullable": True})
        self.assertEqual(out["type"], ["string", "null"])

    def test_nested_properties_are_converted(self):
        out = val.openapi_to_jsonschema(
            {"type": "object", "properties": {"n": {"x-kubernetes-int-or-string": True}}}
        )
        self.assertEqual(out["properties"]["n"]["type"], ["integer", "string"])

    def test_array_items_are_converted(self):
        out = val.openapi_to_jsonschema({"type": "array", "items": {"x-kubernetes-int-or-string": True}})
        self.assertEqual(out["items"]["type"], ["integer", "string"])


class CrdSchemaTest(unittest.TestCase):
    def test_repository_crds_load_without_error(self):
        index, issues = val.load_crd_schemas(val.CRD_DIR)
        self.assertEqual(issues, [])
        self.assertTrue(index, "expected at least one CRD schema")

    def test_stellarnode_schema_is_indexed(self):
        index, _ = val.load_crd_schemas(val.CRD_DIR)
        self.assertIn(("stellar.org/v1alpha1", "StellarNode"), index)

    def test_custom_resource_is_checked_against_its_crd(self):
        index, _ = val.load_crd_schemas(val.CRD_DIR)
        crd = index[("stellar.org/v1alpha1", "StellarNode")]
        doc = {
            "apiVersion": "stellar.org/v1alpha1",
            "kind": "StellarNode",
            "metadata": {"name": "n"},
            "spec": {"nodeType": "NotARealType", "network": "testnet", "version": "v21.0.0"},
        }
        issues = val.validate_against_schema("f.yaml", doc, crd.schema, 0, "L3-schema")
        self.assertTrue(any("nodeType" in i.message for i in issues))

    def test_unserved_versions_are_not_indexed(self):
        with tempfile.TemporaryDirectory() as tmp:
            write(
                Path(tmp),
                "crd.yaml",
                "apiVersion: apiextensions.k8s.io/v1\n"
                "kind: CustomResourceDefinition\n"
                "metadata:\n  name: widgets.example.com\n"
                "spec:\n  group: example.com\n  names:\n    kind: Widget\n"
                "  versions:\n    - name: v1\n      served: false\n"
                "      schema:\n        openAPIV3Schema:\n          type: object\n",
            )
            index, _ = val.load_crd_schemas(Path(tmp))
            self.assertEqual(index, {})


class NegativeFixtureTest(unittest.TestCase):
    def test_fixture_that_fails_is_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(
                Path(tmp),
                "invalid-x.yaml",
                "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: BAD_NAME\n",
            )
            config = val.Config(expect_invalid=[str(path)])
            report = val.validate([path], config)
            self.assertNotIn("L4-fixture", layers(report))
            self.assertEqual(report.errors, [])

    def test_fixture_that_passes_is_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(
                Path(tmp),
                "invalid-x.yaml",
                "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: fine\n",
            )
            config = val.Config(expect_invalid=[str(path)])
            report = val.validate([path], config)
            self.assertIn("L4-fixture", layers(report))
            self.assertIn("regressed", messages(report))


class DeviationTest(unittest.TestCase):
    def _bad_manifest(self, tmp: Path) -> Path:
        return write(tmp, "bad.yaml", "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: BAD_NAME\n")

    def test_matching_deviation_downgrades_to_warning(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self._bad_manifest(Path(tmp))
            config = val.Config(
                deviations=[val.Deviation(path=str(path), layer="L2-structure", contains="", reason="legacy")]
            )
            report = val.validate([path], config)
            self.assertEqual(report.errors, [])
            self.assertTrue(report.warnings)
            self.assertIn("known deviation: legacy", messages(report))

    def test_stale_deviation_is_reported_as_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = write(Path(tmp), "ok.yaml", "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: fine\n")
            config = val.Config(
                deviations=[val.Deviation(path="nowhere/*.yaml", layer="", contains="", reason="obsolete")]
            )
            report = val.validate([path], config)
            self.assertIn("L0-config", layers(report))
            self.assertIn("no longer matches", messages(report))

    def test_contains_filter_narrows_the_waiver(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self._bad_manifest(Path(tmp))
            config = val.Config(
                deviations=[
                    val.Deviation(path=str(path), layer="", contains="something else", reason="narrow")
                ]
            )
            report = val.validate([path], config)
            # The waiver does not match the real issue, so the issue stands
            # *and* the waiver is flagged as stale.
            self.assertTrue(report.errors)
            self.assertIn("L0-config", layers(report))


class BoundSchemaTest(unittest.TestCase):
    def test_document_is_checked_against_a_bound_json_schema(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            schema = tmp_path / "s.json"
            schema.write_text(
                json.dumps({"type": "object", "required": ["replicas"], "properties": {"replicas": {"type": "integer"}}})
            )
            good = write(tmp_path, "good.yaml", "replicas: 3\n")
            bad = write(tmp_path, "bad.yaml", "replicas: many\n")
            config = val.Config(schemas=[val.SchemaBinding(glob=str(tmp_path / "*.yaml"), schema=str(schema))])
            self.assertEqual(val.validate([good], config).errors, [])
            self.assertTrue(val.validate([bad], config).errors)


class RepositoryGateTest(unittest.TestCase):
    """The repository must stay free of hard YAML validation errors."""

    def test_repository_has_no_validation_errors(self):
        config = val.Config.load(val.DEFAULT_CONFIG)
        paths = val.discover([str(val.REPO_ROOT)], config)
        self.assertTrue(paths, "expected to discover YAML files")
        report = val.validate(paths, config)
        self.assertEqual(
            [i.location() + ": " + i.message for i in report.errors],
            [],
            "YAML manifest validation errors introduced",
        )

    def test_repository_config_declares_negative_fixtures(self):
        config = val.Config.load(val.DEFAULT_CONFIG)
        self.assertTrue(config.expect_invalid, "negative fixtures must stay declared")

    def test_helm_values_are_bound_to_the_chart_schema(self):
        config = val.Config.load(val.DEFAULT_CONFIG)
        bound = config.schema_for("charts/stellar-operator/values.yaml")
        self.assertIsNotNone(bound)
        self.assertTrue(bound.is_file())


if __name__ == "__main__":
    unittest.main()
