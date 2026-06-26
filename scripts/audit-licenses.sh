#!/usr/bin/env bash
# Generate/update docs/third-party-licenses.md from Cargo metadata.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
OUT_FILE="${REPO_ROOT}/docs/third-party-licenses.md"

TMP_METADATA="$(mktemp)"
cleanup() {
  rm -f "${TMP_METADATA}"
}
trap cleanup EXIT

cd "${REPO_ROOT}"

echo "==> Collecting dependency metadata"
cargo metadata --format-version 1 --locked > "${TMP_METADATA}"

echo "==> Writing ${OUT_FILE}"
python3 - "${TMP_METADATA}" "${OUT_FILE}" <<'PY'
import datetime
import json
import pathlib
import sys

metadata_path = pathlib.Path(sys.argv[1])
out_path = pathlib.Path(sys.argv[2])

with metadata_path.open("r", encoding="utf-8") as f:
    metadata = json.load(f)

packages = metadata.get("packages", [])
third_party = []
for pkg in packages:
    source = (pkg.get("source") or "").strip()
    # Workspace/path dependencies typically have no source; skip them.
    if not source or source.startswith("path+"):
        continue
    third_party.append(
        {
            "name": pkg.get("name", "unknown"),
            "version": pkg.get("version", "unknown"),
            "license": pkg.get("license") or "UNKNOWN",
            "repository": pkg.get("repository") or pkg.get("homepage") or "",
            "source": source,
        }
    )

third_party.sort(key=lambda p: (p["name"].lower(), p["version"]))

unknown_count = sum(1 for p in third_party if p["license"] == "UNKNOWN")
timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")

lines = []
lines.append("# Third-Party License References")
lines.append("")
lines.append(
    "This document is generated from Cargo metadata and tracks license declarations for third-party Rust dependencies."
)
lines.append("")
lines.append(f"- Generated at (UTC): {timestamp}")
lines.append("- Data source: `cargo metadata --format-version 1 --locked`")
lines.append(f"- Total third-party crates: {len(third_party)}")
lines.append(f"- Crates with UNKNOWN license field: {unknown_count}")
lines.append("")
lines.append("## Review Guidance")
lines.append("")
lines.append("- Verify all `UNKNOWN` entries before release packaging.")
lines.append("- Validate compatibility with the project license policy.")
lines.append("- Re-run `make license-audit` whenever `Cargo.lock` changes.")
lines.append("")
lines.append("## Dependency License Table")
lines.append("")
lines.append("| Crate | Version | License | Source | Repository |")
lines.append("|---|---|---|---|---|")

for pkg in third_party:
    repo = pkg["repository"].replace("|", "\\|")
    source = pkg["source"].replace("|", "\\|")
    license_expr = pkg["license"].replace("|", "\\|")
    lines.append(
        f"| {pkg['name']} | {pkg['version']} | {license_expr} | {source} | {repo} |"
    )

out_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY

echo "==> License report updated: ${OUT_FILE}"
