# First-Class Issues

Every issue opened against Stellar-K8s belongs to exactly one **first-class
issue type**. The types are declared in
[`.github/issue-types.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/issue-types.yml),
which maps each type to its GitHub Issue Form, its title prefix, and the
labels GitHub applies automatically.

> The registry is validated in CI. Run `make issue-types-check` locally to
> reproduce the gate.

## Issue types

| Type | Title prefix | Labels | When to use |
|------|--------------|--------|-------------|
| `epic` | `[EPIC] ` | `epic`, `200-points`, `needs-triage` | A multi-part effort spanning several issues and weeks of work |
| `feature` | `[Feature]: ` | `enhancement` | A new capability or enhancement |
| `bug` | `[Bug]: ` | `bug`, `needs-triage` | A reproducible defect in the operator, charts, or scripts |
| `maintenance` | `[Chore]: ` | `maintenance` | Repository hygiene, CI, documentation, or dependency work |
| `support` | *(none)* | *(none)* | A guidance request; use [Discussions](https://github.com/OtowoOrg/Stellar-K8s/discussions) when possible |
| `wave` | `[Wave] ` | `stellar-wave` | A structured issue opened by a contributor in the Stellar Wave program |

Open an issue from the template chooser:
**<https://github.com/OtowoOrg/Stellar-K8s/issues/new/choose>**

## Why the registry exists

Before the registry, the issue forms and the issue tracker could drift apart:

- A new form could be added without a matching entry anywhere, so nobody
  knew which category it belonged to.
- A form could apply labels that the tracking metadata did not mention, which
  broke label-based triage and the `N-points` workflow automation.
- A title prefix could change in one place and not the other, which made
  searches and filters unreliable.

The registry removes that ambiguity. It is a small, reviewable file, and the
linter enforces the invariants described below.

## What the linter enforces

`make issue-types-check` (and the `Issue Template Lint` CI job) fails when:

1. A registered type points at a template file that does not exist.
2. An issue form in `.github/ISSUE_TEMPLATE/` is not registered as a
   first-class type.
3. A type and its form disagree about the labels they apply.
4. A type and its form disagree about the title prefix.
5. Two types share the same `id` or the same title prefix.
6. A required field (`id`, `name`, `description`, `template`) is missing.

Run the full gate, which covers the issue forms and the registry together:

```bash
make issue-template-lint
```

## Adding a new issue type

1. Create the GitHub Issue Form in `.github/ISSUE_TEMPLATE/<name>.yml`. It
   needs `name`, `description`, and a non-empty `body`; use only the
   `markdown`, `input`, `textarea`, `checkboxes`, and `dropdown` field types.
2. Register it in `.github/issue-types.yml`:

   ```yaml
   types:
     - id: security
       name: Security Report
       description: A security-sensitive defect that is not yet public
       title: "[Security]: "
       template: security_report.yml
       labels: [security, needs-triage]
       triage: needs-triage
   ```

3. Validate the change:

   ```bash
   make issue-types-check
   ```

4. Add a row to the table above and to
   [Reporting Issues](issue-reporting.md) so the new type is discoverable.

## Related documentation

- [Reporting Issues](issue-reporting.md) - what to include in a report
- [Issue Label Review](development/issue-label-review.md) - how labels are curated
- [Contributing Guide](contributing/index.md) - fork, branch, and pull request workflow
