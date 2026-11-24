#!/usr/bin/env bash
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
# scripts/reproducible-build.sh — build the release artifacts deterministically (#1997).
#
# Pins everything that otherwise makes two builds of the same source differ:
#
#   timestamps   SOURCE_DATE_EPOCH = commit time of the revision, TZ=UTC
#   paths        --remap-path-prefix for the checkout, CARGO_HOME and RUSTUP_HOME
#   locale       LC_ALL=C, LANG=C
#   linker       --build-id=none (no per-link build id)
#   cargo        --locked, CARGO_INCREMENTAL=0, a fresh target directory
#
# and writes <output-dir>/build-manifest.json, which verify-reproducible-build
# compares between the release pipeline and an independent rebuild.
#
# Usage:
#   scripts/reproducible-build.sh --output-dir DIR --pipeline NAME [--revision REV]
#                                 [--runner NAME] [--bins "a b c"]
#
# Exit codes: 0 built, 1 build failed, 2 bad usage.

set -euo pipefail

OUTPUT_DIR=""
PIPELINE=""
REVISION=""
RUNNER=""
BINS="stellar-operator kubectl-stellar stellar-sidecar"

usage() {
  sed -n '/^# Usage:/,/^# Exit codes/p' "$0" | sed 's/^# \{0,1\}//'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output-dir) OUTPUT_DIR="${2:-}"; shift 2 ;;
    --pipeline) PIPELINE="${2:-}"; shift 2 ;;
    --revision) REVISION="${2:-}"; shift 2 ;;
    --runner) RUNNER="${2:-}"; shift 2 ;;
    --bins) BINS="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ -z "${OUTPUT_DIR}" ] || [ -z "${PIPELINE}" ]; then
  echo "--output-dir and --pipeline are required" >&2
  usage >&2
  exit 2
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

if [ -z "${REVISION}" ]; then
  REVISION="$(git rev-parse HEAD)"
fi
if [ -z "${RUNNER}" ]; then
  RUNNER="${RUNNER_OS:-$(uname -s)}-${RUNNER_ARCH:-$(uname -m)}"
fi

# Building a revision other than the checked-out one would not be the source
# the release was cut from.
HEAD_REVISION="$(git rev-parse HEAD)"
if [ "${HEAD_REVISION}" != "${REVISION}" ]; then
  echo "checked-out revision ${HEAD_REVISION} is not ${REVISION}; check it out first" >&2
  exit 2
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "warning: working tree has uncommitted changes; the build is not of ${REVISION}" >&2
fi

mkdir -p "${OUTPUT_DIR}"
OUTPUT_DIR="$(cd "${OUTPUT_DIR}" && pwd)"

# ── Deterministic environment ────────────────────────────────────────────────
SOURCE_DATE_EPOCH="$(git log -1 --format=%ct "${REVISION}")"
BUILD_PATH_PREFIX="/build"
CARGO_HOME_DIR="${CARGO_HOME:-${HOME}/.cargo}"
RUSTUP_HOME_DIR="${RUSTUP_HOME:-${HOME}/.rustup}"

export SOURCE_DATE_EPOCH
export TZ=UTC
export LC_ALL=C
export LANG=C
export CARGO_INCREMENTAL=0
export BUILD_PATH_PREFIX
RUSTFLAGS="--remap-path-prefix=${REPO_ROOT}=${BUILD_PATH_PREFIX}"
RUSTFLAGS="${RUSTFLAGS} --remap-path-prefix=${CARGO_HOME_DIR}=/cargo"
RUSTFLAGS="${RUSTFLAGS} --remap-path-prefix=${RUSTUP_HOME_DIR}=/rustup"
RUSTFLAGS="${RUSTFLAGS} -C link-arg=-Wl,--build-id=none"
export RUSTFLAGS

# A fresh target directory so no cached object from another build leaks in.
TARGET_DIR="$(mktemp -d)"
export CARGO_TARGET_DIR="${TARGET_DIR}"
trap 'rm -rf "${TARGET_DIR}"' EXIT

echo "reproducible build: pipeline=${PIPELINE} revision=${REVISION} runner=${RUNNER}"
echo "  SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} TZ=${TZ} LC_ALL=${LC_ALL}"
echo "  RUSTFLAGS=${RUSTFLAGS}"

# ── Build ────────────────────────────────────────────────────────────────────
bin_args=()
for bin in ${BINS}; do
  bin_args+=(--bin "${bin}")
done
cargo build --release --locked "${bin_args[@]}"

for bin in ${BINS}; do
  cp "${TARGET_DIR}/release/${bin}" "${OUTPUT_DIR}/${bin}"
done

# The CRD bundle is generated, so it is an artifact too.
cargo run --release --locked --bin crdgen > "${OUTPUT_DIR}/stellar-crds.yaml"

# ── Manifest ─────────────────────────────────────────────────────────────────
OUTPUT_DIR="${OUTPUT_DIR}" PIPELINE="${PIPELINE}" REVISION="${REVISION}" RUNNER="${RUNNER}" \
BINS="${BINS}" python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path

out = Path(os.environ["OUTPUT_DIR"])
bins = os.environ["BINS"].split()
pinned = ["SOURCE_DATE_EPOCH", "TZ", "LC_ALL", "LANG", "CARGO_INCREMENTAL", "BUILD_PATH_PREFIX", "RUSTFLAGS"]
env = {key: os.environ[key] for key in pinned}


def artifact(name, kind):
    data = (out / name).read_bytes()
    return {
        "name": name,
        "kind": kind,
        "path": name,
        "sha256": hashlib.sha256(data).hexdigest(),
        "len": len(data),
    }


artifacts = [artifact(name, "binary") for name in bins] + [artifact("stellar-crds.yaml", "crd_bundle")]
manifest = {
    "revision": os.environ["REVISION"],
    "pipeline": {"name": os.environ["PIPELINE"], "runner": os.environ["RUNNER"]},
    "steps": [
        {"name": "cargo-build", "env": env, "artifacts": bins},
        {"name": "crdgen", "env": env, "artifacts": ["stellar-crds.yaml"]},
    ],
    "artifacts": artifacts,
}
(out / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(f"wrote {out / 'build-manifest.json'} ({len(artifacts)} artifacts)")
PY
