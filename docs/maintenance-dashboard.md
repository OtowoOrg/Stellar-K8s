# Contributor Maintenance Dashboard

This document is the go-to reference for repository maintainers. It summarizes
health signals, routine tasks, and quick commands needed to keep Stellar-K8s
running smoothly.

---

## Health Signals

| Signal | Where to check | Healthy threshold |
|--------|---------------|-------------------|
| CI pipeline | [GitHub Actions](https://github.com/OtowoOrg/Stellar-K8s/actions/workflows/ci.yml) | All jobs green |
| Test coverage | [Codecov](https://codecov.io/gh/OtowoOrg/Stellar-K8s) | ≥ 70% |
| Dependency audit | `make audit` | 0 unsound advisories |
| Stale artifacts | `make quick` | Exits 0 |
| Docs drift | CI `api-docs` job | Job passes |
| Helm schema | CI `helm-lint` job | Job passes |

---

## Routine Maintenance

### Weekly

- [ ] Review open PRs and triage new issues
- [ ] Check `cargo audit` for new advisories: `make audit`
- [ ] Review Dependabot PRs using the [dependency update process](security/dependency-updates.md) (config: `.github/dependabot.yml`)
- [ ] Verify CI is green on `main`

### Monthly

- [ ] Update Rust toolchain pin in CI (`1.9x` in `.github/actions/setup-rust`)
- [ ] Update `stellar-core` / `horizon` / `soroban-rpc` version defaults in samples
- [ ] Rotate any expiring tokens (Codecov, GHCR)
- [ ] Review and merge Dependabot dependency updates per [dependency-updates.md](security/dependency-updates.md) (CI must be green)
- [ ] Run `make ci-local` and confirm clean
- [ ] Review open issues for stale labels

### On Release

- [ ] Bump version in `Cargo.toml`
- [ ] Run `make changelog` to update `CHANGELOG.md`
- [ ] Regenerate CRD schema: `make crd-gen`
- [ ] Regenerate API docs: `make generate-api-docs`
- [ ] Tag release: `git tag v<VERSION> && git push origin v<VERSION>`
- [ ] Verify release workflow completes in GitHub Actions
- [ ] Update Helm chart `appVersion` in `charts/stellar-operator/Chart.yaml`

---

## Quick Commands

```bash
# Full local CI pass
make ci-local

# Fast pre-commit check
make quick

# Regenerate API docs after CRD changes
make crd-gen && make generate-api-docs

# Check for security advisories
make audit

# Run specific test suite
cargo test controller::pruning_worker -- --nocapture

# Lint Helm chart
make helm-lint

# Check docs links
make link-check
```

---

## Stale Artifact Cleanup

The operator's pruning worker (`src/controller/pruning_worker.rs`) manages
history archive cleanup. To verify the cleanup logic is healthy:

```bash
# Run regression tests for artifact cleanup
cargo test stale_artifact_cleanup -- --nocapture
```

Safety constraints enforced by the pruning worker:
- Always retains ≥ `min_checkpoints` recent checkpoints
- Never deletes within `max_age_days` of the latest checkpoint
- Dry-run is enabled by default (`--force` required for actual deletion)

---

## Adding a New Maintainer

1. Add to `.github/CODEOWNERS`
2. Grant "Maintain" role in GitHub repository settings
3. Share this document and `CONTRIBUTING.md`
4. Run `make dev-setup` to install toolchain and pre-commit hooks

---

## Escalation

For security issues, follow the process in `SECURITY.md`.
For production incidents, see `docs/operations/incident-response.md`.

## Planned-maintenance orchestration: verification gates (#1961)

The `MaintenancePlan` controller (`src/controller/maintenance/plan_engine.rs`, backend in `plan_controller.rs`)
sequences `Planned → Prewarming → Verifying → Draining (→ Stalled) → Verifying → Completed`. Beyond the existing
PDB-aware batching, SLO gate and stall recovery, these gates are enforced:

| Gate | Behaviour |
|---|---|
| **Capacity pre-check** | Before anything is cordoned/evicted, the pods on the target nodes must fit the free capacity of the remaining schedulable nodes (CPU and memory requests; DaemonSet pods excluded). While short, the plan stays `Planned` (condition `CapacityAvailable=False`, so a cluster autoscaler can add nodes) and fails after `prewarm.readinessTimeoutSeconds` with an actionable message. |
| **Observed prewarm readiness** | Requesting a surge is *not* readiness. The backend scales matching Deployments once (recording `stellar.org/maintenance-prewarm-base`) and reports ready only when `readyReplicas >= desired`. A wait longer than `prewarm.readinessTimeoutSeconds` fails the plan **before any node is cordoned**. Workloads not backed by a Deployment (e.g. StatefulSets) are not surged; replacement readiness is checked from pod placement (enough Ready replicas off the node for *all* pods on it). |
| **Post-drain verification** | Before `Completed`: drained nodes hold no non-DaemonSet pods and are still cordoned; every affected workload has all desired replicas Ready; every PDB has `currentHealthy >= minAvailable`. Unresolved checks keep the plan in `Verifying` (condition `PostDrainVerified=False`, `status.message` lists them) and fail it after the timeout, **keeping** surge replicas to protect availability. |
| **Surge release** | After verified completion, surge (and stall-recovery) replicas are scaled back to the recorded base and the annotation is removed (idempotent). Aborted plans keep any `autoRecover` surge by design. |
| **Scope** | Readiness, SLO and verification are limited to `status.affectedWorkloads` (workloads that had pods on the target nodes), so an unrelated unhealthy Deployment cannot block or abort a plan. |

Restart safety: all gate state lives in `status` (`postVerifyStartedUnix`, `affectedWorkloads`, `prewarmedWorkloads`, cursor);
each reconcile is a pure function of `spec`, `status` and the observed cluster.

### Metrics (per plan; derived from status on each reconcile)

`stellar_maintenance_plan_phase{namespace,name,phase}`, `..._nodes_completed`, `..._nodes_failed`, `..._pdb_stalls`,
`..._patch_cycle_ms`, `..._cold_window_ms`, `..._user_facing_errors`. Events: `Prewarmed`, `Cordoned`, `Evicted`, phase transitions.

### Known limitations

* `slo().errorRate` is **not measured** by the Kubernetes backend (no metrics source is wired) and reports 0; `availability`
  is the real Ready/desired ratio of affected Deployments. `userFacingErrors` and `coldWindowMs` are likewise 0 outside the simulator.
* Cordoning is per wave; pods rescheduled during a wave may land on later target nodes and be evicted again. Provide spare
  schedulable capacity (or cordon targets in advance) to avoid churn.
* The plan does not uncordon completed nodes (patching/uncordon is the operator's step); a full-cluster plan therefore needs spare capacity.
* Surge uses Deployment `replicas`; it will fight an HPA that also manages the same Deployment.
* Not implemented: drain/PDB-stall/prewarm duration histograms (only counts and patch-cycle time), and any autoscaler API integration
  beyond waiting for capacity to appear.
* Simulation (`SimulatedCluster`) models rescheduling only when `reschedule_evicted = true`; earlier behaviour (evicted pods never
  return) cannot complete a large drain.
