# Stellar-K8s Documentation

Welcome to the comprehensive documentation portal for **Stellar-K8s** - a Kubernetes-based infrastructure solution for deploying and managing Stellar blockchain nodes.

## :star: What is Stellar-K8s?

Stellar-K8s provides Kubernetes operators, Custom Resource Definitions (CRDs), and Helm charts to simplify the deployment, scaling, and management of Stellar network nodes including validators, Horizon API servers, and Soroban RPC nodes.

## :rocket: Key Features

- **Automated Node Management**: Deploy and manage Stellar validator nodes with Kubernetes operators
- **High Availability**: Built-in support for multi-node deployments with load balancing
- **Scalable Storage**: Dynamic volume provisioning and automatic disk scaling
- **Monitoring Integration**: Native Prometheus and Grafana dashboards
- **Security First**: RBAC policies, network policies, and secrets management
- **Easy Updates**: Rolling updates and automated version management

## :material-rocket-launch: Quick Links

<div class="grid cards" markdown>

- :fontawesome-solid-play: **Getting Started**

    ---

    New to Stellar-K8s? Start here to set up your first node

    [:octicons-arrow-right-24: Get Started](getting-started/index.md)

- :fontawesome-solid-rocket: **Deployment Guides**

    ---

    Step-by-step guides for deploying validators, Horizon, and Soroban RPC

    [:octicons-arrow-right-24: Deploy Nodes](deployment-guides/index.md)

- :fontawesome-solid-book: **Tutorials**

    ---

    Hands-on tutorials for common tasks and configurations

    [:octicons-arrow-right-24: View Tutorials](tutorials/index.md)

- :fontawesome-solid-wrench: **Troubleshooting**

    ---

    Solutions to common problems and diagnostic guides

    [:octicons-arrow-right-24: Get Help](troubleshooting/index.md)

</div>

## :material-download: Installation

Get started quickly with our installation guide:

```bash
# Clone the repository
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s

# Install with Helm
helm install stellar-k8s ./charts/stellar-k8s
```

[:octicons-arrow-right-24: Detailed Installation Instructions](getting-started/installation.md)

## :material-compass: Common Tasks

!!! tip "Popular Documentation Sections"

    - [Deploy a testnet validator node](tutorials/deploy-testnet-validator.md)
    - [Configure high-availability setup](tutorials/configure-ha-setup.md)
    - [Set up monitoring with Prometheus](deployment-guides/validator.md#monitoring)
    - [Troubleshoot sync problems](troubleshooting/sync-problems.md)
    - [Scale storage volumes](troubleshooting/disk-scaling.md)

## :material-help-circle: Need Help?

- **Documentation Search**: Use the search bar above to find specific topics
- **Troubleshooting Guide**: Check our [common issues](troubleshooting/common-issues.md) section
- **GitHub Issues**: Report bugs or request features on [GitHub](https://github.com/OtowoOrg/Stellar-K8s/issues)
- **Contributing**: Learn how to [contribute](contributing/index.md) to the project

## :material-star-circle: Project Status

Stellar-K8s is actively maintained and production-ready. We follow semantic versioning and provide migration guides for breaking changes.

!!! info "Latest Release"
    Check the [releases page](https://github.com/OtowoOrg/Stellar-K8s/releases) for the latest version and changelog.

---

**Ready to deploy your first Stellar node on Kubernetes?** Start with our [Quick Start Guide](getting-started/quick-start.md)!
