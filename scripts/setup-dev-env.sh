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
#
# scripts/setup-dev-env.sh
#
# Single entrypoint for bootstrapping a local Stellar-K8s dev environment.
# Detects the host OS and delegates to the existing platform-specific
# setup script (setup-linux.sh / setup-mac.sh) instead of requiring
# contributors to know which one to run.
#
# Usage:
#   ./scripts/setup-dev-env.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$(uname -s)" in
  Linux)
    exec "${SCRIPT_DIR}/setup-linux.sh" "$@"
    ;;
  Darwin)
    exec "${SCRIPT_DIR}/setup-mac.sh" "$@"
    ;;
  *)
    echo "Unsupported platform: $(uname -s)" >&2
    echo "See DEVELOPMENT.md for manual setup instructions." >&2
    exit 1
    ;;
esac
