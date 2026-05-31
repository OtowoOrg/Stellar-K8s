# Automated Backup Verification via Temporary Clusters

## Overview

Stellar-K8s provides automated verification of database backups by spinning up temporary Kubernetes clusters, restoring backups, and validating data integrity. This ensures that backups are recoverable and meet recovery time objectives (RTO) and recovery point objectives (RPO).

## Features

- **Automated Restore Testing**: Periodic verification of backup recoverability
- **Temporary Cluster Provisioning**: Ephemeral namespaces for isolated testing
- **Data Integrity Checks**: Checksums, row counts, and sample queries
- **Performance Benchmarking**: Query performance validation on restored data
- **Configurable Strategies**: Quick, standard, or full verification
- **Automatic Cleanup**: Temporary resources cleaned up after verification
- **Detailed Reports**: Verification reports stored in S3 for compliance
- **Multiple Backup Sources**: Support for S3, VolumeSnapshots, and pgBackRest

## Architecture

The backup verification process follows these steps:

1. **Create Temporary Namespace**: Isolated environment for testing
2. **Deploy PostgreSQL**: Temporary database instance
3. **Restore Backup**: Load backup data from configured source
4. **Run Integrity Checks**: Validate data consistency and completeness
5. **Run Benchmarks**: Test query performance (optional)
6. **Generate Report**: Detailed verification report
7. **Cleanup Resources**: Remove temporary namespace and resources

```
┌─────────────────────────────────────────────────────────────┐
│              Backup Verification Flow                        │
└─────────────────────────────────────────────────────────────┘

  ┌──────────────┐
  │   Scheduler  │  (Cron-based)
  └──────┬───────┘
         │
         ▼
  ┌──────────────────┐
  │ Create Namespace │  (verify-node-timestamp)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Deploy Postgres  │  (StatefulSet + Service)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Restore Backup   │  (S3/Snapshot/pgBackRest)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Integrity Checks │  (Checksums, Queries)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │   Benchmarks     │  (Optional)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Generate Report  │  (Store in S3)
  └──────┬───────────┘
         │
         ▼
  ┌──────────────────┐
  │ Cleanup Namespace│
  └──────────────────┘
```

## Configuration

### Basic Configuration

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-horizon
  namespace: stellar
spec:
  nodeType: Horizon
  network: Testnet
  version: "v21.0.0"

  managedDatabase:
    enabled: true
    backup:
      enabled: true
      schedule: "0 2 * * *"
      destinationPath: "s3://stellar-backups/horizon"

  backupVerification:
    enabled: true
    schedule: "0 2 * * 0" # Weekly on Sunday

    backupSource:
      type: s3
      bucket: stellar-backups
      region: us-east-1
      prefix: horizon/
      credentialsSecret: aws-credentials

    strategy: standard
    timeoutMinutes: 60
```

### Advanced Configuration

```yaml
backupVerification:
  enabled: true

  # Verification schedule (cron format)
  schedule: "0 2 * * 0" # Weekly on Sunday at 2 AM

  # Backup source configuration
  backupSource:
    type: s3
    bucket: stellar-backups
    region: us-east-1
    prefix: horizon-testnet/
    credentialsSecret: aws-credentials

  # Verification strategy: quick, standard, full
  strategy: full

  # Timeout for verification process (in minutes)
  timeoutMinutes: 120

  # Enable performance benchmarking
  benchmarkEnabled: true

  # Notification webhook for verification results
  notificationWebhook: "https://slack.example.com/hooks/backup-verification"

  # S3 bucket for storing verification reports
  reportStorage:
    bucket: stellar-reports
    region: us-east-1
    prefix: verification-reports/

  # Resource limits for temporary verification pods
  resources:
    cpuLimit: "4000m"
    memoryLimit: "8Gi"
    storageSize: "200Gi"
```

## Backup Sources

### S3 Backups

```yaml
backupSource:
  type: s3
  bucket: stellar-backups
  region: us-east-1
  prefix: horizon-testnet/
  credentialsSecret: aws-credentials
```

**Requirements**:

- AWS credentials in Kubernetes Secret
- S3 bucket with backup files (pg_dump format)
- Network access to S3 from cluster

### VolumeSnapshot Backups

```yaml
backupSource:
  type: volumeSnapshot
  snapshotName: "validator-snapshot-latest"
  storageClass: "fast-ssd"
```

**Requirements**:

- CSI driver with snapshot support
- VolumeSnapshot resource exists
- Storage class supports volume cloning

### pgBackRest Backups

```yaml
backupSource:
  type: pgbackrest
  repoPath: "/pgbackrest/repo"
  stanza: "horizon-testnet"
```

**Requirements**:

- pgBackRest repository accessible
- Stanza configured and valid
- Network access to repository

## Verification Strategies

### Quick Verification

**Duration**: ~5 minutes  
**Checks**:

- Database connectivity
- Table existence
- Basic checksums

**Use Case**: Frequent verification with minimal resource usage

```yaml
strategy: quick
```

### Standard Verification (Default)

**Duration**: ~15-30 minutes  
**Checks**:

- Database connectivity
- Table existence
- Row counts per table
- Sample queries (last 10 ledgers)

**Use Case**: Regular verification with balanced coverage

```yaml
strategy: standard
```

### Full Verification

**Duration**: ~60-120 minutes  
**Checks**:

- Database connectivity
- Table existence
- Row counts per table
- Full table scans
- Sample queries
- Performance benchmarks

**Use Case**: Comprehensive verification for compliance

```yaml
strategy: full
benchmarkEnabled: true
```

## Verification Checks

### 1. Database Connectivity

Verifies that the restored database is accessible:

```sql
SELECT 1
```

### 2. Table Existence

Checks for expected Horizon/Stellar Core tables:

- `accounts`
- `ledgers`
- `transactions`
- `operations`
- `assets`
- `offers`

### 3. Row Counts

Counts rows in each table to verify data completeness:

```sql
SELECT COUNT(*) FROM ledgers;
SELECT COUNT(*) FROM transactions;
SELECT COUNT(*) FROM operations;
```

### 4. Sample Queries

Executes representative queries to verify functionality:

```sql
-- Latest ledgers
SELECT * FROM ledgers ORDER BY sequence DESC LIMIT 10;

-- Recent transactions
SELECT * FROM transactions ORDER BY created_at DESC LIMIT 100;

-- Account balances
SELECT * FROM accounts WHERE balance > 0 LIMIT 10;
```

### 5. Performance Benchmarks

Measures query performance on restored data:

- Queries per second (QPS)
- Average query latency
- P95 query latency
- P99 query latency

## Verification Reports

Reports are generated in JSON format and stored in S3:

```json
{
  "timestamp": "2026-04-25T02:00:00Z",
  "namespace": "stellar",
  "nodeName": "my-horizon",
  "backupSource": "s3://stellar-backups/horizon-testnet/",
  "status": "success",
  "durationSeconds": 1847,
  "checks": [
    {
      "name": "DatabaseConnectivity",
      "passed": true,
      "message": "Database is accessible",
      "durationMs": 234
    },
    {
      "name": "TableExistence",
      "passed": true,
      "message": "All expected tables exist",
      "durationMs": 156
    },
    {
      "name": "RowCounts",
      "passed": true,
      "message": "Row counts: {\"accounts\": 1234567, \"ledgers\": 987654, ...}",
      "durationMs": 5432
    }
  ],
  "benchmarkResults": {
    "queriesPerSecond": 245.3,
    "avgQueryLatencyMs": 4.08,
    "p95QueryLatencyMs": 12.5,
    "p99QueryLatencyMs": 23.7
  }
}
```

### Report Storage

Reports are stored in S3 with the following structure:

```
s3://stellar-reports/
  verification-reports/
    stellar/
      my-horizon-20260425-020000.json
      my-horizon-20260418-020000.json
      my-horizon-20260411-020000.json
```

## Resource Requirements

### Temporary Namespace Resources

Each verification creates:

- 1 Namespace (temporary)
- 1 StatefulSet (PostgreSQL)
- 1 Service (PostgreSQL)
- 1 PersistentVolumeClaim (data storage)
- 1 Job (backup restore, if applicable)

### Recommended Resource Limits

| Node Type           | CPU Limit | Memory Limit | Storage Size |
| ------------------- | --------- | ------------ | ------------ |
| Horizon (Testnet)   | 2000m     | 4Gi          | 100Gi        |
| Horizon (Mainnet)   | 4000m     | 8Gi          | 500Gi        |
| Validator (Testnet) | 2000m     | 4Gi          | 100Gi        |
| Validator (Mainnet) | 4000m     | 8Gi          | 1Ti          |

## Monitoring

### Prometheus Metrics

```
# Total number of backup verifications
stellar_operator_backup_verifications_total{namespace, node_name, status}

# Duration of backup verification in seconds
stellar_operator_backup_verification_duration_seconds{namespace, node_name}

# Last successful verification timestamp
stellar_operator_backup_verification_last_success_timestamp{namespace, node_name}

# Number of failed verification checks
stellar_operator_backup_verification_failed_checks{namespace, node_name}
```

### Grafana Dashboard

Import the backup verification dashboard from `monitoring/backup-verification-dashboard.json`:

- Verification success rate
- Verification duration trends
- Failed checks by type
- Time since last successful verification
- Benchmark performance trends

## Troubleshooting

### Common Issues

#### 1. Namespace Creation Failure

**Symptom**: Verification fails with "Failed to create temporary namespace"

**Solution**:

- Check operator has permissions to create namespaces
- Verify cluster has available resources
- Check for namespace naming conflicts

#### 2. PostgreSQL Deployment Timeout

**Symptom**: Verification fails with "PostgreSQL not ready"

**Solution**:

- Increase `timeoutMinutes` configuration
- Check storage class is available
- Verify sufficient cluster resources

#### 3. Backup Restore Failure

**Symptom**: Verification fails with "Failed to restore backup"

**Solution**:

- Verify backup source is accessible
- Check AWS credentials (for S3)
- Verify backup format is compatible
- Check network connectivity

#### 4. Integrity Check Failures

**Symptom**: Verification completes with failed checks

**Solution**:

- Review verification report for specific failures
- Verify backup was taken correctly
- Check for data corruption in backup
- Validate backup retention policy

### Manual Verification

To trigger a manual verification outside the schedule:

```bash
# Add annotation to trigger immediate verification
kubectl annotate stellarnode my-horizon \
  stellar.org/verify-backup=true \
  -n stellar
```

### View Verification Logs

```bash
# Check operator logs
kubectl logs -n stellar-system \
  -l app=stellar-operator \
  --tail=100 | grep "backup verification"

# Check temporary namespace (during verification)
kubectl get all -n verify-my-horizon-1714017600

# View verification report
aws s3 cp s3://stellar-reports/verification-reports/stellar/my-horizon-20260425-020000.json -
```

## Best Practices

1. **Regular Schedule**: Verify backups at least weekly
2. **Test Restores**: Use full verification strategy monthly
3. **Monitor Reports**: Set up alerts for failed verifications
4. **Resource Planning**: Ensure cluster has capacity for temporary resources
5. **Retention Policy**: Keep verification reports for compliance (90+ days)
6. **Network Isolation**: Use separate namespaces for verification
7. **Cleanup Verification**: Monitor for orphaned temporary namespaces
8. **Benchmark Trends**: Track performance trends over time

## Compliance

Backup verification helps meet compliance requirements:

- **SOC 2**: Backup and recovery testing
- **ISO 27001**: A.12.3.1 (Information backup)
- **PCI DSS**: Requirement 9.5 (Backup media protection)
- **HIPAA**: 164.308(a)(7)(ii)(A) (Data backup plan)

## Performance Impact

### Cluster Resources

- Temporary namespace: ~100-500Gi storage
- CPU usage: 2-4 cores during verification
- Memory usage: 4-8Gi during verification
- Network: S3 download bandwidth

### Production Impact

- **Zero impact**: Verification runs in isolated namespace
- **No downtime**: Production nodes unaffected
- **Scheduled**: Runs during low-traffic periods

## Examples

See `examples/backup-verification-example.yaml` for complete configuration examples.

## Related Documentation

- [Secret Rotation](secret-rotation.md)
- [Disaster Recovery](dr-failover.md)
- [Volume Snapshots](volume-snapshots.md)
- [Database Management](database-management.md)
