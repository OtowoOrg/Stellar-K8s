# Issue label review

Review issue labels when changing an issue template and during routine maintainer
triage. This procedure is read-only; label creation, renaming, and removal should
be agreed by maintainers before changing the repository's label catalog.

## Compare templates with the live catalog

From an authenticated GitHub CLI session, list every upstream label:

```bash
gh api --paginate repos/OtowoOrg/Stellar-K8s/labels --jq '.[] | [.name, .description] | @tsv'
```

Compare the exact names with the top-level `labels` fields in
[the issue templates](https://github.com/OtowoOrg/Stellar-K8s/tree/main/.github/ISSUE_TEMPLATE):

| Template | Requested labels |
| --- | --- |
| Bug report | `bug`, `needs-triage` |
| Feature request | `enhancement` |
| Maintenance / Chore | `maintenance` |
| 200-point Epic | `epic`, `200-points`, `needs-triage` |
| Stellar Wave Issue | `stellar-wave` |

The support form has no preset labels. Confirm every requested label exists;
GitHub does not automatically create missing labels for issue forms. Record
missing labels and unclear or overlapping descriptions for maintainer review.
In particular, check `stellar-wave` versus `Stellar Wave` before proposing
consolidation. Keep program and points labels
until their automation consumers have been reviewed.

The catalog review on **2026-10-16** found that `triage` and `chore` did not
exist upstream. The bug form now uses the existing `needs-triage` label and the
maintenance form requests only `maintenance`. All labels in the table above
were present in that catalog. Both Wave labels exist and are retained to avoid
changing program integrations. These template edits affect newly filed issues;
they do not relabel existing issues.

## Review open issues

```bash
# Paginate the REST endpoint; exclude pull requests returned by the issues API.
gh api --paginate 'repos/OtowoOrg/Stellar-K8s/issues?state=open&per_page=100' --jq '.[] | select(has("pull_request") | not) | [.number, .title, ([.labels[].name] | join(", "))] | @tsv'

# Review consumers before proposing a rename or deletion.
rg -n 'labels:|needs-triage|stellar-wave|Stellar Wave|200-points|security' .github CONTRIBUTING.md
```

Check for unlabeled issues, mismatched issue types, stale triage labels, and
duplicate labels with the same purpose. Keep `security` intact: the dependency
auto-merge workflow uses it to exclude security updates from automatic merging.
Also account for external integrations such as the Stellar Wave program, which
are not fully represented by a repository search.

## Verification and review record

Record the review date, missing template labels, affected issue numbers, proposed
changes, and automation impact in the maintenance issue. A complete review
accounts for every template label and documents any remaining discrepancies.
After approved changes, repeat the catalog comparison and inspect an affected
issue to confirm its labels. Update this table whenever template labels change.

See [GitHub's issue-form syntax](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)
and [the contributing guide](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md).
