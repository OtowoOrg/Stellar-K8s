# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Documentation
- Audited all TOML configuration examples for correct syntax highlighting (```toml) - all verified correct

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **Snapshot Bootstrap**: `spec.storage.snapshotRef` field on `StorageConfig` for near-instant
  node bootstrapping from pre-computed snapshots or compressed DB backups.
  - `volumeSnapshotName` / `volumeSnapshotNamespace`: provision PVC directly from a CSI
    `VolumeSnapshot` (zero-copy, no init container required).
  - `backupUrl` + `credentialsSecretRef`: inject a `snapshot-restore` init container that
    downloads and extracts a `.tar.gz` / `.tar.zst` archive from S3 or HTTPS before
    Stellar Core starts.
  - `restoreImage`: override the restore container image (defaults to `amazon/aws-cli:latest`
    for S3, `alpine:3` for HTTPS).
- **Auto-Snapshot Worker**: background Tokio task (`snapshot_worker`) that wakes every 60 s,
  evaluates cron schedules for all Validator nodes with `spec.snapshotSchedule`, and creates
  CSI `VolumeSnapshot` resources automatically — decoupled from the per-node reconcile loop.
- **Bootstrap Status Tracking**: `status.snapshotBootstrap` field on `StellarNodeStatus`
  tracks the full lifecycle of a snapshot-based bootstrap:
  - Phases: `Pending → Restoring → Restored → Syncing → Synced | Failed`
  - `secondsToSync`: elapsed seconds from restore completion to first `Synced` state.
    A value ≤ 600 satisfies the "synced within 10 minutes" acceptance criterion.
  - Kubernetes Events emitted at key transitions (`SnapshotBootstrapSynced`,
    `SnapshotBootstrapSlowSync`, `SnapshotBootstrapDeadlineExceeded`).
- **CRD update**: `snapshotRef` added to `StorageConfig` schema; `snapshotBootstrap` added
  to status schema in `config/crd/stellarnode-crd.yaml`.
- **Sample manifests**: `config/samples/snapshot-bootstrap-csi.yaml` and
  `config/samples/snapshot-bootstrap-backup.yaml` demonstrating both bootstrap modes.

### Changed
- `build_pvc` in `resources.rs` now resolves snapshot source with priority:
  `spec.storage.snapshotRef.volumeSnapshotName` > `spec.restoreFromSnapshot.volumeSnapshotName`.
- `build_pod_template` injects the `snapshot-restore` init container when
  `spec.storage.snapshotRef.backupUrl` is set (idempotent — skips if `/data` is already populated).
- `main.rs`: auto-snapshot worker spawned as a background task alongside the benchmark controller.

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
