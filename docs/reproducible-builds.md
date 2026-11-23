# Reproducible Builds and Verification

Epic: #1997 · Library: `src/reproducible_build.rs` · Build: `scripts/reproducible-build.sh` · Verifier: `verify-reproducible-build` · CI: `.github/workflows/reproducible-build.yml`

A release is reproducible when rebuilding it from source gives the same bits.
This is checked on every release, and a mismatch is traced to the build step
that caused it.

## Deterministic build flags

`scripts/reproducible-build.sh` (and the `builder` stage of the `Dockerfile`)
pin everything that otherwise makes two builds of the same source differ:

| Source of variation | Pinned by |
|---------------------|-----------|
| Timestamps | `SOURCE_DATE_EPOCH` = commit time of the revision, `TZ=UTC` |
| Build paths | `--remap-path-prefix` for the checkout (`/build`), `CARGO_HOME` (`/cargo`) and `RUSTUP_HOME` (`/rustup`); `BUILD_PATH_PREFIX=/build` |
| Locale | `LC_ALL=C`, `LANG=C` |
| Linker build id | `-C link-arg=-Wl,--build-id=none` |
| Cargo | `--locked`, `CARGO_INCREMENTAL=0`, a fresh target directory |

In the Dockerfile pass the commit time so embedded timestamps match:

```bash
docker build --build-arg SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" .
```

The script builds `stellar-operator`, `kubectl-stellar` and `stellar-sidecar`
(override with `--bins`), generates the CRD bundle, and writes
`build-manifest.json` with each artifact's SHA-256, the pinned environment, and
the pipeline and revision that produced it.

```bash
make reproducible-build        # builds into build-out/
scripts/reproducible-build.sh --output-dir out --pipeline release-pipeline
```

## Verifying a release

The workflow builds the release twice in separate jobs: `release-pipeline` and
`independent-rebuild`, with no shared cache, both from the same revision.
`verify-reproducible-build` then:

1. refuses to compare unless the rebuild really is independent: a different
   pipeline, flagged as a rebuild, of the **same** revision;
2. checks every file against the digest in its manifest, so the bytes and the
   manifest must describe the same build;
3. compares each artifact bit for bit (SHA-256 plus the byte offset of the first
   difference), not by version or timestamp;
4. localizes each mismatch to the step that produced the artifact and lists the
   non-determinism sources found in the differing bytes (timestamps, path
   prefixes, locale, VCS metadata, build id, archive metadata, line endings,
   random seeds), and which of them that step does not pin.

```text
reproducibility: non_reproducible (1/2 artifacts, 50%)
  MISMATCH `stellar-operator` differs at byte 4180; sources [timestamps]; step `cargo-build` does not pin [timestamps]
```

| Verdict | Meaning | Exit code |
|---------|---------|-----------|
| `reproducible` | every artifact identical | 0 |
| `mostly_reproducible` | some differ, but at least 95% match | 0 (1 with `--strict`) |
| `non_reproducible` | below 95% | 1 |
| could not run | wrong revision, same pipeline, digest mismatch, unreadable files | 2 |

## Release audit

Every verification, passing or failing, appends one JSON line to
`release-audit/reproducibility.jsonl`:

```json
{"recorded_at":"2026-11-19T10:20:00Z","release":"v2.6.0","revision":"6921706…",
 "verdict":"reproducible","artifacts_checked":4,"reproducible":4,"reproducible_rate":1.0,
 "implicated_steps":[],"release_pipeline":"release-pipeline",
 "rebuild_pipeline":"independent-rebuild","report_sha256":"ab12…"}
```

`report_sha256` ties each line to the full report
(`reproducibility-report.json` / `.md`), so the detailed evidence behind an audit
entry can be checked. On a published release the workflow uploads the report and
the audit log as release assets; the `reproducibility-audit` workflow artifact
holds the same files for pull requests and manual runs.

## Scope

- The check covers the binaries and the CRD bundle. The container image is built
  with the same pinned inputs (see the Dockerfile), but its layers are not
  compared yet: that needs `buildx` timestamp rewriting and an OCI-level digest
  comparison.
- The independent rebuild runs on the same runner image as the release build.
  That proves the build does not depend on job-local state, not that it is
  identical across toolchain or OS versions.
- Helm chart and operator-bundle packaging are not yet part of the manifest.
