#!/usr/bin/env python3
"""Unit tests for scripts/semver_gate.py (issue #1523)."""

import contextlib
import importlib.util
import io
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

import yaml

_SPEC = importlib.util.spec_from_file_location(
    "semver_gate", Path(__file__).resolve().parent.parent / "semver_gate.py"
)
gate = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(gate)


def _crd(properties, versions=("v1alpha1",)):
    return {
        "apiVersion": "apiextensions.k8s.io/v1",
        "kind": "CustomResourceDefinition",
        "metadata": {"name": "widgets.stellar.org"},
        "spec": {
            "versions": [
                {
                    "name": v,
                    "served": True,
                    "storage": i == 0,
                    "schema": {
                        "openAPIV3Schema": {
                            "type": "object",
                            "properties": {"spec": {"type": "object", "properties": properties}},
                        }
                    },
                }
                for i, v in enumerate(versions)
            ]
        },
    }


BASE_PROPS = {"size": {"type": "string"}, "replicas": {"type": "integer"}}


class ReleaseRepo:
    """Temporary git repo with a chart + CRD released as chart-v1.0.0."""

    def __init__(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self._git("init", "-q")
        self._git("config", "user.email", "t@example.com")
        self._git("config", "user.name", "t")
        self.write(version="1.0.0")
        self._git("add", "-A")
        self._git("commit", "-qm", "release")
        self._git("tag", "chart-v1.0.0")

    def _git(self, *args):
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def write(self, version, app_version=None, tag="", props=None, versions=("v1alpha1",),
              chart_versions=None):
        chart = self.root / gate.CHART_DIR
        (chart / "templates").mkdir(parents=True, exist_ok=True)
        (chart / "Chart.yaml").write_text(
            yaml.safe_dump({"apiVersion": "v2", "name": "stellar-operator", "version": version,
                            "appVersion": app_version or version})
        )
        (chart / "values.yaml").write_text(yaml.safe_dump({"image": {"tag": tag}}))
        crd = _crd(props or BASE_PROPS, versions)
        chart_crd = _crd(props or BASE_PROPS, chart_versions or versions)
        (chart / "templates" / "crd.yaml").write_text(
            "{{- if .Values.installCRDs }}\n---\n" + yaml.safe_dump(chart_crd) + "{{- end }}\n"
        )
        (self.root / gate.CRD_DIR).mkdir(parents=True, exist_ok=True)
        (self.root / gate.CRD_DIR / "widget-crd.yaml").write_text(yaml.safe_dump(crd))

    def run(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = gate.main(["--root", str(self.root), *argv])
        return code, out.getvalue(), err.getvalue()

    def close(self):
        self._tmp.cleanup()


class SemverGateTest(unittest.TestCase):
    def setUp(self):
        self.repo = ReleaseRepo()

    def tearDown(self):
        self.repo.close()

    def test_aligned_patch_release_passes(self):
        self.repo.write(version="1.0.1")
        code, out, _ = self.repo.run("check", "--version", "1.0.1")
        self.assertEqual(code, 0, out)

    def test_mismatched_versions_blocked_with_remediation(self):
        self.repo.write(version="1.0.1", app_version="1.0.0", tag="0.9.0")
        code, _, err = self.repo.run("check", "--version", "1.0.1")
        self.assertEqual(code, 1)
        self.assertIn("appVersion is '1.0.0', expected '1.0.1'", err)
        self.assertIn("image.tag is '0.9.0'", err)
        self.assertIn("Fix:", err)

    def test_chart_crd_out_of_sync_blocked(self):
        self.repo.write(version="1.1.0", versions=("v1alpha1", "v1beta1"),
                        chart_versions=("v1alpha1",))
        code, _, err = self.repo.run("check", "--version", "1.1.0")
        self.assertEqual(code, 1)
        self.assertIn("regenerate the chart CRD", err)

    def test_breaking_crd_change_forces_major(self):
        self.repo.write(version="1.1.0", props={"size": {"type": "string"}})
        code, out, _ = self.repo.run("required-bump")
        self.assertEqual(out.strip(), "major")

        code, _, err = self.repo.run("check", "--version", "1.1.0")
        self.assertEqual(code, 1)
        self.assertIn("require a major bump", err)
        self.assertIn("property 'spec.replicas' was removed", err)
        self.assertIn("bump_override=major", err)

        self.repo.write(version="2.0.0", props={"size": {"type": "string"}})
        code, out, err = self.repo.run("check", "--version", "2.0.0")
        self.assertEqual(code, 0, err)

    def test_type_change_is_breaking(self):
        props = dict(BASE_PROPS, replicas={"type": "string"})
        self.repo.write(version="1.0.1", props=props)
        self.assertEqual(self.repo.run("required-bump")[1].strip(), "major")

    def test_additive_change_requires_minor(self):
        props = dict(BASE_PROPS, labels={"type": "object"})
        self.repo.write(version="1.0.1", props=props, versions=("v1alpha1", "v1beta1"))
        self.assertEqual(self.repo.run("required-bump")[1].strip(), "minor")
        code, _, err = self.repo.run("check", "--version", "1.0.1")
        self.assertEqual(code, 1)
        self.assertIn("require a minor bump", err)
        self.repo.write(version="1.1.0", props=props, versions=("v1alpha1", "v1beta1"))
        self.assertEqual(self.repo.run("check", "--version", "1.1.0")[0], 0)

    def test_version_must_increase(self):
        code, _, err = self.repo.run("check", "--version", "1.0.0")
        self.assertEqual(code, 1)
        self.assertIn("not greater than 1.0.0", err)

    def test_check_is_fast(self):
        self.repo.write(version="1.0.1")
        start = time.monotonic()
        self.repo.run("check", "--version", "1.0.1")
        self.assertLess(time.monotonic() - start, 120)

    def test_bump_level(self):
        self.assertEqual(gate.bump_level("1.2.3", "2.0.0"), "major")
        self.assertEqual(gate.bump_level("1.2.3", "1.3.0"), "minor")
        self.assertEqual(gate.bump_level("1.2.3", "1.2.4"), "patch")
        self.assertEqual(gate.bump_level("1.2.3", "1.2.3"), "invalid")
        self.assertEqual(gate.bump_level("1.2.3", "v1.3"), "invalid")


if __name__ == "__main__":
    unittest.main()
