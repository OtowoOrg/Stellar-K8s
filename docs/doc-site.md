# Documentation Site

The full Stellar-K8s documentation is published at:

**<https://otowoorg.github.io/Stellar-K8s/>**

The site is built with [MkDocs Material](https://squidfunk.github.io/mkdocs-material/) and is automatically
deployed from the `main` branch whenever docs change.

## Quick Navigation

- 🚀 **New to Stellar-K8s testnet?** Start with [Testnet Onboarding Guide](getting-started/testnet-onboarding.md)
- 🔧 **Deploying to testnet?** See [Testnet Peer Configuration](configuration/testnet-peer-configuration.md)
- 🐛 **Report an issue?** Use [Issue Reporting Guide](issue-reporting.md)
- 📡 **Networking issues?** Check [Testnet Peer Egress Port Requirements](networking/testnet-egress-ports.md)

## What's in the docs

| Section | What you'll find |
|---------|-----------------|
| [Getting Started](getting-started/index.md) | Prerequisites, installation, quick-start |
| [Deployment Guides](deployment-guides/index.md) | Validator, Horizon, Soroban RPC |
| [Configuration](configuration/index.md) | CRD reference, storage, operators |
| [Tutorials](tutorials/index.md) | Step-by-step walkthroughs |
| [Operations & Observability](MONITORING_SETUP_GUIDE.md) | Monitoring, tracing, dashboards |
| [Troubleshooting](troubleshooting/index.md) | Common issues and diagnostic trees |
| [API Reference](api-reference.md) | CRD fields, REST endpoints, webhooks |
| [Security](security/compliance-guide.md) | RBAC, mTLS, secrets, pod security |
| [Networking](networking/index.md) | Network policies, peer discovery |
| [Contributing](contributing/index.md) | Dev setup, PR workflow, conventions |

## Building the docs locally

```bash
# One-time setup (requires Python 3.9+)
pip install mkdocs-material mkdocs-minify-plugin mkdocs-git-revision-date-localized-plugin

# Serve with live-reload
make docs-serve
# → opens http://127.0.0.1:8000

# Build static site into site/
make docs-build
```

## Editing a page

1. Find the source file under `docs/` (navigation lives in `mkdocs.yml`).
2. Edit the Markdown file.
3. Run `make docs-serve` to preview your changes.
4. Run `make link-check` to catch broken links before opening a PR.

See [docs/README.md](https://github.com/OtowoOrg/Stellar-K8s/blob/main/docs/README.md) for the full documentation maintenance guide.
