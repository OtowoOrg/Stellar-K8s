# Dependency Security Audit & Cleanup Report

## Executive Summary

This audit addresses security vulnerabilities and dependency management issues in the Stellar Kubernetes Operator. The project has comprehensive security monitoring in place via `cargo-deny` and `cargo-audit`.

> **Note on advisory counts (issue #936):** this document previously claimed a
> single figure of "23 known advisories". There is no single authoritative list —
> four independent ignore lists exist and **none of them contains 23 entries**:

> | Location | Entries |
> |---|---|
> | `.cargo/audit.toml` | 26 |
> | `deny.toml` | 20 |
> | `.github/workflows/ci.yml` | 18 |
> | `.github/workflows/dependency-review.yml` | 15 |
>
> `deny.toml` states it is "in sync with `.cargo/audit.toml`" and with the
> workflow `cargo-audit --ignore` lists, but it is missing 6 entries present in
> `.cargo/audit.toml` (`RUSTSEC-2026-0049`, `-0149`, `-0182`, `-0185`, `-0188`,
> `-0192`). `ci.yml` and `dependency-review.yml` each ignore 5 advisories that
> appear in neither `deny.toml` nor `.cargo/audit.toml` (`RUSTSEC-2020-0071`,
> `RUSTSEC-2026-0085`, `-0087`, `-0091`, `-0204`). Reconciling these into one
> generated source of truth is outstanding work — see "Open Gaps" below.

## Current Security Posture

### ✅ Strengths
- Comprehensive security monitoring via `cargo-deny` and `cargo-audit` 
- Well-documented advisory exceptions with justifications
- License compliance enforcement
- Explicit crate banning (openssl blocked in favor of rustls)
- Version pinning for critical security fixes (anyhow 1.0.104, bytes 1.11.1)

### ⚠️  Areas for Improvement
- 20 advisories ignored by `deny.toml`; 26 by `.cargo/audit.toml` — see the count note above
- Some transitive dependencies cannot be upgraded due to ecosystem constraints
- Major version upgrades needed for wasmtime (24.x → 36.x) 
- Several unmaintained dependencies in the dependency tree

## Priority Security Issues

### HIGH PRIORITY - Action Required

1. **wasmtime 24.x → 36.x Upgrade** 
   - **Impact**: 6 critical vulnerabilities affecting Winch backend (unused by us, but still present)
   - **Current**: wasmtime 24.0.11, wasmtime-wasi 24.0.11  
   - **Target**: wasmtime ≥36.x
   - **Blockers**: Breaking API changes require code updates
   - **Risk**: LOW (vulnerabilities are in unused Winch backend)

2. **rustls-webpki Multiple Versions**
   - **Impact**: TLS certificate parsing vulnerabilities
   - **Current**: 0.101.7 (via kube-client) + 0.102.8 (via reqwest) + 0.103.13
   - **Target**: ≥0.103.12 — the 0.101.7 and 0.102.8 copies still lag; 0.103.13 already satisfies the target
   - **Blockers**: Requires upstream kube-rs and reqwest updates
   - **Risk**: LOW (we don't process untrusted certificates)

3. **Unmaintained Dependencies**
   - `backoff 0.4.0` (via kube-runtime)
   - `derivative 2.2.0` (via kube-runtime) 
   - `instant 0.1.13` (via backoff)
   - `fxhash 0.2.1` (via wasmtime)
   - `paste 1.0.15` (via wasmtime)
   - `rustls-pemfile 2.2.0` (via kube-client/axum-server)
   - `ttf-parser 0.19.2` (via printpdf)

### MEDIUM PRIORITY

4. **Version Pinning Updates**
   - Review pinned versions for newer patches
   - `anyhow` — was pinned to the non-existent `1.0.108`, which broke
     resolution. Now `1.0.104` (the latest published patch). ✅ resolved
   - `bytes = "1.11"` (locked 1.11.1) — matches the lock; monitor for patches

5. **Dependency Deduplication**
   - Multiple syn versions (1.x vs 2.x ecosystem split)
   - Multiple tokio-util versions
   - Review with `cargo tree --duplicates`

### LOW PRIORITY

6. **Transitive-Only Issues**
   - `rsa 0.9.10` Marvin Attack (sqlx-mysql only, we use postgres)
   - `rand 0.9.4` unsound behavior (RUSTSEC-2026-0097; no patched 0.9.x exists)
   - Various wasmtime Winch backend issues (unused backend)

## Hardening Recommendations

### 1. Automated Security Monitoring
```bash
# Add to CI pipeline
cargo deny check
cargo audit --deny warnings
```

### 2. Dependency Review Process
- Require security review for new dependencies
- Monthly audit of ignored advisories
- Quarterly major version upgrade assessment

### 3. Build Hardening
```toml
# Add to Cargo.toml profiles
[profile.release]
strip = true          # Remove debug symbols
panic = "abort"       # Don't unwind on panic  
codegen-units = 1     # Better optimization
lto = true            # Link-time optimization
```

### 4. Supply Chain Security
- Pin exact versions in Cargo.lock
- Use `cargo-vet` for dependency auditing
- Implement SBOM generation

## Implementation Plan

### Phase 1: Immediate Actions (Week 1)
- [ ] Update dependency scanning in CI
- [ ] Review and update pinned security patches  
- [ ] Document security review process
- [ ] Add automated security scanning to pre-commit hooks

### Phase 2: Ecosystem Dependencies (Weeks 2-4)
- [ ] Evaluate wasmtime 36.x upgrade path
- [ ] Create tracking issues for upstream dependency updates
- [ ] Implement workarounds for unmaintained dependencies where possible

### Phase 3: Long-term Hardening (Month 2)
- [ ] Implement SBOM generation
- [ ] Set up automated dependency update PRs
- [ ] Create security baseline documentation
- [ ] Establish quarterly security review process

## Testing & Verification

### Security Test Suite
```bash
# Current commands
cargo deny check
cargo audit 

# Proposed additions  
cargo outdated --root-deps-only
cargo tree --duplicates
cargo vet
```

### Pipeline Integration
- Block PRs with new security advisories
- Require security team approval for ignored advisories
- Automated SBOM generation and publishing

## Compliance & Documentation

### License Compliance
- ✅ Comprehensive allowlist in `deny.toml`
- ✅ Unicode-DFS-2016 exception documented
- ⚠️ "Explicit handling of copyleft licenses" — no dedicated copyleft entry
  exists. Copyleft options (`ittapi`/`ittapi-sys` are `BSD-3-Clause OR GPL-2.0`,
  `r-efi` is `Apache-2.0 OR LGPL-2.1-or-later OR MIT`) are permitted only
  because cargo-deny accepts an `OR` expression when any branch is allowlisted.
  The permissive branch is what is relied upon, which is correct but implicit.

## Open Gaps (issue #936)

### 1. Undisclosed dependencies behind optional features
`scripts/generate-third-party-licenses.sh` pins the feature set to
`--features "rest-api,metrics,admission-webhook,k8s-v1-30"`, but CI and
pre-commit build the workspace with **`--all-features`**
(`.pre-commit-config.yaml` runs `cargo clippy`/`cargo test --workspace --all-features`).
The following compiled dependencies are therefore **absent from
`THIRD_PARTY_LICENSES.md`**:

| Crate | Declared at | Feature | Why it matters |
|---|---|---|---|
| `rdkafka` | `Cargo.toml:100` | `kafka` | librdkafka bindings (MIT) |
| `sasl2-sys` | `Cargo.toml:107` | `kafka` | vendors Cyrus SASL + zlib |
| `async-nats` | `Cargo.toml:105` | `nats` | NATS client (Apache-2.0) |

Their transitive closure (`cmake`, `libz`/`libz-sys`, `ar_archive_writer`, …) is
missing as well. **Fix:** switch the generator to `--all-features` so the
inventory matches what is actually built, then run `make third-party-licenses`.

### 2. `THIRD_PARTY_LICENSES.md` includes the workspace's own crate
Line 477 lists `| stellar-k8s | 0.1.0 | Apache-2.0 |` — the project itself is
not a third party. The generator does not filter the root package.

### 3. One dependency has no machine-readable license
`jsonpath-rust 0.5.1` resolves to `(see crate)` (the generator's fallback for an
empty license field), so no SPDX attribution can be asserted for it.

### 4. The license file and `cargo deny` evaluate different graphs
`deny.toml` restricts `[graph] targets` to the two `*-linux-gnu` triples, while
the license file is generated from a Linux-host feature set. Platform-specific
crates that appear in the file (e.g. `winapi-util`, `windows-*`) are outside the
graph `cargo deny` ever validated, so the inventory over-reports relative to
what is actually enforced.

### 5. Non-Rust third parties are not tracked at all
`THIRD_PARTY_LICENSES.md` is 100% Rust crates. Also in use but undisclosed:
Python packages in `requirements.txt` (plus `PyYAML`, imported by
`scripts/crd_migration_lint.py` but missing from `requirements.txt`); the
Grafana **k6** load-test runner (AGPL-3.0) used by `benchmarks/k6/*.js`; the
second, non-workspace crate at `tools/manifest-validator/`; and third-party
container base images (`lukemathwalker/cargo-chef`, `rust:1.95-bookworm`,
`rancher/k3s`, `debian:bookworm-slim`). `.github/dependabot.yml` has no `npm`
or `pip` ecosystem entries.

### 6. Dead advisory exceptions for a non-existent dependency
`deny.toml:111-113` and `.cargo/audit.toml` carry `RUSTSEC-2024-0380` /
`RUSTSEC-2024-0381` ignores justified as an "experimental pqcrypto KMS path,
behind a feature flag". **No `pqcrypto` package exists in `Cargo.toml` or
`Cargo.lock`.** The ignores are inert, but the justification describes a
component that is not present and should be corrected or removed.

### 7. `THIRD_PARTY_LICENSES.md` has no generation provenance
The file records no timestamp, generator version, or `Cargo.lock` hash, so
staleness can only be detected by running the check.

### Security Documentation
- Update README with security contact
- Document security advisory triage process
- Create SECURITY.md with vulnerability reporting

## Risk Assessment

| Issue Category | Risk Level | Justification |
|----------------|------------|---------------|
| Wasmtime vulnerabilities | LOW | Unused Winch backend only |
| Unmaintained deps | MEDIUM | Transitive only, no direct usage |
| TLS cert parsing | LOW | No untrusted cert processing |
| Supply chain | LOW | Comprehensive scanning in place |
| License compliance | LOW | Well-controlled allowlist |

**Overall Risk Level: LOW to MEDIUM**

The project demonstrates strong security awareness with comprehensive monitoring. Most high-severity advisories are appropriately justified as not applicable to production usage patterns.