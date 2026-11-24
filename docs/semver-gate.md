# Semantic Versioning Gate

Epic: #1523 · Script: `scripts/semver_gate.py` · Wired into: `.github/workflows/helm-release.yml`

The gate compares the working tree with the last chart release, which is the
highest `chart-v*` tag. It runs in well under a second.

```bash
python3 scripts/semver_gate.py required-bump          # prints major | minor | none
python3 scripts/semver_gate.py check --version 1.4.0  # exit 1 = release blocked
```

## Rules

- **Breaking CRD change means major.** The gate reuses the API diff in
  `scripts/crd_migration_lint.py`. Each of these counts as breaking:
  - a served version was removed
  - a property was removed
  - a property changed type
  - an existing field became required
  - a CRD was removed

  New CRDs and new served versions require at least a minor bump.
- **Versions are aligned.** Chart.yaml `version` and `appVersion` must both
  equal the release version. `values.yaml` `image.tag` must be empty (it
  defaults to appVersion) or equal the release version.
- **CRD and chart are compatible.** The chart's CRD template must serve the
  same versions, with the same storage version, as `config/crd`.
- **Versions move forward.** The release version must be greater than the
  last released version.

## Pipeline

1. `version-bump` runs `required-bump`. If it prints `major` and no manual
   override was given, the major bump is forced, whatever the commit messages
   say.
2. `bump-and-tag` runs `check` after Chart.yaml is bumped and before anything
   is committed, tagged or published.

Each failure message includes a `Fix:` line explaining how to resolve it.
