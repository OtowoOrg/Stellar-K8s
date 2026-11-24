# Static Analysis Gates

Three repository-wide gates run in CI and are runnable locally. Each one
replaces or extends a check that was previously partial, advisory, or
structurally unable to fail.

| Gate | Command | Issue | Replaces |
|---|---|---|---|
| Shell safety | `make shell-safety` | #1049 | Extends `shellcheck -S error` |
| YAML manifest validation | `make validate-yaml` | #1044 | Extends `validate-config-samples.sh` |
| YAML lint + CRD JSON schemas + Helm kubeconform | `make yaml-schema-validate` | #1291 | Adds yamllint, `schemas/crd/`, rendered-chart kubeconform |
| Helm template drift | `make helm-drift` | #1045 | Replaces `check-chart-diff.sh` |
| Helm edge-case / upgrade unittest | `make helm-unittest` / `make helm-upgrade-test` | #1289 | Extends helm-unittest |
| Database migration harness | `make test-db-migrations` | #1317 | sqlx + Postgres |
| License header enforcement | `make license-headers` | #1286 | New gate |

All three need only Python 3 with `pyyaml` and `jsonschema`:

```bash
pip install pyyaml jsonschema
```

---

## Shell safety gate (#1049)

`scripts/check-shell-safety.py` scans shell scripts for patterns that cause
silent data loss, command injection, or non-deterministic CI behaviour.

It exists alongside `shellcheck`, not instead of it. The repository runs
`shellcheck -S error`, which by design reports only what a linter considers
fatal — it stays quiet about an unquoted `rm -rf $DIR`, a `curl | bash`, or a
script that never enabled `set -euo pipefail`. Those are stylistic to a
linter and operational to an operator repository.

### Running it

```bash
make shell-safety                              # gate the repo (scripts/)
python3 scripts/check-shell-safety.py --strict # warnings fail too
python3 scripts/check-shell-safety.py --list-rules
python3 scripts/check-shell-safety.py --format json scripts/preflight.sh
make test-shell-safety                         # the gate's own unit tests
```

Exit codes: `0` pass, `1` findings at `error` severity, `2` bad invocation.

### Rules

| ID | Severity | Detects |
|---|---|---|
| `SH000` | error | A suppression pragma with no `--` reason |
| `SH001` | error | Executable script without `set -euo pipefail` |
| `SH002` | error | Unquoted expansion passed to `rm`/`mv`/`chmod`/`dd`/… |
| `SH003` | error | `rm -rf "$dir"/sub` with no empty-value guard |
| `SH004` | error | `eval` on interpolated data |
| `SH005` | error | `curl … \| bash` |
| `SH006` | error | `-k` / `--insecure` / `--no-check-certificate` |
| `SH007` | error | `chmod 777`, `a+rwx`, `o+w` |
| `SH008` | warning | Predictable `/tmp/name` instead of `mktemp` |
| `SH009` | warning | `mktemp` result with no cleanup |
| `SH010` | error | `cd` whose failure is unhandled (non-strict scripts only) |
| `SH011` | warning | Backtick command substitution |
| `SH012` | error | Unquoted `$@` / `$*` argument forwarding |
| `SH013` | warning | Iterating over `ls`/`find` output |
| `SH014` | error | Unquoted expansion inside `[ … ]` |
| `SH015` | error | `curl` without `-f`/`--fail` |

The checker understands shell context: comments, heredoc bodies, and
single-quoted spans are inert; `#` inside a string does not start a comment;
`"$*"` inside a quoted message is not flagged while a bare `$@` is.

Some rules are deliberately narrower than they first appear, because a rule
that fires on safe code is a rule people learn to ignore:

- **`SH003`** flags `rm -rf "$dir"/build` (an empty `$dir` deletes `/build`)
  but not a bare `rm -rf "$dir"` (`rm -rf ""` is a harmless no-op).
- **`SH010`** is silent under `set -e`, which already aborts on a failed `cd`.
- **`SH001`** exempts sourced libraries (`scripts/lib/*`, files with no
  shebang) and `.bats` files, which must not impose `set -e` on their caller.

### Waiving a finding

Every waiver needs a `--` reason; a bare `allow` is itself an error, so
waivers cannot be added silently.

```bash
rm -rf "$BUILD_DIR"/artifacts  # shell-safety: allow SH003 -- path validated above

# shell-safety: allow SH002 -- fixture path is a literal
rm -rf $FIXTURE

# In the file header, for a file-wide rule:
# shell-safety: disable-file SH001 -- report must exit 0 even when cargo fails
```

Repository-wide exclusions and severity overrides live in
`config/shell-safety.yaml`. Prefer an inline pragma: it keeps the
justification next to the line that needs it.

### Current state

The repository is clean at `error` severity. Four `SH008` warnings remain
(predictable `/tmp` paths in CI helper scripts) and are visible in every run.

---

## YAML manifest validation (#1044)

`scripts/validate-yaml-manifests.py` validates **every** YAML file in the
repository in four layers.

The previous check (`scripts/ci/validate-config-samples.sh`, still run) looked
at `examples/` and `config/samples/` only, delegated to `kubeconform` with
`-ignore-missing-schemas` — so every `stellar.org` custom resource was skipped
outright — and downgraded all findings to warnings.

| Layer | What it checks |
|---|---|
| `L1-syntax` | Every document parses; duplicate mapping keys and literal tabs are errors |
| `L2-structure` | Kubernetes docs have a well-formed `apiVersion`/`kind`, a DNS-1123 `metadata.name`, and valid label/annotation keys and values |
| `L3-schema` | Custom resources validate against this repo's own CRDs in `config/crd/`; any path can be bound to a JSON Schema |
| `L4-fixture` | Manifests declared as negative fixtures **must** fail — a schema that silently starts accepting bad input is caught |

Duplicate keys matter because PyYAML (and most YAML loaders) silently keep the
last value: that is how a manifest ends up with two `image:` keys and quietly
deploys the wrong one.

### Running it

```bash
make validate-yaml
python3 scripts/validate-yaml-manifests.py --summary
python3 scripts/validate-yaml-manifests.py --format json
python3 scripts/validate-yaml-manifests.py examples/          # a subtree
make test-yaml-validation                                     # unit tests
```

### Configuration

`config/yaml-validation.yaml` holds four lists:

- **`exclude`** — files not read at all (Helm templates are Go templates, not
  YAML; their rendered output is covered by the drift gate below).
- **`schemas`** — bind a path glob to a JSON Schema, e.g. the chart's
  `values.yaml` to `values.schema.json`.
- **`expect_invalid`** — manifests that must fail, such as
  `config/samples/invalid-*.yaml`.
- **`known_deviations`** — pre-existing failures reported as warnings instead
  of errors. Every entry needs a reason, **and a waiver that stops matching
  anything is reported as an error**, so the list cannot silently rot.

### Current state

Zero errors. 233 warnings, all recorded in `known_deviations`:

- **212** come from one root cause. `src/crd/schema_utils.rs` supplies
  hand-written schemas via `#[schemars(schema_with = ...)]`. schemars applies
  that override *before* it unwraps `Option<T>`, so `minAvailable`,
  `maxUnavailable`, and `topologySpreadConstraints` — all `Option<...>` in
  `src/crd/stellar_node.rs` — land in the generated CRD's `required` list.
  The operator treats them as optional; only the generated CRD disagrees.
  Fixing it means changing CRD generation and regenerating `config/crd/`.
- The rest are illustrative examples that intentionally omit unrelated
  required fields.

---

## Helm template drift detection (#1045, enhanced #1395)

`scripts/check-helm-drift.sh` renders the chart across five value profiles,
normalises each render through `scripts/sort-manifests.py`, and diffs the
result against golden files committed under
`charts/stellar-operator/rendered/`.

The predecessor, `scripts/check-chart-diff.sh`, compared against a baseline in
gitignored `.cache/`. In CI that directory is always empty, so the baseline was
recreated on every run and the comparison never happened — drift detection
that structurally could not fail. (That script was also committed twice into
the same file.) It has been removed.

Storing goldens in git means a template change that alters rendered output
shows up as a concrete manifest diff in the pull request, reviewable like any
other file.

### Enhanced features (#1395)

- **YAML-aware diffing**: When [dyff](https://github.com/homeport/dyff) is installed, the script uses `dyff between` for diffs that handle key reordering and formatting noise gracefully. Falls back to `diff -u` if dyff is not available.
- **High-risk field detection**: The `--check-high-risk` flag detects changes to critical fields (image tags, replicas, resources, RBAC, secrets) and emits elevated alerts.
- **$GITHUB_STEP_SUMMARY**: When running in GitHub Actions, a summary is written to the job summary for better visibility.

### Profiles

| Profile | Values |
|---|---|
| `default` | `values.yaml` |
| `ha` | `values-ha.yaml` |
| `production` | `examples/values-production.yaml` |
| `development` | `examples/values-development.yaml` |
| `dr-cross-region` | DR + cross-region bridge enabled with one peer cluster |

### Running it

```bash
make helm-drift                                  # verify
make helm-drift-update                           # regenerate goldens
scripts/check-helm-drift.sh --profile production # one profile
scripts/check-helm-drift.sh --check-high-risk    # detect high-risk field changes
scripts/check-helm-drift.sh --list
make test-helm-drift                             # bats tests
```

`make helm-lint` runs the drift check too.

### Intentional template changes

```bash
make helm-drift-update
git diff charts/stellar-operator/rendered   # review the rendered impact
git add charts/stellar-operator/rendered
```

Reviewing that diff is the point: it shows exactly which manifests a template
edit changes.

### Unintentional drift (issue #1365)

If CI fails here on a PR that didn't knowingly touch chart templates, the
change usually came from somewhere less obvious than
`charts/stellar-operator/templates/`:

1. **A values default changed.** `values.yaml`, `examples/values-*.yaml`, or
   a Helm library dependency version bump can shift rendered output without
   a single line of template code changing. `git diff charts/stellar-operator/rendered`
   after `make helm-drift-update` (run it in a scratch branch, don't commit
   yet) shows exactly which fields moved and from what.
2. **It's flagged high-risk.** `--check-high-risk` specifically watches RBAC
   rules, `securityContext`, and resource `limits`/`requests` — fields where
   an unreviewed change has real blast radius (privilege escalation, pods
   evicted under memory pressure, etc.). Treat a high-risk hit as needing a
   second reviewer's sign-off before regenerating goldens, not a rubber-stamp
   `make helm-drift-update`.
3. **The PR didn't mean to change the chart at all.** If `git diff` on the
   rendered goldens shows nothing you can attribute to your own change, check
   whether `Chart.yaml`'s dependency versions or the pinned Helm version in
   this workflow (`azure/setup-helm@v4`, currently v3.14.0) moved underneath
   you — either can shift template function output (e.g. `include`
   ordering, `lookup` behavior) with no diff in this repo's own files.

Once the cause is understood and the change is confirmed intentional,
regenerate goldens as in the section above; if it isn't, fix the template
or values regression instead of updating goldens to match it.

### What it caught

`charts/stellar-operator/examples/values-production.yaml` did not render at
all. `templates/cross-region-bridge.yaml` dereferenced `.Values.crossRegion`,
a key absent from every values file, so `featureFlags.enableDr: true` alone
aborted rendering with a nil-pointer error. The template is now nil-safe and
`values.yaml` documents a `crossRegion` block. `scripts/tests/helm-drift.bats`
carries regression tests for it.

---

## License header enforcement (#1286)

`scripts/check-license-headers.py` scans every Rust, Shell, and YAML file for
the repository's canonical Apache-2.0 header and fails CI when one is
missing or malformed. It runs as a pre-commit hook (`license-headers`, wired
to `*.rs`/`*.sh`/`*.ya?ml`) and in CI (`wave-security-compliance.yml`).

### Header format

The canonical header (identical in spirit for all three languages, only the
comment marker changes — `//` for Rust, `#` for Shell/YAML):

```rust
// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
```

It must appear within the first 25 lines of the file. A short-form SPDX
identifier is also accepted as an alternative to the full block, on any
line within that same window:

```rust
// SPDX-License-Identifier: Apache-2.0
```

### Exceptions

Two categories of file are skipped entirely — `should_exclude()` in
`scripts/check-license-headers.py` is the source of truth; this list exists
so a reviewer doesn't have to read the script to know why a given file has
no header:

**By path** — generated, vendored, or non-source content: `target/`,
`bundle/`, `vendor/`, `.github/`, `docs/`, `config/crd/` (generated by
`crdgen`), `config/samples/`, `examples/`, `schemas/`, `.kiro/` (AI-generated
specs), Helm chart `templates/` and `tests/` (Go template syntax, not valid
Rust/Shell/YAML), `charts/stellar-operator/rendered/` and
`benchmarks/baselines/` (generated output), `benchmarks/k6/` (JavaScript,
not one of the three checked languages), security-tool configs
(`.gitleaks.toml`, `.cargo/audit.toml`, `deny.toml`), and `build.rs` (code
generator, not itself generated — excluded because it predates the gate and
regenerating its header isn't worth the diff).

**By filename** — repo-root docs and tool configs where a license header
would be meaningless or actively wrong (e.g. `LICENSE` itself):
`README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, and similar
top-level `*.md` files, plus tool configs like `.editorconfig`,
`.gitignore`, `.yamllint.yml`, `mkdocs.yml`, `Tiltfile`, and `PROJECT`.

Adding a new exception means adding it to `EXCLUDED_PATHS` or
`EXCLUDED_FILENAMES` in the script, not special-casing it in CI — the
pre-commit hook and CI both call the same script, so there is exactly one
place exceptions are declared.

### Fixing a violation

```bash
python3 scripts/check-license-headers.py            # check (matches CI)
python3 scripts/check-license-headers.py --fix      # insert missing headers
python3 scripts/check-license-headers.py --report   # report-only, exit 0
make license-headers                                # alias for the check
```

`--fix` inserts the canonical header for the file's language at the top of
the file (after a shebang line, if present, for Shell scripts); it does not
attempt to reformat or replace a malformed header, so a header that's
present but wrong (wrong year, wrong wording) still needs a manual edit.

---

## Verification

```bash
make shell-safety && make test-shell-safety
make validate-yaml && make test-yaml-validation
make helm-drift && make test-helm-drift
make license-headers
```

All gates should report zero errors on a clean checkout.
