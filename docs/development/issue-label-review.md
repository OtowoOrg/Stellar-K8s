# Issue label review

Review issue labels when changing an issue template and during routine maintainer
triage. Label creation, renaming, and removal should be agreed by maintainers
before changing the repository's catalog.

## Priority labels

The repository does not currently have priority labels. The following proposed
scheme documents how maintainers can prioritize issues if the labels are approved
and added to the GitHub catalog:

| Label | Use when |
| --- | --- |
| `priority-high` | An active security exposure, production outage or data-loss risk, or release-blocking defect needs prompt attention. |
| `priority-medium` | Normal planned work, an ordinary defect, or an issue whose urgency has not yet been established. |
| `priority-low` | Non-urgent documentation, polish, optional improvements, or test-only work. |

Priority is based on impact and urgency, not effort or issue type. Assign exactly
one priority label after reviewing an issue, and reassess it whenever impact
changes. Do not add these labels to issue forms until they exist in the catalog:
GitHub does not create missing labels from forms. This documentation change does
not create labels or change labels on existing issues.

## Compare templates with the live catalog

From an authenticated GitHub CLI session, list every upstream label:

```bash
gh api --paginate repos/OtowoOrg/Stellar-K8s/labels --jq '.[] | [.name, .description] | @tsv'
```

Compare the exact names with the top-level `labels` fields in
[the issue templates](https://github.com/OtowoOrg/Stellar-K8s/tree/main/.github/ISSUE_TEMPLATE).
The support form has no preset labels.

| Template | Requested labels |
| --- | --- |
| Bug report | `bug`, `needs-triage` |
| Feature request | `enhancement` |
| Maintenance / Chore | `maintenance` |
| 200-point Epic | `epic`, `200-points`, `needs-triage` |
| Stellar Wave Issue | `stellar-wave` |

Confirm every requested label exists; GitHub does not automatically create
missing labels for issue forms. Record missing labels and unclear or overlapping
descriptions for maintainer review.
In particular, check `stellar-wave` versus `Stellar Wave` before proposing
consolidation. Keep program and points labels until their automation consumers
have been reviewed.

The catalog review on **2026-10-16** found that `triage` and `chore` did not
exist upstream. The bug form now uses the existing `needs-triage` label and the
maintenance form requests only `maintenance`. Both Wave labels exist and are
retained to avoid changing program integrations. The title-case `Stellar Wave`
and lowercase `stellar-wave` labels are retained as a documented integration
exception: do not consolidate them until the external Stellar Wave integration
has been checked. Preset form labels affect new issues; they do not relabel
existing issues.

The catalog check on **2026-11-25** found no priority labels. It also found
duplicate-purpose aliases: `Stellar Wave` / `stellar-wave` and `good first
issue` / `good-first-issue` (both newcomer labels have the same description).
They remain unchanged pending maintainer review of their consumers.

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
Priority-label creation and issue-by-issue assignment require GitHub maintainer
permissions and are not part of this documentation-only scheme update. Update
this table whenever template labels change.

See [GitHub's issue-form syntax](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms)
and [the contributing guide](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md).
