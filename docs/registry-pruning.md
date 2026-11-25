# Registry Pruning

The operator image is published to GitHub Container Registry as
`ghcr.io/otowoorg/stellar-k8s`. Every push to `main` adds a `sha-<commit>` version, and every
moved tag (`main`, `latest`) leaves an untagged version behind, so the package grows without
bound. `scripts/prune-registry-images.sh` removes the stale versions, and the
[Registry Prune](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/workflows/registry-prune.yml)
workflow runs it on a schedule.

Deleting is always opt-in. The script and the workflow both default to a **dry run** that only
prints the plan.

## What is kept

A version is kept when any of the following holds. Only versions that match none of them are
planned for deletion.

| Rule | Keeps |
|---|---|
| Protected tag | `latest`, `main`, `master`, and release tags: `X.Y.Z`, `vX.Y.Z`, `X.Y.Z-rc.1`, `X.Y`, `X`. Add more with `--protect-tag REGEX`. |
| Referenced by the repo | Tags and digests of the image found under `charts/` (chart `appVersion`, `values*.yaml`), `config/` and `examples/` (manifests, kustomize `newTag`). `tests/` directories are ignored. |
| Newest N | The `--keep-latest` newest versions (default 10). |
| Young | Versions younger than `--min-age-days` (default 30, minimum 1). |
| Child of a kept image | Untagged per-platform manifests and attestations that belong to a kept multi-arch index. They are resolved through the registry, and a failed lookup aborts the run before anything is deleted. |
| Signature of a kept image | Cosign signatures and attestations (`sha256-<digest>[.sig\|.att\|.sbom]`) follow the image they sign. Ones whose image is not in the package are kept. |

Two more guards apply to `--delete`: it refuses to run a plan larger than `--max-delete`
(default 100, exit code 3), and it cannot be combined with `--versions-file`, so a fixture is
never used to delete real versions.

## Running it locally

The script needs `gh` (logged in with `read:packages`, plus `delete:packages` to delete) and `jq`.

```bash
# 1. Dry run (the default): prints KEEP/DELETE for every version and deletes nothing
scripts/prune-registry-images.sh

# 2. Tighter or looser limits
scripts/prune-registry-images.sh --min-age-days 60 --keep-latest 20

# 3. Apply the plan: prints it again, then deletes exactly those versions
scripts/prune-registry-images.sh --delete
```

Selection can be checked without any network access by pointing the script at a saved API
response (and optionally a map of index digest to child digests):

```bash
scripts/prune-registry-images.sh \
  --versions-file scripts/tests/fixtures/prune-registry-images/versions.json \
  --manifests-file scripts/tests/fixtures/prune-registry-images/manifests.json \
  --now 1795392000 --keep-latest 3
```

The bats suite `scripts/tests/prune-registry-images.bats` runs this fixture and asserts that
protected tags, referenced tags, child manifests and signatures of kept images are never selected
(`make test-bats`).

## CI

`.github/workflows/registry-prune.yml` runs on a weekly schedule and on manual dispatch only.

- Every run executes the plan step first and writes it to the job summary.
- Scheduled runs never delete.
- To delete, a maintainer dispatches the workflow with `dry_run` set to `false`. The delete step
  runs after the plan step in the same job, with the same limits.
- The job has `contents: read` and `packages: write` only. The package must allow the repository
  to manage it (package settings, "Manage Actions access") for `GITHUB_TOKEN` to list and
  delete its versions.
