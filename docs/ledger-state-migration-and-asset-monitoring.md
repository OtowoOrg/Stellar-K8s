# Ledger State Migration and Asset Monitoring

## Ledger snapshots

For an S3 export, configure `storage.snapshotRef.export` on the source node, suspend the node, then request the export. A CSI `VolumeSnapshot` is captured after the Core pod exits; the operator creates a snapshot-backed PVC and starts the upload Job from that stable copy. The source may be resumed as soon as the request annotation is cleared; the S3 transfer continues from the snapshot clone. The Job uploads a compressed data-directory archive, a SHA-256 sidecar, and an internal manifest with the ledger sequence and network. A CSI snapshot controller/default `VolumeSnapshotClass` must be installed.

```yaml
# Source validator
spec:
  suspended: true
  storage:
    snapshotRef:
      export:
        destination: s3://stellar-migration/validator-a
        credentialsSecretRef: ledger-export-credentials
```

Trigger export with `kubectl annotate stellarnode validator-a stellar.org/request-ledger-export=true --overwrite`. The export requires `spec.suspended: true`; it waits for the source pod to exit and for the CSI snapshot to become ready. Wait for the annotation to be cleared before resuming the source. Wait for the `ledger-export` Job to complete before creating the target node.

Configure the target with the archive URL emitted by the Job. The checksum sidecar is fetched and verified automatically; `sha256` can pin a digest explicitly. The init container also checks ledger sequence/network when configured and checks the per-file manifest before starting Stellar Core.

```yaml
# Target validator
spec:
  storage:
    snapshotRef:
      backupUrl: s3://stellar-migration/validator-a/ledger-state-123456.tar.gz
      credentialsSecretRef: ledger-export-credentials
      expectedLedgerSequence: 123456
      expectedNetwork: mainnet
```

Preserve the source cluster until the target has started, reached the expected ledger, and passed state verification. Archive transfer time depends on state size and object-storage throughput; the operator cannot guarantee a five-minute cutover bound. Existing archives without a checksum sidecar remain restorable, but should be pinned with `sha256` to get integrity verification.

## SAC monitoring

Install `config/crd/stellarassetmonitor-crd.yaml`, then create a namespaced watch list:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarAssetMonitor
metadata:
  name: issued-assets
  namespace: stellar
spec:
  enabled: true
  network: pubnet
  largeSupplyChangePercent: 10
  watchList:
    - assetCode: USD
      issuer: G...
      contractId: C...
```

The monitor processor accepts decoded TrustLine and ContractData changes through `controller::asset_monitor::process_ledger_change`. It exports per-asset supply, holders, liquidity, supply-change, and clawback metrics; the chart's Prometheus rules alert on large changes and clawbacks. This repository does not yet include a Stellar-Core ledger-entry decoder/stream adapter, so an upstream adapter must provide those decoded changes for metrics to update.