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
- [ ] Review Dependabot PRs (`.github/dependabot.yml`)
- [ ] Verify CI is green on `main`

### Monthly

- [ ] Update Rust toolchain pin in CI (`1.9x` in `.github/actions/setup-rust`)
- [ ] Update `stellar-core` / `horizon` / `soroban-rpc` version defaults in samples
- [ ] Rotate any expiring tokens (Codecov, GHCR)
- [ ] Review and merge Dependabot dependency updates
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
