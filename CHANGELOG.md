# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Placeholder for upcoming features and changes

## [0.1.0] - 2024-02-25

### Added
- Initial release of Stellar-K8s Kubernetes Operator
- `StellarNode` Custom Resource Definition (CRD) for declarative node management
- Support for Stellar Core Validator nodes with StatefulSet deployment
- Support for Horizon API server nodes with Deployment
- Support for Soroban RPC nodes with captive core configuration
- Rust-based controller using `kube-rs` and `tokio` for high performance (~15MB binary)
- Auto-sync health checks for Horizon and Soroban RPC nodes
- Automatic readiness detection based on network sync status
- Built-in finalizers for clean PVC and resource cleanup
- Helm chart for easy operator installation (`charts/stellar-operator`)
- kubectl plugin (`kubectl-stellar`) for convenient node management
  - List all StellarNode resources
  - Check sync status
  - View logs from nodes
- Peer discovery mechanism for cross-cluster node communication
- MetalLB BGP Anycast support for high-availability networking
- Ingress configuration examples for external access
- Prometheus metrics integration for observability
- OpenTelemetry distributed tracing support
- REST API for operator management (optional feature)
- Admission webhook with WASM-based custom validation plugins
- Backup scheduler for automated node backups
- Leader election support for high-availability operator deployments
- Cross-cluster deployment examples:
  - Direct IP connectivity
  - External DNS integration
  - Istio service mesh
  - Submariner multi-cluster networking
- Canary rollout strategy for safe upgrades
- Custom metrics-based Horizontal Pod Autoscaling (HPA)
- CVE handling and security update examples
- Comprehensive documentation:
  - Quick start guides
  - Health checks documentation
  - Peer discovery guide
  - kubectl plugin usage
  - Ingress configuration guide
  - MetalLB BGP Anycast setup
  - Docker build optimization
  - Benchmarking guide
- CI/CD pipeline with GitHub Actions
- Docker multi-stage builds with optimization
- Grafana dashboard for monitoring
- k6 performance benchmarking suite
- E2E tests with KIND (Kubernetes in Docker)
- Dry-run testing capabilities
- Leader election tests

### Security
- Type-safe error handling to prevent runtime failures
- TLS certificate generation for webhook server using `rcgen`
- Rustls-based TLS implementation for secure communications
- SHA256-based integrity verification for WASM plugins
- Security policy documentation (SECURITY.md)

[unreleased]: https://github.com/Harbduls/Stellar-K8s/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Harbduls/Stellar-K8s/releases/tag/v0.1.0
