#!/usr/bin/env python3
"""Fail when a local MkDocs navigation entry points to a missing docs file."""

import argparse
from pathlib import Path
from urllib.parse import urlsplit

import yaml


def nav_paths(nav):
    if isinstance(nav, dict):
        for value in nav.values():
            yield from nav_paths(value)
    elif isinstance(nav, list):
        for value in nav:
            yield from nav_paths(value)
    elif isinstance(nav, str):
        yield nav


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("config", nargs="?", default="mkdocs.yml", type=Path)
    args = parser.parse_args()

    config_path = args.config.resolve()
    with config_path.open(encoding="utf-8") as config_file:
        config = yaml.load(config_file, Loader=yaml.BaseLoader)

    docs_dir = (config_path.parent / config.get("docs_dir", "docs")).resolve()
    missing = []
    for entry in nav_paths(config.get("nav", [])):
        parsed = urlsplit(entry)
        if parsed.scheme or entry.startswith("//"):
            continue
        target = (docs_dir / parsed.path).resolve()
        if not target.is_relative_to(docs_dir) or not target.is_file():
            missing.append((entry, target))

    if missing:
        for entry, target in missing:
            print(f"Missing MkDocs nav target: {entry} ({target})")
        return 1

    print(f"All local MkDocs nav targets resolve under {docs_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())