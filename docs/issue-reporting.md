# Reporting Issues

Found a bug, have a feature request, or want to propose an improvement?
Use the GitHub issue tracker:

**<https://github.com/OtowoOrg/Stellar-K8s/issues/new/choose>**

## Quick Links

- **Getting started?** See [Testnet Onboarding Guide](getting-started/testnet-onboarding.md) or [Quick Start](getting-started/quick-start.md)
- **Documentation site:** [https://otowoorg.github.io/Stellar-K8s/](https://otowoorg.github.io/Stellar-K8s/)
- **Testnet peer ports:** [Testnet Peer Configuration](configuration/testnet-peer-configuration.md)

## Issue templates

Select the template that best matches your report:

| Template | When to use |
|----------|-------------|
| **Bug report** | Something is broken or behaves unexpectedly |
| **Feature request** | New capability or enhancement you'd like to see |
| **Epic** | A larger, multi-part effort spanning several issues |
| **Maintenance / chore** | Dependency bumps, refactors, CI improvements |
| **Support / question** | You're stuck and need guidance |

Each template is a *first-class issue type* with a fixed title prefix and a
fixed set of labels. The mapping is declared in
[`.github/issue-types.yml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/.github/issue-types.yml)
and validated in CI, so the tracker, the labels, and the automation that reads
them never drift apart. See
[First-Class Issue Types](first-class-issues.md) for the full table and for
how to add a new type.

## Before opening an issue

1. Search [existing issues](https://github.com/OtowoOrg/Stellar-K8s/issues) — your problem may already be tracked.
2. Check the [Troubleshooting Guide](troubleshooting/common-issues.md) for known solutions.
3. Review the [FAQ](faq.md) for common questions.

## What to include

A good bug report helps maintainers reproduce and fix the issue faster. Include:

- **Version** — operator version (`stellar-operator --version`) and Kubernetes version (`kubectl version`)
- **Steps to reproduce** — minimal sequence that triggers the bug
- **Expected vs actual behavior** — what you expected and what happened instead
- **Logs** — relevant pod logs (`kubectl logs`) or operator output
- **Environment** — cloud provider, cluster setup, Helm values (redact secrets)

## Security vulnerabilities

**Do not open a public issue for security vulnerabilities.**

Report them privately to <security@stellar.org> or via
[GitHub private vulnerability reporting](https://github.com/OtowoOrg/Stellar-K8s/security/advisories/new).
See [SECURITY.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/SECURITY.md) for the full disclosure policy.

## Contributing a fix

If you'd like to fix the issue yourself, see the [Contributing Guide](contributing/index.md)
for the fork-and-pull-request workflow, branch naming conventions, and commit message format.
