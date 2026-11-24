# Navigation Coverage

> **Issue #1711** — Adds a CI job and local `make` target that verify every local path in the `mkdocs.yml` navigation tree resolves to an existing file under `docs/`.

## Overview

MkDocs silently skips navigation entries that point to missing files, which produces a published site where sidebar links lead nowhere.  The **navigation coverage** gate runs `scripts/check_mkdocs_nav.py` to catch this class of error before it reaches production.

## Local Usage

```bash
make nav-coverage
```

Exits `0` when every local nav entry resolves; exits `1` and prints the offending paths otherwise.

## CI Behaviour

| Trigger | What runs |
|---|---|
| PR / push touching `docs/**`, `mkdocs.yml`, or `scripts/check_mkdocs_nav.py` | Navigation coverage check |
| `workflow_dispatch` | Navigation coverage check |

The check is intentionally fast (no network access, no build step) so it runs as an early gate before the heavier `docs-build` job.

## How It Works

`scripts/check_mkdocs_nav.py` loads `mkdocs.yml` with `yaml.BaseLoader`, walks the `nav:` tree recursively, and resolves each string entry against the configured `docs_dir` (defaults to `docs/`).  URL-scheme entries (`https://`, `//`) are skipped.  Any path that does not exist as a regular file is reported and causes a non-zero exit.

## Adding a New Page

1. Create the `.md` file under `docs/`.
2. Add it to the `nav:` section of `mkdocs.yml`.
3. Run `make nav-coverage` locally to confirm the entry resolves before pushing.
4. Run `make docs-ci-build`. MkDocs logs every page that exists under `docs/` but is missing from `nav:`; only pages listed under `not_in_nav` in `mkdocs.yml` may be left out.

## Cross-References

- Script: [`scripts/check_mkdocs_nav.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check_mkdocs_nav.py)
- CI workflow: [`.github/workflows/nav-coverage.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/nav-coverage.yml)
- Documentation build gate: [`docs-build.md`](docs-build.md)
