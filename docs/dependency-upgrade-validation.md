# Dependency Upgrade Validation

Epic: #1999 · Library: `src/dependency_contract.rs`, `src/dependency_scan.rs` · CLI: `dependency-upgrade-gate` · CI: `.github/workflows/dependency-contract-gate.yml`

A dependency upgrade that breaks an in-repo consumer is blocked before merge,
and the failure names the consumer and the `file:line` call sites responsible.

## How it works

1. **Find the changed dependencies.** `scripts/dependency-upgrades.py <base-ref>`
   compares `Cargo.lock` against the base branch and prints `name from to` for
   each *direct* dependency whose locked version changed. Transitive-only
   changes are not gated, because nothing in this repository calls them.
2. **Find the call sites.** `dependency_scan::scan_tree` reads every `.rs` file
   (skipping `target/`, `.git/`, `node_modules/`, `vendor/`) and records each line
   that reaches into the dependency by path: `use kube::runtime::Controller;`,
   `use kube::{Api, Client, runtime::{watcher, Controller}};` (also across lines)
   and `kube::Client::try_default()`. Comments are ignored. Each site carries the
   consumer, the `file:line` and the full API paths it reaches.
3. **Generate contract tests.** `ContractTestGenerator` turns every call site into
   a contract test, so the suite is derived from the code that really calls the
   dependency and cannot drift from it. There is no second suite to maintain.
4. **Check against known breaking changes.** The tests are grouped into a
   consumer x dependency compatibility matrix and checked against
   `config/dependency-breaking-changes.json`.
5. **Block or approve.** Any incompatible consumer blocks the upgrade. A major
   version bump with no call sites is also blocked, since nothing exercises the
   new surface. An approved upgrade carries a SHA-256-signed validation artifact.

## Consumer attribution

The consumer comes from where the call site lives:

| Path | Consumer |
|------|----------|
| `src/main.rs` | `stellar-operator` |
| `src/bin/<name>.rs` | `<name>` |
| `src/<module>/…` or `src/<module>.rs` | `stellar-operator::<module>` |
| `tests/…`, `benches/…` | `tests::<file>`, `benches::<file>` |

A blocked upgrade reads, for example:

```text
kube 0.94.0 -> 1.0.0: 3 call site(s) scanned, verdict blocked
upgrade of kube blocked: 1 incompatible consumer(s)
  - stellar-operator::controller blocked: removed API kube::runtime::Controller (call sites: src/controller/reconcile.rs:41)
```

## Recording breaking changes

`config/dependency-breaking-changes.json` is keyed by crate name. Each entry is
`{ "api", "kind", "detail" }` where `kind` is one of `removed_api`,
`changed_signature`, `changed_return_type`, `renamed_config_key` or
`behavior_change`. `api` is matched as a path prefix, so
`kube::runtime::Controller` also covers `kube::runtime::Controller::new`. A
`behavior_change` makes the consumer *unverified* (needs human sign-off) rather
than blocking it.

Add an entry when an upgrade PR or a changelog shows a break. A crate with no
entry has no known breaks, which is why the gate still blocks a major bump that
nothing in the repository exercises.

## Running it

In CI the gate runs on every pull request that touches `Cargo.toml` or
`Cargo.lock`, once per upgraded dependency. It writes a job summary and a PR
comment, uploads the reports and the generated contract tests as the
`dependency-contract-gate` artifact, and fails the job when an upgrade is
blocked. Make **Validate dependency upgrades** a required status check on `main`
so a blocked upgrade cannot merge.

Locally:

```bash
make dependency-gate DEP=kube FROM=0.94.0 TO=0.95.0

# or directly
cargo run --locked --bin dependency-upgrade-gate -- \
  --dependency kube --from 0.94.0 --to 0.95.0 --pr 123 \
  --breaking-changes config/dependency-breaking-changes.json \
  --json-out report.json --markdown-out report.md --tests-out contract_tests.rs
```

Exit codes: `0` approved, `1` blocked, `2` the gate could not run (bad version,
unreadable file). CI treats `2` as a failure, so the gate never passes by
accident.

## Limits

- Call sites are found by path. A later use of an imported name
  (`Api::namespaced(..)`) is covered by the import that brought it in, not listed
  separately.
- Only Rust sources are scanned. Base-image and sibling-operator upgrades use the
  library API directly (`DependencyKind::BaseImage`, `DependencyKind::Operator`).
- The breaking-change list is curated. The gate proves "no known break reaches a
  consumer", not that the new version is correct.
