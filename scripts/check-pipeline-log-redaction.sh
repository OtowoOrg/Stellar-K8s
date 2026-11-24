#!/usr/bin/env bash
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
# check-pipeline-log-redaction.sh
#
# Enforce secret redaction checks in logs produced by pipeline commands.
# Closes Issue #1153.
#
# Wraps the `check-pipeline-log-redaction` Cargo binary so Makefile and CI share
# a single entrypoint. See docs/log-redaction-policy.md (§ Pipeline log checks).
#
# Exit codes
# ----------
#   0  — No findings (or --report)
#   1  — One or more redaction failures
#   2  — Tooling / usage error
#
# Usage:
#   ./scripts/check-pipeline-log-redaction.sh
#   ./scripts/check-pipeline-log-redaction.sh --report
#   ./scripts/check-pipeline-log-redaction.sh --fixture path/to/log.txt
#   ./scripts/check-pipeline-log-redaction.sh --scrub path/to/job.log

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

echo "→ Checking pipeline log secret redaction..."
exec cargo run --quiet --locked --bin check-pipeline-log-redaction -- "$@"
