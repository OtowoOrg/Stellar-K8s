# Copyright 2026 Stellar-K8s Contributors
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
"""Tests for scripts/dependency-upgrades.py (#1999)."""

import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "dependency-upgrades.py"
spec = importlib.util.spec_from_file_location("dependency_upgrades", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def lock(*packages):
    return "\n".join(
        f'[[package]]\nname = "{name}"\nversion = "{version}"\n' for name, version in packages
    )


class LockedVersionsTest(unittest.TestCase):
    def test_collects_every_version_of_a_crate(self):
        text = lock(("kube", "0.94.0"), ("syn", "1.0.0"), ("syn", "2.0.0"))
        versions = module.locked_versions(text)
        self.assertEqual(versions["kube"], ["0.94.0"])
        self.assertEqual(versions["syn"], ["1.0.0", "2.0.0"])


class DirectDependenciesTest(unittest.TestCase):
    def test_reads_every_section_and_target_tables(self):
        manifest = """
[dependencies]
kube = { version = "0.94" }
serde = "1"
yaml = { package = "serde_yaml", version = "0.9" }

[dev-dependencies]
wiremock = "0.6"

[build-dependencies]
walkdir = "2"

[target.'cfg(unix)'.dependencies]
nix = "0.30"
"""
        self.assertEqual(
            module.direct_dependencies(manifest),
            {"kube", "serde", "serde_yaml", "wiremock", "walkdir", "nix"},
        )


class UpgradesTest(unittest.TestCase):
    def test_reports_only_changed_direct_dependencies(self):
        base = module.locked_versions(lock(("kube", "0.94.0"), ("serde", "1.0.1"), ("bytes", "1.0.0")))
        head = module.locked_versions(lock(("kube", "0.95.0"), ("serde", "1.0.1"), ("bytes", "2.0.0")))
        changed, skipped = module.upgrades(base, head, {"kube", "serde"})
        self.assertEqual(changed, [("kube", "0.94.0", "0.95.0")])
        self.assertEqual(skipped, [])

    def test_transitive_only_changes_are_not_gated(self):
        base = module.locked_versions(lock(("bytes", "1.0.0")))
        head = module.locked_versions(lock(("bytes", "2.0.0")))
        self.assertEqual(module.upgrades(base, head, {"kube"}), ([], []))

    def test_crates_locked_at_several_versions_are_skipped(self):
        base = module.locked_versions(lock(("syn", "1.0.0"), ("syn", "2.0.0")))
        head = module.locked_versions(lock(("syn", "1.0.1"), ("syn", "2.0.0")))
        changed, skipped = module.upgrades(base, head, {"syn"})
        self.assertEqual(changed, [])
        self.assertEqual(skipped, ["syn"])

    def test_new_and_removed_crates_are_not_upgrades(self):
        base = module.locked_versions(lock(("old", "1.0.0")))
        head = module.locked_versions(lock(("new", "1.0.0")))
        self.assertEqual(module.upgrades(base, head, {"old", "new"}), ([], []))


if __name__ == "__main__":
    unittest.main()
