# CRD Drift Detection (Helm Templates vs. Rendered Manifests)

> **TL;DR** — `crd-drift-check` compares the CustomResourceDefinitions
> rendered by `helm template charts/stellar-operator` against the canonical
> manifests in `config/crd/`. Today's state is the accepted baseline
> (`.crd-drift-baseline.toml`); CI fails when the current state diverges from
> it. Update the baseline deliberately after a reviewed schema change.

---

## Why this exists

Stellar-K8s's CustomResourceDefinitions live in two places that have to be
kept in sync by hand:

- `config/crd/*.yaml` — the canonical, full CRD manifests (also the source
  used by `schema-validate`, see [schema-validation.md](schema-validation.md)).
- `charts/stellar-operator/templates/*.yaml` — a Helm-templated subset of the
  same CRDs, rendered via `helm template`.

Nothing previously checked that these two sources agreed. In practice they
already diverge in known ways (the Helm chart only templates 3 of the 13
CRDs defined in `config/crd/`, and its copies aren't byte-identical to
`config/crd/`'s). `crd-drift-check` doesn't require them to be identical —
instead it snapshots today's state once as an accepted **baseline** and
fails only when something *changes* relative to that baseline without a
human explicitly re-accepting it. This is the same pattern `doc-check` (see
[stale-docs-detector.md](stale-docs-detector.md)) already uses for
source-vs-doc drift.

### What it caught on first run

Building this tool immediately surfaced a real bug: `config/crd/stellarnode-crd.yaml`
listed `maxUnavailable`, `minAvailable`, and `topologySpreadConstraints` as
`required`, even though the Rust source (`src/crd/stellar_node.rs`) defines
all three as `Option<...>` with a `None` default — the checked-in schema had
drifted from the CRD's actual source of truth. That mismatch alone caused
246 false-looking violations across 44 example manifests in `schema-validate`.
It's fixed in this change; see the commit history for the corrected
`required:` list.

## Running it

```bash
# Check current state against the accepted baseline
make crd-drift-check
cargo run --bin crd-drift-check

# Show every known CRD's presence/hash on both sides, no pass/fail
cargo run --bin crd-drift-check -- status

# Accept the current state as the new baseline (after a reviewed change)
cargo run --bin crd-drift-check -- update-baseline
```

Requires the `helm` binary on `PATH` (already a prerequisite for this repo's
`helm-lint` CI job and local dev setup).

It runs as a step in the CI `helm-lint` job (gated on the same
`charts/ | config/crd/ | config/samples/ | examples/` path filter as the
rest of that job) and as a local pre-commit hook in warn-only mode.

## Updating the baseline

Any time you intentionally change a CRD's schema in `config/crd/` or in the
Helm chart's templates, run `cargo run --bin crd-drift-check -- update-baseline`
and commit the resulting `.crd-drift-baseline.toml` diff alongside your
change. Reviewers can see exactly what shifted by diffing that file.

## Tests

Unit tests in `src/bin/crd_drift_check.rs` (`#[cfg(test)] mod tests`) cover
hash stability/order-independence, CRD document parsing, and every drift
category (new, removed, presence changed, schema changed). Run with:

```bash
cargo test --bin crd-drift-check
```
