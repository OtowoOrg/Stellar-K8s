# Scripts Directory Conventions

This page describes the rules for everything under `scripts/`. The canonical
rule set lives in [CONVENTIONS.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONVENTIONS.md);
the script inventory lives in [`scripts/README.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/README.md).
This page summarizes what a contributor needs before adding or changing a script.

## Layout

| Path | What goes there |
|---|---|
| `scripts/` | Operational scripts you run locally or from `make` targets |
| `scripts/ci/` | CI-only helpers that run inside GitHub Actions. No `make` target or local wrapper |
| `scripts/lib/` | Sourced shell helpers. Not entry points, so they must not define `main()` or `run()` |
| `scripts/tests/` | Test suites (`*.bats` for shell, `test_*.py` for Python) |

Scripts never live at the repository root.

## Naming

- Shell scripts use `kebab-case.sh` (for example `cleanup.sh`).
- Python tools use `kebab-case.py` (for example `check-links.py`).
- Three legacy `snake_case.py` files are grandfathered: `issue_template_lint.py`,
  `crd_migration_lint.py`, and `check_mkdocs_nav.py`. Do not add new snake_case names.

## Required in every script

- **Usage header.** Every shell script in `scripts/` (not `ci/` or `lib/`) contains a
  `Usage:` comment, enforced by the scripts convention gate.
- **Strict mode.** Shell scripts start with `set -euo pipefail`.
- **Shellcheck.** Every shell script passes `shellcheck -S error` (`make shellcheck`).
- **Shell safety.** Shell scripts pass `scripts/check-shell-safety.py` (`make shell-safety`).
  It flags unguarded `rm -rf`, `eval`, `curl | bash`, and missing strict mode.
- **License header.** Rust, shell, and YAML files carry the Apache-2.0 header
  (`make license-headers`).
- **Python.** Python tools are `py_compile`-clean (`make python-lint`).

## Make targets first

When a script and a `make` target both exist, prefer the `make` target. It pins the
workspace feature flags so results match CI.

## Tests

- Scripts with logic get a suite in `scripts/tests/`. Use
  [`tests/preflight.bats`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/tests/preflight.bats)
  as the reference layout.
- Keep each suite self-contained. Put temp files under `$BATS_TEST_TMPDIR`.
- Scripts that need a live cluster or network access are marked as such in
  `scripts/README.md` and are not part of the fast local gate.

## Scope rules

- Operational scripts only. Do not commit one-off, archive, or batch helpers.
- Cleanup goes through `scripts/cleanup.sh` (`make cleanup`). Do not add new cleanup scripts.
- If a script is obsolete, delete it rather than archiving it.

## Adding a script checklist

1. Pick a `kebab-case` name and place it in the right subdirectory.
2. Add a `Usage:` header and strict mode (shell).
3. Run `make shellcheck`, `make shell-safety`, and `make scripts-convention-check`.
4. Add a row to the matching table in [`scripts/README.md`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/README.md)
   with its purpose, invocation, and test suite (or `—`).
5. Add a `make` target if the script is meant to be run routinely.
6. Add or update a `scripts/tests/` suite if the script has logic.

## Related

- [Contributing Guide](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md)
- [Development Setup](development-setup.md)
- [Documentation Standards](code-standards.md)
