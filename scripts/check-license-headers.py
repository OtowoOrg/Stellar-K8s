#!/usr/bin/env python3
"""License header enforcement for Stellar-K8s — Issue #1286.

Scans Rust, Shell, and YAML files for the repository's canonical Apache-2.0
license header. Fails CI when a required header is missing or malformed.

Usage:
    python3 scripts/check-license-headers.py            # check all files
    python3 scripts/check-license-headers.py --fix      # auto-fix missing headers
    python3 scripts/check-license-headers.py --report   # report-only, always exit 0

Exit codes:
    0  All applicable files have valid headers
    1  One or more files are missing or have malformed headers
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent

# ── Canonical license header ───────────────────────────────────────────────────
# Apache-2.0 short-form SPDX header. This is the single source of truth.
# The header must appear within the first MAX_HEADER_LINES of the file.

RUST_HEADER = """\
// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
"""

SHELL_HEADER = """\
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
"""

YAML_HEADER = """\
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
"""

# SPDX short-form header (acceptable alternative)
SPDX_PATTERN = re.compile(
    r"#.*SPDX-License-Identifier:\s*Apache-2\.0",
    re.IGNORECASE,
)

MAX_HEADER_LINES = 25

# ── Exclusion patterns ────────────────────────────────────────────────────────
# Files matching these patterns are skipped entirely.
EXCLUDED_PATHS = [
    # Generated files
    "target/",
    "bundle/",
    "charts/stellar-operator/rendered/",
    # CRD files (auto-generated from Rust types via crdgen)
    "config/crd/",
    # Vendored / third-party
    "vendor/",
    # CI generated
    ".github/",
    # Pre-commit config itself
    ".pre-commit-config.yaml",
    # Documentation
    "docs/",
    # Helm chart templates (Go template syntax)
    "charts/stellar-operator/templates/",
    # Helm chart tests
    "charts/stellar-operator/tests/",
    # Config samples (example manifests)
    "config/samples/",
    # Examples
    "examples/",
    # Benchmark baselines (JSON data)
    "benchmarks/baselines/",
    # Benchmark k6 scripts (JavaScript)
    "benchmarks/k6/",
    # Security tool configs
    ".gitleaks.toml",
    ".cargo/audit.toml",
    "deny.toml",
    # Generated JSON dashboards
    "monitoring/*.json",
    "config/grafana/",
    # Schema files
    "schemas/",
    # Lock files
    "Cargo.lock",
    "package-lock.json",
    # Kiro specs (AI-generated)
    ".kiro/",
    # Build script (generates code)
    "build.rs",
]

# Files that should never have headers (data, binary, or non-applicable)
EXCLUDED_FILENAMES = [
    "LICENSE",
    "CHANGELOG.md",
    "README.md",
    "CONTRIBUTING.md",
    "CONVENTIONS.md",
    "SECURITY.md",
    "DEVELOPMENT.md",
    "RELEASE_CHECKLIST.md",
    "DEPENDENCY_SECURITY_AUDIT.md",
    "SECURITY_IMPLEMENTATION.md",
    "PIPELINE_HARDENING_SUMMARY.md",
    "THIRD_PARTY_LICENSES.md",
    "mkdocs.yml",
    "cliff.toml",
    "lychee.toml",
    "doc-coverage.toml",
    ".doc-hashes.toml",
    ".editorconfig",
    ".gitattributes",
    ".gitignore",
    ".dockerignore",
    ".yamllint.yml",
    ".commitlintrc.yaml",
    ".codecov.yml",
    "mlc_config.json",
    "requirements.txt",
    "Tiltfile",
    "skaffold.yaml",
    "PROJECT",
    "krew-plugin.yaml",
]


def should_exclude(path: Path) -> bool:
    """Determine if a file should be excluded from header checks."""
    rel = path.relative_to(REPO_ROOT)
    rel_str = str(rel)

    # Check excluded filenames
    if path.name in EXCLUDED_FILENAMES:
        return True

    # Check excluded path patterns
    for pattern in EXCLUDED_PATHS:
        if pattern.endswith("/"):
            if rel_str.startswith(pattern) or f"/{pattern}" in rel_str:
                return True
        elif pattern.startswith("*"):
            if rel_str.endswith(pattern[1:]):
                return True
        elif pattern in rel_str:
            return True

    return False


def get_header_for_file(path: Path) -> str | None:
    """Return the expected header for a file, or None if not applicable."""
    suffix = path.suffix.lower()
    if suffix == ".rs":
        return RUST_HEADER
    elif suffix == ".sh":
        return SHELL_HEADER
    elif suffix in (".yaml", ".yml"):
        return YAML_HEADER
    return None


def has_valid_header(path: Path, expected_header: str) -> tuple[bool, str]:
    """Check if a file has a valid license header.

    Returns (is_valid, reason).
    """
    try:
        content = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return True, "binary file, skipping"

    lines = content.splitlines()
    header_lines = lines[:MAX_HEADER_LINES]

    # Check for SPDX identifier (acceptable alternative)
    for line in header_lines:
        if SPDX_PATTERN.search(line):
            return True, "SPDX-License-Identifier found"

    # Check for the canonical header
    expected_lines = expected_header.strip().splitlines()
    header_region = "\n".join(header_lines)

    # Normalize whitespace for comparison (allow slight indentation differences)
    normalized_expected = [line.strip() for line in expected_lines]
    normalized_region = [line.strip() for line in header_lines[: len(expected_lines) + 5]]

    # Check that all expected lines appear in order
    exp_idx = 0
    for line in normalized_region:
        if exp_idx < len(normalized_expected) and line == normalized_expected[exp_idx]:
            exp_idx += 1

    if exp_idx == len(normalized_expected):
        return True, "canonical header found"

    # Check if there's at least a copyright line
    for line in header_lines[:10]:
        if "Copyright" in line and "Stellar-K8s" in line:
            return False, "copyright line present but header is incomplete or malformed"

    return False, "missing license header"


def fix_header(path: Path, expected_header: str) -> bool:
    """Add the expected license header to a file. Returns True if modified."""
    try:
        content = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return False

    lines = content.splitlines()

    # If file starts with a shebang, insert header after shebang
    insert_at = 0
    if lines and lines[0].startswith("#!"):
        insert_at = 1

    new_content = ""
    if insert_at == 1 and len(lines) > 0:
        new_content = lines[0] + "\n" + expected_header + "\n".join(lines[1:])
    else:
        new_content = expected_header + "\n".join(lines)
    path.write_text(new_content, encoding="utf-8")
    return True


def collect_files() -> list[Path]:
    """Collect all applicable files in the repository."""
    applicable = []

    for pattern in ["**/*.rs", "**/*.sh", "**/*.yaml", "**/*.yml"]:
        for path in REPO_ROOT.glob(pattern):
            if path.is_file() and not should_exclude(path):
                applicable.append(path)

    return sorted(applicable)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="License header enforcement for Stellar-K8s"
    )
    parser.add_argument(
        "--fix",
        action="store_true",
        help="Auto-add missing headers (modifies files in place)",
    )
    parser.add_argument(
        "--report",
        action="store_true",
        help="Report-only mode, always exit 0",
    )
    args = parser.parse_args(argv)

    files = collect_files()
    violations: list[tuple[Path, str]] = []
    fixed = 0

    for path in files:
        header = get_header_for_file(path)
        if header is None:
            continue

        is_valid, reason = has_valid_header(path, header)

        if not is_valid:
            if args.fix:
                if fix_header(path, header):
                    fixed += 1
                    print(f"  FIXED: {path.relative_to(REPO_ROOT)}")
                else:
                    violations.append((path, reason))
            else:
                violations.append((path, reason))

    # Report
    if violations:
        print(f"\n{'='*60}")
        print(f"License Header Check: {len(violations)} file(s) need attention")
        print(f"{'='*60}")
        for path, reason in sorted(violations):
            rel = path.relative_to(REPO_ROOT)
            suffix = path.suffix.lower()
            if suffix == ".rs":
                expected_desc = "Rust // header"
            elif suffix == ".sh":
                expected_desc = "# header (after shebang)"
            else:
                expected_desc = "# YAML header"
            print(f"\n  {rel}")
            print(f"    Reason: {reason}")
            print(f"    Expected: {expected_desc}")
            print(f"    Fix: Add the canonical Apache-2.0 header to the first lines")
    elif args.fix and fixed > 0:
        print(f"\n{'='*60}")
        print(f"Fixed {fixed} file(s)")
        print(f"{'='*60}")

    if args.report:
        return 0

    if violations:
        return 1

    print(f"\nAll {len(files)} applicable files have valid license headers.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
