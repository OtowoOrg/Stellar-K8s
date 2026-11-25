# Branch Protection

Protection for `main` is kept in the repository as a
[repository ruleset](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets)
definition, [`.github/rulesets/main.json`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/rulesets/main.json).
A pull request cannot change repository settings, so a maintainer with admin rights applies the
file with `scripts/apply-branch-protection.sh`. Changing the rules afterwards is a normal pull
request that edits the JSON, followed by another run of the script.

## What the ruleset enforces

| Requirement | How it is expressed |
|---|---|
| Changes reach `main` only through a pull request | `pull_request` rule. A direct push is rejected for everyone. |
| At least one approving review | `required_approving_review_count: 1`; new commits dismiss earlier approvals. |
| Status checks must pass | `required_status_checks`, listed below. |
| No force pushes | `non_fast_forward` rule. |
| `main` cannot be deleted | `deletion` rule. |
| Direct pushes restricted | Besides the pull request rule, the only bypass actor is the repository admin role, and only in `pull_request` mode: an admin may merge a pull request past the rules but cannot push to `main` directly. |

The ruleset does not use the separate "restrict updates" rule. It would block the merge button for
everyone who cannot bypass, which defeats the pull request flow.

### Required status checks

These are the checks [CONTRIBUTING.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md#branch-protection)
already names, as job names from the repository's own workflows. All of them are reported by
GitHub Actions (`integration_id` 15368) and none is path-filtered.

| Check | Workflow | Why |
|---|---|---|
| `Lint & Format` | `ci.yml` | Formatting, clippy, rustdoc and shell lint. |
| `Test` | `ci.yml` | The Rust test suite. Runs only after `Lint & Format` and the secret checks succeed. |
| `Security Audit` | `ci.yml` | `cargo audit`. Skipped when dependency files are unchanged. |
| `Validate Conventional Commits` | `conventional-commits.yml` | Commit messages follow Conventional Commits (commitlint). |
| `Detect Changes` | `ci.yml` | Decides which jobs run. If it fails, downstream jobs are skipped, and a skipped required check counts as passing, so it has to be required itself. |
| `Preflight (toolchain versions)` | `ci.yml` | `Lint & Format` needs it, so the same skipped-means-passing hole applies. |

`scripts/apply-branch-protection.sh` checks that every required name matches a job `name:` in
`.github/workflows/`, and `--apply` refuses to continue otherwise, because a required check that
no workflow reports would block every merge.

`main` is currently red: `Lint & Format` fails while the Rust dependencies do not build together.
Apply the ruleset after the build is fixed, or expect that fix to be merged with an admin bypass
(the admin role may merge a pull request past the rules, see above).

Other choices that can be tightened later: `require_code_owner_review` is `false` until the teams in
`.github/CODEOWNERS` are confirmed to be real teams with write access (a required review from an
owner that cannot review cannot be satisfied), and `strict_required_status_checks_policy` is
`false` so a busy `main` does not force a rebase before every merge.

## Applying it

You need `gh` and `jq`, and admin rights on the repository.

```bash
gh auth login

# 1. Dry run (the default): prints the ruleset, the required checks, and whether it would
#    create a new ruleset or update the existing one. Nothing is sent.
scripts/apply-branch-protection.sh

# 2. Create the ruleset, or update it in place if one named "main-protection" exists
scripts/apply-branch-protection.sh --apply

# 3. Read-only: confirm the live ruleset is present and matches the file
scripts/apply-branch-protection.sh --check
```

`--repo OWNER/REPO` targets another repository (for example a fork) and `--ruleset FILE` another
definition. `--check` exits 1 when the ruleset is missing or differs, so it can also run on a
schedule to detect drift.

To apply it from the web UI instead, open **Settings, Rules, Rulesets, New ruleset, Import a
ruleset** and upload `.github/rulesets/main.json`.

The suite `scripts/tests/apply-branch-protection.bats` asserts the properties above on the
committed file and exercises the script against a `gh` stand-in (`make test-bats`).
