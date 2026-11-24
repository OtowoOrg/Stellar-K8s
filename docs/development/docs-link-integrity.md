# Link Integrity

> **Issue #1712** — Adds a CI workflow and local `make` targets that detect broken internal and external links across all Markdown documentation.

## Overview

Broken links erode contributor trust and frustrate end-users.  The **link integrity** gate combines two complementary tools:

| Tool | Scope | When it runs |
|---|---|---|
| `scripts/check-links.py` | Internal markdown links (relative paths, anchors) | Every PR |
| [lychee](https://lychee.cli.rs/) | Internal + external URLs | Push to main, weekly schedule, or `workflow_dispatch` with `check_external: true` |

## Local Usage

```bash
# Check internal links only (fast, no network)
make link-integrity

# Check internal + external links (slower, requires network)
make link-integrity-full
```

## CI Behaviour

| Trigger | Internal check | External check |
|---|---|---|
| Pull request | ✅ | ❌ (avoids rate-limiting) |
| Push to `main` | ✅ | ✅ |
| Weekly schedule (Mon 06:00 UTC) | ✅ | ✅ |
| `workflow_dispatch` with `check_external: true` | ✅ | ✅ |

The lychee report is uploaded as a workflow artifact (`lychee-report-<sha>`, retained 14 days) and appended to the job summary.

## lychee Configuration

Global exclusions (localhost, known rate-limited hosts, in-cluster DNS, placeholder URLs) are declared in `lychee.toml`.  Add new exclusions there rather than inline in the workflow.

```toml
# lychee.toml (excerpt)
exclude = [
  "^http://localhost",
  "^https://kubernetes\\.io/docs/",
  # ...
]
```

## Handling False Positives

- **Rate-limited external URLs** — Add the domain pattern to the `exclude` list in `lychee.toml`.
- **Planned docs that do not exist yet** — Add the relative path to `IGNORED_MISSING_FILES` in `scripts/check-links.py`.
- **In-cluster hostnames** — Already excluded via the `stellar-core.*:` patterns in `lychee.toml`.
- **Links to source, examples or workflows** — These live outside `docs/`, so a relative `../src/...` path breaks the MkDocs build. Use an absolute `https://github.com/OtowoOrg/Stellar-K8s/blob/main/<path>` URL instead.

## Cross-References

- Script: [`scripts/check-links.py`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/scripts/check-links.py)
- lychee config: [`lychee.toml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/lychee.toml)
- CI workflow: [`.github/workflows/link-integrity.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/link-integrity.yml)
- Documentation build gate: [`docs-build.md`](docs-build.md)
