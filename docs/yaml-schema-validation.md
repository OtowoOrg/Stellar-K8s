# YAML and CRD Schema Validation

Repository YAML is linted with **yamllint** and checked against Kubernetes /
CRD JSON schemas with **kubeconform**. Helm charts are rendered first; raw
templates are not treated as deployable YAML.

## Scope

| Included | How |
|----------|-----|
| YAML syntax / style | `yamllint -c .yamllint.yml .` |
| Helm values + chart | `helm lint` / `helm template` then kubeconform on the render |
| CRDs in `config/crd/` | kubeconform built-in CRD schema |
| Custom resources in `examples/` and `config/samples/` | kubeconform + generated CRD JSON schemas |
| GitHub workflow YAML | yamllint only (not Kubernetes resources) |

| Excluded | Reason |
|----------|--------|
| `charts/**/templates/**` | Go templates, not valid YAML until `helm template` |
| `target/`, `bundle/` | Build / OLM output |
| `.kiro/` | Local spec drafts |
| YAML fragments named `_*.yaml` | Shared snippets without `apiVersion`/`kind` |

## Generating CRD JSON schemas

Canonical CRDs live in `config/crd/`. JSON schemas are derived from each
served version's `openAPIV3Schema` (not invented by hand):

```bash
python3 scripts/ci/extract-crd-json-schemas.py
```

Output files use kubeconform's KindSuffix layout:

```text
schemas/crd/<Kind>-<group>-<version>.json
```

Example: `StellarNode-stellar.org-v1alpha1.json`.

Helm chart copies of the same CRD (`charts/stellar-operator/templates/crd*.yaml`)
are skipped when `config/crd/` already provides that Kind. Obsolete / unserved
CRD versions are not emitted.

### Adding or updating a CRD

1. Change the Rust types under `src/crd/` and regenerate YAML with `make crd-gen`
   (or edit `config/crd/` if `crdgen` is blocked — see CI).
2. Run `python3 scripts/ci/extract-crd-json-schemas.py`.
3. Commit both the CRD YAML and `schemas/crd/` updates in the same PR.
4. Add or refresh a sample under `config/samples/` or `examples/`.
5. Reviewers should check required fields, enums, and validation rules in the
   CRD OpenAPI — the JSON schema is a projection of that source.

## Local commands

```bash
# Full #1291 gate used by CI (`yaml-schema` job)
make yaml-schema-validate

# Existing #1044 structure/schema walk (every YAML file)
make validate-yaml

# Pieces
yamllint -c .yamllint.yml .
python3 scripts/ci/extract-crd-json-schemas.py --check
helm template stellar-operator charts/stellar-operator > /tmp/rendered.yaml
kubeconform -strict -schema-location default \
  -schema-location 'schemas/crd/{{ .ResourceKind }}{{ .KindSuffix }}' \
  /tmp/rendered.yaml
```

Pinned tools: yamllint **v1.35.1** (pre-commit), kubeconform **v0.6.4** (CI).

## Helm-rendered manifests

`scripts/ci/validate-yaml.sh` always runs `helm template` before kubeconform.
Chart-only mistakes (nil pointers, bad indent in templates) fail at render time;
schema mistakes fail at kubeconform.

Helm unit tests (`helm unittest`) catch values/schema problems earlier. Invalid
`values.yaml` combinations that `values.schema.json` rejects are asserted in
`ci.yml` (`helm lint -f` with bad overrides).
