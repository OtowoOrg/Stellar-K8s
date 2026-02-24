# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Integration tests for backup scheduler and remediation module
- Leader election implementation for high-availability setups
- Formal verification for Helm chart linting and manifest validation
- CVE test coverage for security scanning
- Support for building both operator and kubectl plugin binaries

### Changed
- Updated production dependencies (cargo bump)
- Improved CI/CD pipeline with additional validation steps

## [0.1.0] - 2026-01-19

### Added
- **Core Operator**: Kubernetes Operator for Stellar infrastructure written in Rust using `kube-rs`
- **StellarNode CRD**: Custom Resource Definition for defining Stellar node requirements (Network, Type, Resources)
- **Controller Logic**: Rust-based reconciliation loop that watches for changes and drives cluster state
- **Auto-Sync Health Checks**: Automatic monitoring of Horizon and Soroban RPC nodes, marking them Ready only when fully synced
- **Helm Chart**: Easy deployment with Helm 3.x including customizable values for production use
- **kubectl-stellar Plugin**: Convenient CLI plugin for interacting with StellarNode resources
  - `kubectl stellar list` - List all StellarNode resources
  - `kubectl stellar status` - Check sync status
  - `kubectl stellar logs` - View logs from nodes
- **CI/CD Pipeline**: GitHub Actions workflow with Docker builds and automated testing
- **Backup Scheduler**: Automated backup system with configurable schedules and decentralized storage options
- **Remediation Module**: Automatic handling of node failures with escalation policies (Restart → ClearAndResync)
- **Prometheus Metrics**: Native integration for monitoring node health, ledger sync status, and resource usage
- **Soroban RPC Support**: First-class support for Soroban RPC nodes with captive core configuration
- **Type-Safe Error Handling**: Comprehensive error handling using Rust's type system to prevent runtime failures
- **Finalizers**: Clean PVC and resource cleanup on node deletion
- **GitOps Ready**: Compatible with ArgoCD and Flux for declarative infrastructure management
- **Security**: TLS certificate generation, webhook admission controls, and plugin integrity verification

### Security
- TLS certificate management for secure node communication
- Webhook admission controls for validation
- Plugin integrity verification with SHA256

[Unreleased]: https://github.com/OtowoOrg/Stellar-K8s/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OtowoOrg/Stellar-K8s/releases/tag/v0.1.0