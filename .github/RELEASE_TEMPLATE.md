# Release v[VERSION]

**Release Date:** [DATE]

---

## 📋 Overview

<!-- Provide a brief summary of what this release accomplishes. This appears at the top of the release page. -->

A concise overview of the release highlights and major improvements.

---

## ✨ Features

<!-- List all new features added in this release -->
<!-- Format: - **Feature Name**: Brief description of the feature and its benefits -->

- **[Feature Name]**: Description of the feature. What problem does it solve? Why should users care?
- **[Feature Name]**: Description of the feature and its implementation details.

---

## 🐛 Bug Fixes

<!-- List all bugs fixed in this release -->
<!-- Format: - **[Issue/PR Link]** - Brief description of the bug and how it was fixed -->

- **[Issue #XXX]** - Description of the bug that was fixed.
- **[Issue #XXX]** - Description of the bug that was fixed.

---

## ⚠️ Breaking Changes

<!-- List any breaking changes that users need to be aware of -->
<!-- Format: - **[Change Name]**: Detailed explanation of what changed and migration path -->

- **[Breaking Change]**: Detailed explanation of what changed, why it was necessary, and how users should migrate their code/configuration.

<!-- If there are no breaking changes, you can remove this section or leave it with a note: -->
<!-- No breaking changes in this release. -->

---

## 📦 Docker Images

New container images are available for this release:

- `ghcr.io/0xolivanode/stellar-operator:v[VERSION]`
- `ghcr.io/0xolivanode/stellar-soroban-rpc:v[VERSION]`

Pull images with:
```bash
docker pull ghcr.io/0xolivanode/stellar-operator:v[VERSION]
```

---

## 🚀 Installation & Upgrade

### Using Helm

```bash
helm repo update stellar

# Fresh installation
helm install stellar-operator stellar/stellar-operator --version v[VERSION]

# Upgrade existing installation
helm upgrade stellar-operator stellar/stellar-operator --version v[VERSION]
```

### Using kubectl

```bash
kubectl apply -f https://github.com/0xOlivanode/Stellar-K8s/releases/download/v[VERSION]/stellar-operator.yaml
```

### Using Operator Lifecycle Manager (OLM)

```bash
operator-sdk run bundle ghcr.io/0xolivanode/stellar-operator-bundle:v[VERSION]
```

---

## 📚 Documentation

- [Installation Guide](https://github.com/0xOlivanode/Stellar-K8s/blob/v[VERSION]/docs/getting-started.md)
- [API Reference](https://github.com/0xOlivanode/Stellar-K8s/blob/v[VERSION]/docs/api-reference.md)
- [Changelog](https://github.com/0xOlivanode/Stellar-K8s/blob/v[VERSION]/CHANGELOG.md)

---

## 👥 Contributors

<!-- List all contributors who made this release possible -->
<!-- Format: @username, @username -->

Thank you to all contributors who made this release possible:

- @[username]
- @[username]
- @[username]

---

## 🔍 Known Issues

<!-- List any known issues or limitations in this release (optional) -->

<!-- Example:
- [Issue #XXX] - Brief description of the known issue and any workarounds.
-->

---

## 📊 Release Statistics

| Metric | Count |
|--------|-------|
| Commits | [#] |
| Issues Closed | [#] |
| PRs Merged | [#] |
| New Contributors | [#] |

---

## 🙏 Support

If you encounter any issues with this release:

1. Check the [documentation](https://github.com/0xOlivanode/Stellar-K8s/blob/v[VERSION]/docs)
2. Search [existing issues](https://github.com/0xOlivanode/Stellar-K8s/issues)
3. Review the [SECURITY.md](https://github.com/0xOlivanode/Stellar-K8s/blob/v[VERSION]/SECURITY.md) for security concerns
4. Open a new issue if the problem is not already reported

---

## 🔗 Links

- **GitHub Repository**: https://github.com/0xOlivanode/Stellar-K8s
- **Full Changelog**: [Compare with previous release](https://github.com/0xOlivanode/Stellar-K8s/compare/v[PREVIOUS_VERSION]...v[VERSION])
- **Documentation**: https://github.com/0xOlivanode/Stellar-K8s/tree/v[VERSION]/docs

---

## How to Use This Template

1. **Before Publishing**: Replace all placeholder values:
   - `[VERSION]` → e.g., `1.0.0`
   - `[DATE]` → Release date, e.g., `2026-04-28`
   - `[PREVIOUS_VERSION]` → e.g., `0.9.0`
   - `[Feature Name]` → Actual feature names with descriptions
   - `[Issue #XXX]` → Replace with actual issue numbers
   - `@[username]` → Replace with actual GitHub usernames

2. **Sections to Customize**:
   - **Overview**: Keep this brief (1-2 sentences)
   - **Features**: Add all new features with meaningful descriptions
   - **Bug Fixes**: Reference PR/Issue numbers for traceability
   - **Breaking Changes**: Be explicit about migration paths
   - **Contributors**: Thank everyone who contributed
   - **Known Issues**: Only include if there are known problems
   - **Release Statistics**: Update with actual metrics

3. **Best Practices**:
   - Keep the tone professional and user-focused
   - Use clear, descriptive language
   - Provide links to relevant documentation
   - Include migration guides for breaking changes
   - Highlight security-related updates
   - Add examples where helpful

4. **Publishing**:
   - Create a new release on GitHub
   - Copy this template content into the release description
   - Attach binaries/artifacts if applicable
   - Select "Create a discussion for this release" if desired
   - Publish the release

---

**Template Version:** 1.0  
**Last Updated:** 2026-04-28
