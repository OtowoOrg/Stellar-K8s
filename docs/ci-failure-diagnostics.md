# Unified CI Failure Diagnostics Bundle

Introduced for [issue #1151](https://github.com/OtowoOrg/Stellar-K8s/issues/1151).

Failing CI jobs previously uploaded fragmented artifacts (`/tmp/e2e-logs/`,
`/tmp/examples-smoke-triage/`, ad-hoc operator logs). This check consolidates
them into one **diagnostics bundle** with a stable layout so reviewers can
download a single artifact and triage without re-running the job.

## Bundle layout

| Path | Contents |
|------|----------|
| `manifest.json` | Machine-readable metadata (`schema`, `job_name`, `run_id`, `sha`, …) |
| `summary.txt` | Human-readable triage summary |
| `cluster/` | kubectl dumps (pods, operator logs, events, CRDs, nodes) |
| `extras/` | Caller-supplied job logs / triage directories |
| `env/sanitized.env` | Environment snapshot with secret-like keys omitted |

Schema id: `stellar-k8s.ci-diagnostics/v1`.

## Running locally

```bash
# Assemble a bundle without a cluster (unit / dry-run)
./scripts/ci/collect-failure-diagnostics.sh --no-cluster --bundle-dir /tmp/ci-diagnostics-demo

# Include extra triage files
./scripts/ci/collect-failure-diagnostics.sh \
  --bundle-dir /tmp/ci-diagnostics-demo \
  --extra /tmp/examples-smoke-triage \
  --job-name examples-smoke

# Makefile entrypoint
make collect-failure-diagnostics
```

## CI usage

Prefer the composite action over ad-hoc `upload-artifact` steps:

```yaml
- name: Upload failure diagnostics
  if: failure()
  uses: ./.github/actions/collect-failure-diagnostics
  with:
    artifact-name: ci-diagnostics-${{ github.job }}-${{ github.run_id }}
    operator-namespace: stellar-system
    extra-paths: |
      /tmp/examples-smoke-triage
      /tmp/operator.log
```

Jobs currently wired:

- `ci.yml` → `examples-smoke`
- `e2e-quickstart.yml` → quickstart verification
- `verify-operator-boot.yml` → operator boot log path

The existing `.github/actions/collect-e2e-logs` action remains for always-on e2e
log dumps; the failure-diagnostics action is the **unified failure triage**
path.

## Verification

```bash
# Script self-test (no cluster required)
make test-failure-diagnostics
```
