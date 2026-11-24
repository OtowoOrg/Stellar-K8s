# CRD JSON schemas

Schemas in this directory are **generated** from the canonical CRD YAML under
`config/crd/`. Do not edit them by hand.

```bash
python3 scripts/ci/extract-crd-json-schemas.py
```

CI runs the same command with `--check` so a CRD change that is not reflected
here fails. See `docs/yaml-schema-validation.md`.
