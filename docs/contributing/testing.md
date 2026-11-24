# Documentation Testing

How Stellar-K8s documentation is validated in CI and locally.

## Link checking

| Check | Command | CI job |
| --- | --- | --- |
| Internal file and anchor links | `python3 scripts/check-links.py` / `make link-check` | `ci.yml` docs-link-check |
| Repo-wide links (markdown, source, configs) | `make link-check-all` (lychee) | `ci.yml` repo-wide-link-check |
| Weekly external link scan | lychee with `lychee.toml` | `link-check.yml` |

## Docs build

Build the site exactly like CI before pushing docs changes:

```bash
pip install -r requirements.txt
mkdocs build --strict
```

A strict build fails on missing nav targets, unresolvable links, and
warnings from plugins. Fix the warning rather than working around it.

## Generated pages

`docs/api-reference.md` is generated from the CRD schema — never edit it
by hand. Regenerate and verify with:

```bash
make generate-api-docs
make check-api-docs
```

## Stale content

`make check-stale-docs` flags documentation that has fallen behind the
source code. Update the page or the code so they agree again.
