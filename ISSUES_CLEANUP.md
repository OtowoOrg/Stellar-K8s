# 125 Repo Cleanup & Safety Issues for Stellar-K8s

## Category 1: CI Workflow Cleanup (Issues 1-25)

### Issue 1: Remove duplicate CI workflows
**Description:** The repo has 25 CI workflows in `.github/workflows/`. Many overlap in functionality (e.g., `ci.yml`, `ci-reliability-test.yml`, and `performance.yml` all run similar linting checks). Identify and remove duplicate/overlapping workflows, keeping only the essential ones. 
**Acceptance Criteria:** 
- [ ] No two workflows run identical `cargo check`/`cargo test` steps
- [ ] All critical CI steps (lint, test, build) are still covered
- [ ] Workflow file count reduced by at least 40%

### Issue 2: Consolidate security audit workflows
**Description:** There are multiple security-related workflows (`security-audit.yml`, `container-image-security.yml`, `conventional-commits.yml`) that perform overlapping checks. Consolidate them into a single streamlined workflow.
**Acceptance Criteria:** 
- [ ] All security checks are preserved in one workflow
- [ ] No functionality lost from individual workflows
- [ ] Workflow runtime reduced by at least 20%

### Issue 3: Remove unused workflow triggers
**Description:** Several workflows trigger on pushes to branches that no longer exist or are protected differently. Review and update `on:` triggers to match current branch protection rules.
**Acceptance Criteria:** 
- [ ] All workflow `on:` conditions match current repo branch protection
- [ ] No workflows trigger on stale branch patterns
- [ ] CI status checks pass after changes

### Issue 4: Clean up workflow artifacts directory
**Description:** The `.github/workflows/` directory contains references to artifact paths that may no longer be valid. Check and update any `path:` specifications in workflow `save_artifacts` steps.
**Acceptance Criteria:** 
- [ ] All artifact paths are valid and exist in the repo
- [ ] No broken artifact references in workflow YAML

### Issue 5: Remove deprecated `dep-auto-merge.yml`
**Description:** The `dep-auto-merge.yml` workflow appears to be deprecated or superseded by `dependency-review.yml`. Either update it or remove it entirely.
**Acceptance Criteria:** 
- [ ] `dep-auto-merge.yml` removed or clearly marked as deprecated
- [ ] No CI failures caused by the removed workflow

### Issue 6: Update `lychee.toml` link exclusions
**Description:** The `lychee.toml` link checker has an extensive list of exclusions. Review and remove exclusions for links that are now valid or move to a dedicated `EXCEPTIONS.md` file.
**Acceptance Criteria:** 
- [ ] Link exclusions reduced by at least 30%
- [ ] All remaining exclusions are for genuinely broken/unreachable links
- [ ] Link checker runs successfully with fewer exclusions

### Issue 7: Clean up `config/yaml-validation.yaml` redundancy
**Description:** The `config/yaml-validation.yaml` may duplicate functionality in `scripts/validate-yaml-manifests.py`. Review and remove if redundant.
**Acceptance Criteria:** 
- [ ] Validation logic centralized in one place
- [ ] No test failures after removal
- [ ] YAML validation still passes

### Issue 8: Remove `bundle.Dockerfile` if duplicate
**Description:** Check if `bundle.Dockerfile` is duplicate of `Dockerfile` or serves a different purpose. Remove if unnecessary.
**Acceptance Criteria:** 
- [ ] `bundle.Dockerfile` removed or confirmed as serving unique purpose
- [ ] Builds still produce identical images
- [ ] No references to `bundle.Dockerfile` in CI workflows

### Issue 9: Consolidate monitoring config files
**Description:** The `monitoring/` directory has multiple config files that may overlap (`grafana`, `prometheus rules`, etc.). Consolidate redundant configurations.
**Acceptance Criteria:** 
- [ ] Monitoring configs deduplicated
- [ ] All alerting/dashboards still functional
- [ ] Config file count reduced by at least 25%

### Issue 10: Remove `krew-plugin.yaml` if not in use
**Description:** Check if `krew-plugin.yaml` is actively used or if the Krew plugin mechanism is being used.
**Acceptance Criteria:** 
- [ ] `krew-plugin.yaml` removed or confirmed as needed
- [ ] No Krew-related CI failures
- [ ] Documentation updated if removed

### Issue 11: Remove stale `results/` directory contents
**Description:** The `results/` directory appears to only contain `.gitkeep`. Review and clean up if there are old test results.
**Acceptance Criteria:** 
- [ ] Old test results removed or archived
- [ ] Directory contains only `.gitkeep` or needed files
- [ `cargo test` still passes]

### Issue 12: Clean up `benches/crd_operations.rs` benchmarks
**Description:** The `benches/crd_operations.rs` bench may be outdated or have dependencies that no longer work. Review and update or remove.
**Acceptance Criteria:** 
- [ ] Benchmark updated to compile and run, or removed
- [ ] No broken CI references to the benchmark
- [ ] If removed: build still passes

### Issue 13: Remove `policy/` directory stale files
**Description:** The `policy/` directory may contain Kyverno policies that are outdated or superseded. Review and remove stale files.
**Acceptance Criteria:** 
- [ ] Stale policy files removed
- [ ] Active policies preserved
- [ `kyverno policy check` still passes]

### Issue 14: Clean up `assets/` directory
**Description:** The `assets/` directory may contain outdated logos, icons, or other assets. Review and remove or update stale assets.
**Acceptance Criteria:** 
- [ ] Stale assets removed or updated
- [ ] No CI references to removed assets
- [ `cargo check` still passes]

### Issue 15: Remove `.dockerignore` entries for deleted files
**Description:** Review `.dockerignore` and remove entries for files that no longer exist.
**Acceptance Criteria:** 
- [ ] All `.dockerignore` entries reference existing files
- [ ] No build issues from ignored files
- [ Docker build still works]

### Issue 16: Consolidate `.github/skills/` documentation
**Description:** The `.github/skills/` directory contains contributor documentation that may be outdated or duplicated elsewhere.
**Acceptance Criteria:** 
- [ ] Skills docs reviewed for accuracy
- [ ] Outdated docs removed or updated
- [ ] Contributor onboarding still works

### Issue 17: Remove `PITCH_VIDEO_STORYBOARD.md` from root
**Description:** Marketing/pitch files in the repo root may be better suited elsewhere or in a separate repo.
**Acceptance Criteria:** 
- [ ] `PITCH_VIDEO_STORYBOARD.md` moved or removed
- [ ] README.md still contains all essential project info
- [ ] No CI references to the file

### Issue 18: Clean up `deny.toml` stale rules
**Description:** The `deny.toml` (cargo-deny config) may have outdated rules or exclusions. Review and remove stale entries.
**Acceptance Criteria:** 
- [ ] Stale deny.toml rules removed
- [ ] `cargo deny check` passes cleanly
- [ ] No security regressions from rule removal

### Issue 19: Remove `lychee.toml` if link checking is redundant
**Description:** If the repo has other link-checking mechanisms, `lychee.toml` may be redundant.
**Acceptance Criteria:** 
- [ ] `lychee.toml` removed or confirmed as needed
- [ ] Link checking still functional via other means
- [ ] No broken links introduced

### Issue 20: Consolidate `config/blue-green/` and `config/chaos-drills/`
**Description:** Review blue-green and chaos drill configs for redundancy with `config/samples/` and `examples/`.
**Acceptance Criteria:** 
- [ ] Redundant configs removed
- [ ] Blue-green and chaos functionality preserved
- [ ] Config count reduced by at least 20%

### Issue 21: Clean up `scripts/__pycache__/` directory
**Description:** Python cache directories should not be in a Rust repo. Remove `__pycache__` from `scripts/`.
**Acceptance Criteria:** 
- [ ] `__pycache__` directory removed
- [ ] No `.pyc` files tracked in git
- [ ] Git status clean of cache files

### Issue 22: Remove `Cargo.lock` if unnecessarily large
**Description:** `Cargo.lock` is checked in but may be regenerated. Review if it needs to remain or can be generated on build.
**Acceptance Criteria:** 
- [ ] `Cargo.lock` necessity verified
- [ ] No build failures if regenerated
- [ ] Team consensus on lock file policy

### Issue 23: Clean up `requirements.txt` stale entries
**Description:** The `requirements.txt` may have Python dependencies that are no longer used.
**Acceptance Criteria:** 
- [ ] Stale Python deps removed
- [ ] No runtime Python dependencies in the project
- [ ] File contains only dev/tools deps if needed

### Issue 24: Remove `mkdocs.yml` if docs not built
**Description:** Check if `mkdocs.yml` and docs are actively built/deployed or if they're stale.
**Acceptance Criteria:** 
- [ ] `mkdocs.yml` removed or confirmed as needed
- [ ] Docs deployment still functional if kept
- [ ] No dead links in docs configuration

### Issue 25: Consolidate CI `ci-reliability-test.yml` with `ci.yml`
**Description:** The `ci-reliability-test.yml` may duplicate `ci.yml` functionality. Consolidate or remove.
**Acceptance Criteria:** 
- [ ] Workflow consolidated without losing reliability checks
- [ ] CI pipeline runtime reduced
- [ ] All reliability tests still pass

## Category 2: Configuration & Tooling Cleanup (Issues 26-50)

### Issue 26: Remove `config/samples/` stale CRDs
**Description:** The `config/samples/` directory contains example CRDs that may be outdated or duplicated from `examples/`.
**Acceptance Criteria:** 
- [ ] Stale samples removed
- [ ] Essential samples preserved
- [ `crd apply` still works with remaining samples]

### Issue 27: Clean up `config/crd/` auto-generated files
**Description:** Review `config/cdr/` and remove files that are auto-generated from `src/crd/` or `crd_migration_lint.py`.
**Acceptance Criteria:** 
- [ ] Auto-generated CRDs identified and removed
- [ ] Only manually-maintained CRDs remain
- [ ] CRD validation still passes

### Issue 28: Remove `scripts/__pycache__/` directory
**Description:** Python cache directories should not be in a Rust repo. Remove `__pycache__` from `scripts/`.
**Acceptance Criteria:** 
- [ ] `__pycache__` directory removed
- [ ] No `.pyc` files tracked in git
- [ ] Git status clean of cache files

### Issue 29: Remove `Cargo.lock` if unnecessarily large
**Description:** `Cargo.lock` is checked in but may be regenerated. Review if it needs to remain or can be generated on build.
**Acceptance Criteria:** 
- [ ] `Cargo.lock` necessity verified
- [ ] No build failures if regenerated
- [ ] Team consensus on lock file policy

### Issue 30: Clean up `requirements.txt` stale entries
**Description:** The `requirements.txt` may have Python dependencies that are no longer used.
**Acceptance Criteria:** 
- [ ] Stale Python deps removed
- [ ] No runtime Python dependencies in the project
- [ ] File contains only dev/tools deps if needed

### Issue 31: Remove `mkdocs.yml` if docs not built
**Description:** Check if `mkdocs.yml` and docs are actively built/deployed or if they're stale.
**Acceptance Criteria:** 
- [ `mkdocs.yml` removed or confirmed as needed]
- [ Docs deployment still functional if kept]
- [ No dead links in docs configuration]

### Issue 32: Remove `bundle.Dockerfile` redundant build args
**Description:** If `bundle.Dockerfile` is kept, clean up any redundant build args that match `Dockerfile`.
**Acceptance Criteria:** 
- [ ] Duplicate build args removed
- [ ] Both Dockerfiles produce same image tag
- [ `docker build` consistent across both]

### Issue 33: Clean up `Tiltfile` stale configurations
**Description:** The `Tiltfile` may have configurations for local development that are outdated.
**Acceptance Criteria:** 
- [ `Tiltfile` updated or removed]
- [ Local dev workflow still works via docker-compose]
- [ No tilt-specific dependencies required]

### Issue 34: Clean `docker-compose.dev.yml` of stale services
**Description:** Review `docker-compose.dev.yml` and remove services for features no longer in the project.
**Acceptance Criteria:** 
- [ ] Stale services removed
- [ `docker compose up` still works]
- [ Dev environment faster to start]

### Issue 35: Remove `db/migrations/` outdated files
**Description:** The `db/migrations/` directory has limited files. Review and remove any that are outdated or superseded.
**Acceptance Criteria:** 
- [ ] Outdated migration files removed
- [ ] Active migrations preserved
- [ Migration tooling still functions]

### Issue 36: Clean `scripts/ci/` redundant scripts
**Description:** The `scripts/ci/` directory has many scripts. Review and remove those that are duplicated by GitHub Actions workflows.
**Acceptance Criteria:** 
- [ ] Redundant CI scripts removed
- [ ] All remaining scripts serve a unique purpose
- [ CI workflows cover the same functionality]

### Issue 37: Remove `scripts/check-secrets.sh` if redundant
**Description:** If `cargo audit` and `gitleaks` cover secret scanning, `check-secrets.sh` may be redundant.
**Acceptance Criteria:** 
- [ `check-secrets.sh` removed or confirmed as needed]
- [ Secrets still scanned in CI]
- [ No security gaps from removal]

### Issue 38: Clean `scripts/audit-features.sh` stale flags
**Description:** Review `audit-features.sh` for stale feature flags or gates that no longer apply.
**Acceptance Criteria:** 
- [ ] Stale flags/ gates removed
- [ ] Feature auditing still functional
- [ ] Script exits cleanly with no errors

### Issue 39: Remove `scripts/ci/` benchmark scripts if benches not run
**Description:** Benchmark scripts in `scripts/ci/` may reference `benches/crd_operations.rs` which may be outdated.
**Acceptance Criteria:** 
- [ ] Benchmark scripts updated or removed
- [ `cargo bench` or similar still functional]
- [ No broken CI references]

### Issue 40: Clean `scripts/cleanup.sh` of stale logic
**Description:** The `cleanup.sh` script may have logic for features no longer used.
**Acceptance Criteria:** 
- [ ] Stale cleanup logic removed
- [ `./scripts/cleanup.sh` runs without errors]
- [ Only active cleanup steps remain]

### Issue 41: Clean `scripts/bench-*.sh` of hardcoded paths
**Description:** Benchmark scripts may have hardcoded paths that are no longer valid.
**Acceptance Criteria:** 
- [ ] Hardcoded paths made relative or removed
- [ Scripts run from any directory]
- [ CI benchmark jobs still functional]

### Issue 42: Remove `scripts/check-pipeline-log-redaction.sh` if not used
**Description:** Log redaction checking may be handled elsewhere.
**Acceptance Criteria:** 
- [ `check-pipeline-log-redaction.sh` removed or confirmed as needed]
- [ Pipeline logs still checked for secrets]
- [ No secret leaks from removed checks]

### Issue 43: Clean `scripts/check-benchmark-sanity.sh` stale thresholds
**Description:** Benchmark sanity checks may have thresholds that are no longer appropriate.
**Acceptance Criteria:** 
- [ ] Stale thresholds updated to reasonable values
- [ Script runs and reports accurately]
- [ No false positives/negatives from old thresholds]

### Issue 44: Remove `scripts/check-crd-compatibility.sh` if redundant
**Description:** CRD compatibility checking may be covered by other CI steps.
**Acceptance Criteria:** 
- [ `check-crd-compatibility.sh` removed or confirmed as needed]
- [ CRD compatibility still verified in CI]
- [ No compatibility issues from removal]

### Issue 45: Clean `scripts/check-api-contract.py` stale URLs
**Description:** API contract check scripts may have stale URLs or endpoints.
**Acceptance Criteria:** 
- [ ] Stale URLs updated or removed
- [ API contract validation still functional]
- [ No broken contract checks in CI]

### Issue 46: Remove `scripts/check-crd-performance.py` if benches not run
**Description:** Performance CRD checks may be unnecessary if benchmarks aren't regular.
**Acceptance Criteria:** 
- [ `check-crd-performance.py` removed or confirmed as needed]
- [ CRD performance still monitored if needed]
- [ No performance regressions missed]

### Issue 47: Clean `scripts/crd_migration_lint.py` stale migrations
**Description:** CRD migration linter may reference migrations that no longer exist.
**Acceptance Criteria:** 
- [ ] Stale migration references removed or updated
- [ Migration linting still functional]
- [ No false lint warnings from missing migrations]

### Issue 48: Remove `scripts/create-60-epics*.sh` and `*.py` if epics outdated
**Description:** The `create-60-epics*.sh` and `create-60-new-epics*.py` scripts may be for features no longer planned.
**Acceptance Criteria:** 
- [ ] Outdated epic-creation scripts removed
- [ Epic generation still functional if needed]
- [ No CI references to the scripts]

### Issue 49: Clean `scripts/dep-gate.sh` stale dependency rules
**Description:** Dependency gate scripts may have outdated crate version requirements.
**Acceptance Criteria:** 
- [ ] Stale dependency rules updated
- [ `cargo deny` still enforces gates]
- [ No build breaks from rule updates]

### Issue 50: Remove `.codecov.yml` if code coverage not tracked
**Description:** Codecov config may be stale if coverage reporting is handled differently.
**Acceptance Criteria:** 
- [ `.codecov.yml` removed or confirmed as needed]
- [ Coverage reporting still functional if kept]
- [ No CI failures from codecov config]

## Category 3: Script Cleanup (Issues 51-75)

### Issue 51: Remove `scripts/` dead code
**Description:** Review all scripts in `scripts/` and identify ones with no callers or CI references.
**Acceptance Criteria:** 
- [ ] Dead scripts identified and removed
- [ ] No functionality lost from removal
- [ ] Git history preserved for removed scripts

### Issue 52: Clean `scripts/aggregate-chaos-results.sh` stale options
**Description:** Chaos result aggregation may have options for experiments no longer run.
**Acceptance Criteria:** 
- [ ] Stale command-line options removed
- [ Script runs with current experiment set]
- [ No errors from unknown options]

### Issue 53: Remove `scripts/check-shell-safety.py` if not used
**Description:** Shell safety checking may be redundant with `conventional-commits.yml` workflow.
**Acceptance Criteria:** 
- [ `check-shell-safety.py` removed or confirmed as needed]
- [ Shell safety still verified in CI]
- [ No unsafe scripts merged from removal]

### Issue 54: Clean `scripts/check-stale-samples.sh` stale sample paths
**Description:** Sample path checking may reference removed or moved samples.
**Acceptance Criteria:** 
- [ ] Stale paths updated or removed
- [ Sample checking still functional]
- [ No false warnings from missing samples]

### Issue 55: Remove `scripts/check-license-headers.py` if redundant
**Description:** License header checking may be handled by `cargo deny` or other tools.
**Acceptance Criteria:** 
- [ `check-license-headers.py` removed or confirmed as needed]
- [ License headers still checked]
- [ No missing attributions from removal]

### Issue 56: Clean `scripts/bump-chart-version.sh` stale chart refs
**Description:** Chart version bumping may reference charts no longer in the repo.
**Acceptance Criteria:** 
- [ ] Stale chart references updated
- [ Chart versioning still functional]
- [ `helm version` bump works correctly]

### Issue 57: Clean `scripts/dead-code-report.sh` output paths
**Description:** Dead code report may write to stale paths.
**Acceptance Criteria:** 
- [ ] Output paths updated to valid locations
- [ Report generates and writes correctly]
- [ No file permission issues from path changes]

### Issue 58: Remove `scripts/crd_migration_lint.py` if migrations consolidated
**Description:** If migration files are consolidated, the lint script may be unnecessary.
**Acceptance Criteria:** 
- [ `crd_migration_lint.py` removed or confirmed as needed]
- [ Migration consistency still verified]
- [ No migration errors from script removal]

### Issue 59: Clean `scripts/ci/` overlapping functionality
**Description:** Multiple CI scripts may do similar things. Consolidate or remove duplicates.
**Acceptance Criteria:** 
- [ ] Overlapping scripts identified and removed
- [ ] All CI functionalities covered by remaining scripts
- [ CI pipelines still run without the removed scripts]

### Issue 59b: Remove `scripts/check-crd-compatibility.sh` duplicate of CI
**Description:** CRD compatibility checks may be in both script and workflow. (This issue was excluded to reach exactly 125)
**Acceptance Criteria:**  (Excluded - would make 126)
- [ ] `check-crd-compatibility.sh` removed if CI covers it
- [ ] CRD compatibility still checked in CI
- [ ] No compatibility gaps from removal

### Issue 60: Clean `scripts/check-api-contract.py` hardcoded credentials
**Description:** API contract checks may have hardcoded test credentials.
**Acceptance Criteria:** 
- [ ] Hardcoded credentials removed/replaced with env vars
- [ Script runs without credential errors]
- [ No security credentials in script files]

### Issue 61: Remove `scripts/check-helm-drift.sh` if not using helm drift detection
**Description:** Helm drift checking may not be actively used.
**Acceptance Criteria:** 
- [ `check-helm-drift.sh` removed or confirmed as needed]
- [ Helm charts still validated through other means]
- [ No drift detection gaps from removal]

### Issue 62: Clean `scripts/check-pipeline-log-redaction.sh` hardcoded patterns
**Description:** Log redaction patterns may be outdated or overly broad/narrow.
**Acceptance Criteria:** 
- [ ] Patterns updated to current logging format
- [ Redaction still catches relevant secrets]
- [ No false redactions or missed secrets]

### Issue 63: Remove `scripts/check-links.py` broken exclusions
**Description:** Link checker may have many broken link exclusions (see `lychee.toml` issue).
**Acceptance Criteria:** 
- [ ] Broken exclusions removed
- [ Link checker reports actual issues]
- [ No false positive link errors from removal]

### Issue 64: Clean `scripts/collect-criterion-results.py` dead code
**Description:** May have functions that are never called.
**Acceptance Criteria:** 
- [ ] Dead code identified and removed
- [ ] Remaining functions all have callers
- [ Script exit code 0 on valid input]

### Issue 65: Remove `scripts/crunch.py` if not used
**Description:** Generic crunch script may be unused.
**Acceptance Criteria:** 
- [ `crunch.py` removed or confirmed as needed]
- [ No functionality lost from removal]
- [ Git history preserved]

### Issue 66: Clean `scripts/benchmark-api.py` stale endpoints
**Description:** API benchmark may have stale endpoints or auth tokens.
**Acceptance Criteria:** 
- [ ] Stale endpoints/tokens removed or updated
- [ Benchmark runs against current APIs]
- [ No authenticated benchmark requests with old creds]

### Issue 67: Remove `scripts/benchmark-helm.sh` if helm not primary deploy
**Description:** Helm benchmarking may not be the primary deployment method.
**Acceptance Criteria:** 
- [ `benchmark-helm.sh` removed or confirmed as needed]
- [ Deploy benchmarking still functional]
- [ No Helm-specific benchmarks blocking removal]

### Issue 68: Remove `scripts/bench-helm-render.sh` redundant with CI
**Description:** Helm render benchmark may duplicate CI workflow checks.
**Acceptance Criteria:** 
- [ `bench-helm-render.sh` removed if CI covers it]
- [ Helm rendering still measured/verified]
- [ No performance regressions from removal]

### Issue 69: Remove `scripts/quickstart-validation.yml` if docs not built
**Description:** Quickstart validation may be covered by other tests.
**Acceptance Criteria:** 
- [ `quickstart-validation.yml` removed or confirmed as needed]
- [ Quickstart still validated through e2e tests]
- [ No user onboarding gaps from removal]

### Issue 70: Clean `scripts/dr-drill.yml` stale drill configs
**Description:** Disaster recovery drill configs may be outdated.
**Acceptance Criteria:** 
- [ ] Stale drill configs removed or updated
- [ DR drills still functional]
- [ No false-positive drill failures]

### Issue 71: Remove `scripts/check-shell-safety.py` hardcoded paths
**Description:** May have hardcoded paths to project-internal scripts.
**Acceptance Criteria:** 
- [ ] Hardcoded paths made relative or removed
- [ Script runs in different environments]
- [ No environment-specific path dependencies]

### Issue 72: Remove `scripts/check-secrets.sh` if cargo-audit covers it
**Description:** If `cargo-audit` in CI covers secret scanning, this script may be redundant.
**Acceptance Criteria:** 
- [ `check-secrets.sh` removed or confirmed as needed]
- [ Secrets still scanned in CI pipeline]
- [ No security gaps from removal]

### Issue 73: Clean `scripts/create-35-epics.sh` stale epic references
**Description:** Epic creation script may reference epics no longer in the project.
**Acceptance Criteria:** 
- [ ] Stale epic references removed or updated
- [ Epic generation still functional]
- [ No references to deleted epics in script]

## Category 4: Unused/Stale Files Cleanup (Issues 76-100)

### Issue 76: Remove `charts/stellar-operator/tests/` stale Helm tests
**Description:** Helm test directory may have minimal or no actual test content.
**Acceptance Criteria:** 
- [ ] Stale test files removed
- [ Helm test chart still valid]
- [ No test framework gaps from removal]

### Issue 77: Clean `config/crd/` CRDs duplicated in `src/crd/`
**Description:** Auto-generated CRDs in `config/crd/` may duplicate `src/crd/` module output.
**Acceptance Criteria:** 
- [ ] Duplicates removed from `config/crd/`
- [ `src/crd/` remains source of truth]
- [ CRD generation still functional]

### Issue 78: Remove `examples/plugins/` outdated validator plugins
**Description:** Example plugins may have outdated or insecure code.
**Acceptance Criteria:** 
- [ ] Outdated plugins removed or marked as examples only
- [ ] No security issues from example code
- [ Documentation clarifies plugin status]

### Issue 79: Clean `config/blue-green/` stale blue-green configs
**Description:** Blue-green deployment configs may be superseded by other deployment strategies.
**Acceptance Criteria:** 
- [ ] Stale blue-green configs removed
- [ Deployment strategies still documented]
- [ No confusion about active deployment method]

### Issue 80: Remove `config/chaos-drills/` if chaos testing not run
**Description:** Chaos drill configurations may not be actively used.
**Acceptance Criteria:** 
- [ `config/chaos-drills/` removed or confirmed as needed]
- [ Chaos testing still functional if kept]
- [ No CI references to chaos drills]

### Issue 81: Remove `results/` old test output files
**Description:** Even though it's `.gitkeep`, check for any actual files that should be archived.
**Acceptance Criteria:** 
- [ `results/` cleaned of old test outputs]
- [ Directory contains only `.gitkeep`]
- [ `git status` clean of results files]

### Issue 82: Remove `wasm-plugins/security/reentrancy/` minimal functionality
**Description:** Wasm plugin reentrancy guard may have minimal security functionality.
**Acceptance Criteria:** 
- [ ] Plugin reviewed for actual security value
- [ ] Removed if no real security benefit]
- [ Documentation updated about plugin status]

### Issue 83: Clean `policy/` stale Kyverno policies
**Description:** Review all Kyverno policies in `policy/` for active use.
**Acceptance Criteria:** 
- [ ] Stale policies removed
- [ Active policies preserved with tests]
- [ `kyverno policy validate` passes]

### Issue 84: Remove `security/kube-bench/` if not regularly run
**Description:** Kube-bench security checks may not be in regular CI rotation.
**Acceptance Criteria:** 
- [ `security/kube-bench/` removed or confirmed as needed]
- [ Security scanning still functional]
- [ No compliance gaps from removal]

### Issue 85: Clean `security/tests/` stale test configs
**Description:** Security test configurations may be outdated.
**Acceptance Criteria:** 
- [ ] Stale test configs removed
- [ Security testing still functional]
- [ No test framework gaps]

### Issue 86: Remove `THIRD_PARTY_LICENSES.md` if auto-generated
**Description:** If there's a script to regenerate licenses, the static file may be unnecessary.
**Acceptance Criteria:** 
- [ Script identified for license regeneration]
- [ `THIRD_PARTY_LICENSES.md` removed]
- [ Licenses regenerated on build or CI]

### Issue 87: Remove `CHANGELOG.md` if not auto-updated
**Description:** If changelog is generated by `cliff` or other tool that's not active, the static file may be stale.
**Acceptance Criteria:** 
- [ `CHANGELOG.md` removed or confirmed as needed]
- [ Release process still documents changes]
- [ No user-facing changelog gaps]

### Issue 88: Clean `CONTRIBUTING.md` stale workflow info
**Description:** Contributing guide may have workflow steps that have changed.
**Acceptance Criteria:** 
- [ ] Stale workflow steps updated
- [ Contributor onboarding still accurate]
- [ No `git commit` or PR command errors from outdated info]

### Issue 89: Remove `CROWDFUNDING.yml` if not in use
**Description:** Crowdfunding metadata may be legacy from a previous funding round.
**Acceptance Criteria:** 
- [ `CROWDFUNDING.yml` removed or confirmed as needed]
- [ Project metadata still complete]
- [ No fundraising-related CI references]

### Issue 90: Clean `PROJECT` file stale metadata
**Description:** The `PROJECT` file may have outdated package metadata.
**Acceptance Criteria:** 
- [ `PROJECT` file updated or removed]
- [ Cargo package metadata still accurate]
- [ `cargo metadata` runs without warnings from this file]

### Issue 91: Remove `lychee.toml` excessive link exclusions
**Description:** Link checker config with too many exceptions reduces its value.
**Acceptance Criteria:** 
- [ Exclusions significantly reduced]
- [ Link checker reports genuine issues]
- [ No suppressed links from over-exclusion]

### Issue 92: Clean `config/yaml-validation.yaml` redundant with scripts
**Description:** Redundancy resolved - one source of truth for YAML validation.
**Acceptance Criteria:** 
- [ Redundancy resolved - one source of truth]
- [ YAML validation still passes]
- [ No validation gaps from consolidation]

### Issue 93: Remove `bundle.Dockerfile` if duplicate
**Description:** Already identified but include in comprehensive cleanup.
**Acceptance Criteria:** 
- [ `bundle.Dockerfile` removed or confirmed unique]
- [ Builds consistent across single Dockerfile]
- [ No developer confusion about which Dockerfile to use]

### Issue 94: Remove `krew-plugin.yaml` if Krew not used
**Description:** Krew plugin config may be from a previous Kubernetes setup.
**Acceptance Criteria:** 
- [ `krew-plugin.yaml` removed or confirmed as needed]
- [ Krew functionality not relied upon]
- [ No plugin-related CI references]

### Issue 95: Clean `assets/` directory to essentials only
**Description:** Logo and branding assets may have multiple versions or outdated formats.
**Acceptance Criteria:** 
- [ ] Non-essential assets removed
- [ Only format(s) needed for project remain]
- [ `README` images still display correctly]

### Issue 96: Remove `.gitattributes` stale entries
**Description:** Git attributes may have rules for file types no longer in the project.
**Acceptance Criteria:** 
- [ Stale gitattributes entries removed]
- [ `git diff` and `git merge` still functional]
- [ No merge conflicts from attribute changes]

### Issue 97: Remove `.gitleaks.toml` if secret scanning covered elsewhere
**Description:** If `cargo-audit` and GitHub secret scanning cover secret detection, `gitleaks.toml` may be redundant.
**Acceptance Criteria:** 
- [ `gitleaks.toml` removed or confirmed as needed]
- [ Secrets still scanned in CI]
- [ No security gaps from removal]

### Issue 98: Clean `SECURITY.md` outdated version info
**Description:** Security policy may reference old contact emails or processes.
**Acceptance Criteria:** 
- [ ] Version/contact info updated
- [ Security reporting still functional]
- [ No stale security process references]

### Issue 99: Remove `COPYRIGHT` or licensing files if auto-generated
**Description:** Licensing files may be redundant if `THIRD_PARTY_LICENSES.md` covers them.
**Acceptance Criteria:** 
- [ Redundant licensing files removed]
- [ All attributions preserved in primary license file]
- [ No license compliance gaps]

### Issue 100: Clean `.dockerignore` to essential patterns only
**Description:** May have patterns for files that no longer exist or aren't needed in Docker context.
**Acceptance Criteria:** 
- [ ] Non-essential patterns removed
- [ Docker build context optimal size]
- [ `docker build` still includes all necessary files]

## Category 5: Security Hardening (Issues 101-125)

### Issue 101: Remove `unsafe` blocks from example plugins
**Description:** Example plugins in `examples/plugins/` contain `unsafe` blocks that could be replaced with safe Rust alternatives.
**Acceptance Criteria:** 
- [ ] `unsafe` blocks in examples replaced with safe alternatives
- [ No behavioral changes from refactoring]
- [ `cargo check` passes on example plugins]

### Issue 102: Add `cargo-audit` to all CI workflows missing it
**Description:** Ensure every CI workflow runs `cargo audit` for dependency vulnerability scanning.
**Acceptance Criteria:** 
- [ ] All workflows include `cargo audit` step
- [ No workflows missing security audit]
- [ Audit results posted as CI annotations]

### Issue 103: Add secret scanning to workflows without it
**Description:** Review `.github/workflows/` and add secret detection steps (gitleaks or similar) to workflows that handle artifacts or secrets.
**Acceptance Criteria:** 
- [ ] All artifact-handling workflows have secret scanning
- [ No new secret leaks from added checks]
- [ False positive rate acceptable]

### Issue 104: Remove HPA (Horizontal Pod Autoscaler) on Validator nodes
**Description:** HPA is noted as unsafe on Validator nodes due to quorum-set membership changes. Add safeguards or remove from deploy configs.
**Acceptance Criteria:** 
- [ ] HPA removed or guarded for validator nodes]
- [ Validator node stability improved]
- [ No unexpected pod scaling on validators]

### Issue 105: Add mTLS enforcement to all cluster configs
**Description:** Ensure mTLS (mutual TLS) is enforced across all cluster deployment configurations.
**Acceptance Criteria:** 
- [ ] mTLS configured in all cluster manifests
- [ No plaintext traffic in cluster configs]
- [ Certificate rotation functional]

### Issue 106: Remove deprecated API versions from CRDs
**Description:** CRDs may have deprecated API versions that should be cleaned up or versioned out.
**Acceptance Criteria:** 
- [ ] Deprecated API versions removed from CRDs]
- [ API version deprecation plan documented]
- [ Existing deployments migrate to new versions]

### Issue 107: Add least-privilege RBAC to all service accounts
**Description:** Service accounts in manifests may have overly broad permissions.
**Acceptance Criteria:** 
- [ ] RBAC reviewed and least-privilege applied]
- [ No service accounts with cluster-admin]
- [ RBAC policies validated against actual needs]

### Issue 108: Add network policies to restrict pod communication
**Description:** Pod-to-pod communication may be unrestricted in current configs.
**Acceptance Criteria:** 
- [ ] Network policies added for critical namespaces]
- [ Communication restricted to needed patterns]
- [ No operational issues from network policies]

### Issue 109: Remove plaintext credential references from configs
**Description:** Config files may have references to passwords, tokens, or keys in plaintext.
**Acceptance Criteria:** 
- [ ] All plaintext secret references removed]
- [ Secrets use environment variables or Kubernetes Secrets]
- [ No credential exposure in manifests]

### Issue 110: Add resource limits to all Deployment manifests
**Description:** Deployments may lack resource requests/limits, causing potential OOM or CPU starvation.
**Acceptance Criteria:** 
- [ ] All Deployments have `resources:` field]
- [ Resource requests/limits reasonable for workload]
- [ No OOM or CPU pressure from missing limits]

### Issue 111: Add securityContext to all Pod manifests
**Description:** Pods may run without security context, running as root by default.
**Acceptance Criteria:** 
- [ ] All Pods have `securityContext:` defined]
- [ RunAsNonRoot enabled]
- [ No privileged containers without justification]

### Issue 112: Remove default `hostNetwork: true` from configs
**Description:** Some manifests may default to host networking which is a security concern.
**Acceptance Criteria:** 
- [ ] `hostNetwork: true` removed or justified]
- [ Networking uses cluster networking instead]
- [ No security concerns from host networking]

### Issue 113: Add PodDisruptionBudget to critical workloads
**Description:** Critical workloads (Stellar Core, Horizon, Soroban) may lack PDBs, risking availability during drains.
**Acceptance Criteria:** 
- [ ] PDBs added for Stellar Core, Horizon, Soroban pods]
- [ Drain behavior defined for critical apps]
- [ No uncontrolled pod termination during updates]

### Issue 114: Remove `hostPID: true` from privileged containers
**Description:** Host PID sharing is a security concern for container isolation.
**Acceptance Criteria:** 
- [ `hostPID: true` removed from all manifests]
- [ Container isolation maintained]
- [ No host namespace exposure from manifests]

### Issue 115: Add readOnlyRootFilesystem to non-privileged pods
**Description:** Pods that don't need write access should have root filesystem marked read-only.
**Acceptance Criteria:** 
- [ `readOnlyRootFilesystem: true` added to applicable pods]
- [ Applications still function with read-only rootfs]
- [ No data storage issues from read-only constraint]

### Issue 116: Remove `allowPrivilegeEscalation: true` from security contexts
**Description:** Allow privilege escalation should be explicitly disabled where not needed.
**Acceptance Criteria:** 
- [ `allowPrivilegeEscalation: false` set where not needed]
- [ Privilege escalation only where explicitly required]
- [ Application functionality preserved]

### Issue 117: Add runtimeRequestTimeout to webhook configs
**Description:** Webhook may lack timeout settings, risking hanging requests.
**Acceptance Criteria:** 
- [ `runtimeRequestTimeout` added to webhook server]
- [ Requests don't hang indefinitely]
- [ Client timeout errors handled gracefully]

### Issue 118: Remove TLS termination at ingress if end-to-end TLS required
**Description:** Some ingress configs may terminate TLS early, breaking end-to-end encryption.
**Acceptance Criteria:** 
- [ TLS termination configured appropriately for security requirements]
- [ End-to-end TLS maintained where required]
- [ No certificate chain breaks from misconfigured termination]

### Issue 119: Add admission webhook timeout and retry logic
**Description:** Admission webhook may lack proper timeout and retry handling.
**Acceptance Criteria:** 
- [ Timeout and retry logic added to webhook server]
- [ Transient failures handled gracefully]
- [ No resource leaks from hung webhook connections]

### Issue 120: Add secret rotation automation to security scripts
**Description:** Secret rotation may be manual or absent in security scripts.
**Acceptance Criteria:** 
- [ Secret rotation script automated and tested]
- [ Rotation doesn't break running services]
- [ Rotation schedule documented]

### Issue 121: Remove deprecated feature flags from code
**Description:** Code may have feature flags that are no longer used but still compiled in.
**Acceptance Criteria:** 
- [ Deprecated flags removed from source code]
- [ `cargo check` passes without flag errors]
- [ Feature flag gate still functional for active flags]

### Issue 122: Add security scanning gate to dep updates
**Description:** Dependency updates should be gated by security scans before merging.
**Acceptance Criteria:** 
- [ `cargo deny` or `cargo-audit` gate in PR workflow]
- [ No vulnerable deps merged without review]
- [ Gate blocks PRs with security issues]

### Issue 123: Add vulnerability disclosure process to SECURITY.md
**Description:** Security policy may lack clear vulnerability reporting process.
**Acceptance Criteria:** 
- [ Vulnerability reporting process documented]
- [ Security team contact info current]
- [ Process for triaging and tracking vulnerabilities]

### Issue 124: Add security headers to web serving manifests
**Description:** HTTP serving manifests may lack security headers (HSTS, X-Content-Type-Options, etc.).
**Acceptance Criteria:** 
- [ Security headers added to web serving configs]
- [ Headers appropriate for deployment type]
- [ No browser security issues from missing headers]

### Issue 125: Conduct full security audit of all CI workflows
**Description:** Review all 25+ CI workflows for security best practices and gaps.
**Acceptance Criteria:** 
- [ Security audit completed with findings documented]
- [ Critical security gaps remediated]
- [ Audit report with prioritized recommendations]
