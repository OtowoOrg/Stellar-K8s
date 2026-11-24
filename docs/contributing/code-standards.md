# Documentation Standards

Conventions for Stellar-K8s documentation so pages render correctly with
the Material theme and pass the strict docs build.

## Structure

- One top-level `#` title per page, matching the nav entry.
- Sections in hierarchy (`##`, `###`); do not skip levels.
- Every new page must be added to the `nav:` section of `mkdocs.yml`.
  Pages missing from nav are invisible in site navigation.

## Links

- Prefer relative links between docs pages (`../deployment-guides/validator.md`).
- Include the `.md` extension so both GitHub rendering and mkdocs resolve them.
- Never link outside `docs/` with `../../` — use an absolute GitHub URL or
  move the content into `docs/`.
- Anchor links must match a heading on the target page (checked by
  `scripts/check-links.py`).

## Code and admonitions

- Fence code blocks with a language tag (```bash, ```yaml).
- Use admonitions (`!!! note`, `!!! warning`) for callouts; the
  `pymdownx.details` and `pymdownx.superfences` extensions are enabled.
- Keep line length reasonable; tables need header separator rows.

## Generated content

- `docs/api-reference.md` is generated — do not edit it (see
  [Documentation Testing](testing.md)).
