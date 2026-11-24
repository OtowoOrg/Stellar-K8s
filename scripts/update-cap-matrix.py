#!/usr/bin/env python3
"""Refresh the checked-in stellar-core release/CAP matrix from GitHub releases."""
import json
import re
import urllib.request
from pathlib import Path


API_URL = "https://api.github.com/repos/stellar/stellar-core/releases?per_page=100"
OUTPUT = Path(__file__).resolve().parents[1] / "config" / "caps.toml"
TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)(?:[-.]([A-Za-z0-9.-]+))?$")
CAP_RE = re.compile(r"\bCAP[- ]0*(\d+)\b", re.IGNORECASE)
PROTOCOL_SECTION_RE = re.compile(
    r"^\s*#{1,3}\s*Protocol\s+(\d+)(.*?)(?=^\s*#{1,3}\s|\Z)",
    re.IGNORECASE | re.MULTILINE | re.DOTALL,
)


def release_rows(releases):
    rows = []
    for release in releases:
        if release.get("draft") or release.get("prerelease"):
            continue
        tag = release.get("tag_name", "")
        match = TAG_RE.match(tag)
        if not match:
            continue
        major = int(match.group(1))
        body = release.get("body") or ""
        section = PROTOCOL_SECTION_RE.search(body)
        protocol = int(section.group(1)) if section else major
        cap_text = section.group(2) if section else body
        caps = sorted({f"CAP-{int(value):04d}" for value in CAP_RE.findall(cap_text)})
        rows.append((tag, protocol, caps))
        if len(rows) == 10:
            break
    if len(rows) < 10:
        raise RuntimeError(f"Expected 10 stable stellar-core releases, received {len(rows)}")
    return rows


def self_check():
    rows = release_rows(
        [
            {
                "tag_name": "v28.0.0",
                "body": "  ## Protocol 28\nIncludes CAP-0083, CAP-0085, and CAP-0086.\n## Other changes",
            },
            {"tag_name": "v27.0.0", "body": "Release notes mention CAP-0026."},
            *[
                {"tag_name": f"v{version}.0.0", "body": ""}
                for version in range(26, 17, -1)
            ],
        ]
    )
    if rows[0] != ("v28.0.0", 28, ["CAP-0083", "CAP-0085", "CAP-0086"]):
        raise AssertionError("indented protocol headings or CAP identifiers were not parsed")
    if rows[1][2] != ["CAP-0026"]:
        raise AssertionError("CAP identifiers outside a protocol heading were not parsed")


def render(rows):
    lines = [
        "# Generated from stellar-core stable release notes by scripts/update-cap-matrix.py.",
        "# `introduced_caps` lists CAPs named in the release's protocol section.",
    ]
    for version, protocol, caps in rows:
        quoted_caps = ", ".join(json.dumps(cap) for cap in caps)
        lines.extend(
            [
                "",
                "[[releases]]",
                f"version = {json.dumps(version)}",
                f"protocol = {protocol}",
                f"xdr_version = {protocol}",
                f"introduced_caps = [{quoted_caps}]",
            ]
        )
    return "\n".join(lines) + "\n"


def main():
    self_check()
    request = urllib.request.Request(API_URL, headers={"Accept": "application/vnd.github+json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        releases = json.load(response)
    OUTPUT.write_text(render(release_rows(releases)), encoding="utf-8")
    print(f"Updated {OUTPUT}")


if __name__ == "__main__":
    main()