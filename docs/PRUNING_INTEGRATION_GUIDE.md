# History Archive Pruning - Integration Guide

## Overview

The history archive pruning feature is now fully integrated into Stellar-K8s. This guide explains how the pruning system works, how it's integrated into the reconciliation loop, and how to use it.

## Architecture

### Components

1. **PruningPolicy (CRD)** - `src/crd/types.rs`
   - Configuration for pruning behavior
   - Validation of retention policies
   - Safety constraints (min checkpoints, max age)

2. **PruningWorker** - `src/controller/pruning_worker.rs`
   - Policy management and validation
   - Checkpoint safety checks
   - Retention criteria evaluation
   - Cron expression parsing for scheduling

3. **PruningReconciler** - `src/controller/pruning_reconciler.rs`
   - Bridges pruning_worker and archive_prune modules
   - Orchestrates pruning operations
   - Updates node status with results

4. **ArchivePrune** - `src/controller/archive_prune.rs`
   - Actual deletion operations
   - S3/GCS/local filesystem support
   - Checkpoint scanning and validation

### Integration Flow

```
StellarNode Reconciliation
    ↓
apply_stellar_node()
    ↓
[... other reconciliation steps ...]
    ↓
Archive Pruning (7c)
    ↓
reconcile_pruning()
    ├─ Check if pruning enabled
    ├─ Validate policy
    ├─ Check schedule
    ├─ Scan archives
    ├─ Identify deletable checkpoints
    ├─ Execute pruning (or dry-run)
    └─ Update status
    ↓
[... continue reconciliation ...]
```

## Usage

### Basic Configuration

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: my-validator
  namespace: stellar
spec:
  nodeType: Validator
  network: Testnet
  version: "v21.0.0"
  
  validator_config:
    seedSecretRef: "my-validator-seed"
    enableHistoryArchive: true
    historyArchiveUrls:
      - "s3://my-bucket/stellar-history"
  
  # Archive pruning configuration
  pruningPolicy:
    enabled: true
    retention_days: 30          # Keep 30 days of history
    min_checkpoints: 50         # Always keep at least 50 checkpoints
    max_age_days: 7             # Never delete checkpoints < 7 days old
    concurrency: 10             # 10 parallel deletions
    schedule: "0 2 * * *"       # Daily at 2 AM UTC
    auto_delete: false          # Dry-run mode (default)
    skip_confirmation: false    # Require confirmation
```

### Dry-Run Mode (Default)

By default, pruning runs in dry-run mode (`auto_delete: false`). This means:
- No actual deletions occur
- The operator analyzes what would be deleted
- Results are logged and stored in node status
- Safe for testing and validation

```yaml
pruningPolicy:
  enabled: true
  retention_days: 30
  auto_delete: false  # Dry-run mode
```

### Enable Actual Deletions

To enable actual deletions, set `auto_delete: true`:

```yaml
pruningPolicy:
  enabled: true
  retention_days: 30
  auto_delete: true   # Enable actual deletions
  skip_confirmation: true  # Skip confirmation prompt
```

### Retention Policies

#### Time-Based Retention

Keep the last N days of history:

```yaml
pruningPolicy:
  enabled: true
  retention_days: 30  # Keep 30 days
  retention_ledgers: null  # Not specified
```

#### Ledger-Based Retention

Keep the last N ledgers of history:

```yaml
pruningPolicy:
  enabled: true
  retention_days: null  # Not specified
  retention_ledgers: 1000000  # Keep 1 million ledgers
```

**Note**: You must specify exactly one of `retention_days` or `retention_ledgers`, not both.

### Safety Features

#### Minimum Checkpoint Retention

Always keep at least N recent checkpoints, regardless of age:

```yaml
pruningPolicy:
  min_checkpoints: 50  # Always keep 50 most recent checkpoints
```

- Default: 50
- Minimum: 10 (hardcoded safety limit)
- Prevents accidental deletion of all history

#### Maximum Age Protection

Never delete checkpoints newer than N days:

```yaml
pruningPolicy:
  max_age_days: 7  # Never delete checkpoints < 7 days old
```

- Default: 7 days
- Provides additional safety against recent data loss
- Works independently of retention policy

#### Mutual Exclusion

Cannot specify both `retention_days` and `retention_ledgers`:

```yaml
# ❌ INVALID - will fail validation
pruningPolicy:
  retention_days: 30
  retention_ledgers: 1000000

# ✅ VALID - specify only one
pruningPolicy:
  retention_days: 30
```

### Scheduling

Pruning can run on a schedule using cron expressions:

```yaml
pruningPolicy:
  schedule: "0 2 * * *"  # Daily at 2 AM UTC
```

Cron format: `minute hour day month weekday`

Examples:
- `"0 2 * * *"` - Daily at 2 AM UTC
- `"0 */6 * * *"` - Every 6 hours
- `"0 0 * * 0"` - Weekly on Sunday at midnight
- `"0 0 1 * *"` - Monthly on the 1st at midnight

### Concurrency Control

Control how many deletions happen in parallel:

```yaml
pruningPolicy:
  concurrency: 10  # 10 parallel deletions
```

- Default: 10
- Higher values speed up pruning but may hit API rate limits
- Adjust based on your cloud provider's limits

## Monitoring

### Node Status

Check pruning status in the node's status subresource:

```bash
kubectl get stellarnode my-validator -o jsonpath='{.status.pruningStatus}'
```

Output:
```json
{
  "lastRunTime": "2024-01-15T02:00:00Z",
  "lastRunStatus": "Success",
  "totalCheckpoints": 1000,
  "deletedCount": 50,
  "retainedCount": 950,
  "bytesFreed": 536870912,
  "message": "Pruned 50 checkpoints, freed 512.00 MB",
  "dryRun": false
}
```

### Kubernetes Events

Pruning operations emit Kubernetes events:

```bash
kubectl describe stellarnode my-validator
```

Look for events like:
- `ArchivePruningStarted` - Pruning operation started
- `ArchivePruningCompleted` - Pruning operation completed
- `ArchivePruningFailed` - Pruning operation failed

### Metrics

Prometheus metrics are exported (when metrics feature enabled):

```promql
# Archive pruning metrics
stellar_archive_pruning_total{namespace, name, network, status}
stellar_archive_pruning_bytes_freed{namespace, name, network}
stellar_archive_pruning_checkpoints_deleted{namespace, name, network}
```

## Troubleshooting

### Pruning Not Running

**Problem**: Pruning policy is configured but not running.

**Solutions**:
1. Check if pruning is enabled: `pruningPolicy.enabled: true`
2. Verify schedule is correct (use cron validator)
3. Check if last run time is recent: `kubectl get stellarnode -o jsonpath='{.status.pruningStatus.lastRunTime}'`
4. Look for errors in operator logs: `kubectl logs -n stellar-system deployment/stellar-operator`

### Dry-Run Mode

**Problem**: Pruning is running but not deleting anything.

**Solution**: This is expected if `auto_delete: false`. To enable deletions:
```yaml
pruningPolicy:
  auto_delete: true
```

### Validation Errors

**Problem**: Pruning policy fails validation.

**Common issues**:
- Both `retention_days` and `retention_ledgers` specified
- `min_checkpoints` < 10
- Invalid cron expression in `schedule`
- Missing required fields

**Solution**: Check node status for validation error message:
```bash
kubectl describe stellarnode my-validator
```

### Archive Not Found

**Problem**: "No history archives configured" message.

**Solution**: Ensure validator config includes archive URLs:
```yaml
validator_config:
  enableHistoryArchive: true
  historyArchiveUrls:
    - "s3://my-bucket/stellar-history"
```

## Best Practices

### 1. Start with Dry-Run

Always start with dry-run mode to validate your retention policy:

```yaml
pruningPolicy:
  enabled: true
  retention_days: 30
  auto_delete: false  # Dry-run first
```

Monitor the results in node status, then enable actual deletions.

### 2. Conservative Retention

Start with conservative retention periods:

```yaml
pruningPolicy:
  retention_days: 60  # 2 months instead of 30 days
  min_checkpoints: 100  # More than default 50
```

Gradually reduce as you gain confidence.

### 3. Schedule During Off-Peak

Schedule pruning during low-traffic periods:

```yaml
pruningPolicy:
  schedule: "0 2 * * *"  # 2 AM UTC (adjust for your timezone)
```

### 4. Monitor Disk Usage

Track freed space over time:

```bash
kubectl get stellarnode my-validator -o jsonpath='{.status.pruningStatus.bytesFreed}'
```

### 5. Test in Non-Production First

Test pruning policies on testnet before production:

```yaml
# Testnet - aggressive pruning
pruningPolicy:
  retention_days: 7
  auto_delete: true

# Mainnet - conservative pruning
pruningPolicy:
  retention_days: 60
  auto_delete: true
```

### 6. Coordinate with Backups

Ensure backups are taken before pruning:

```yaml
pruningPolicy:
  # Schedule after backup completes
  schedule: "0 3 * * *"  # 3 AM (after 2 AM backup)
```

## Cloud-Native Integration

### S3 Lifecycle Rules

Pruning works alongside S3 lifecycle rules:

```json
{
  "Rules": [
    {
      "Id": "DeleteOldCheckpoints",
      "Filter": {"Prefix": "stellar-history/"},
      "Expiration": {"Days": 90},
      "Status": "Enabled"
    }
  ]
}
```

Pruning provides:
- Finer-grained control (ledger-based retention)
- Immediate feedback (status updates)
- Kubernetes-native management

### GCS Lifecycle Rules

Similar to S3, GCS lifecycle rules can complement pruning:

```yaml
lifecycle:
  rule:
    - action:
        type: Delete
      condition:
        age: 90
        matchesPrefix:
          - stellar-history/
```

## Advanced Configuration

### Multiple Archives

Pruning handles multiple archive URLs:

```yaml
validator_config:
  historyArchiveUrls:
    - "s3://primary-bucket/archive"
    - "s3://backup-bucket/archive"
    - "gs://gcs-bucket/archive"

pruningPolicy:
  enabled: true
  retention_days: 30
```

Each archive is pruned independently.

### Dynamic Policy Updates

Update pruning policy without restarting:

```bash
kubectl patch stellarnode my-validator --type merge -p \
  '{"spec":{"pruningPolicy":{"retention_days":45}}}'
```

Changes take effect on next scheduled run.

## Implementation Details

### Validation

All pruning policies are validated before execution:

```rust
pub fn validate(&self) -> Result<(), String> {
    // Mutual exclusion check
    if self.retention_days.is_some() && self.retention_ledgers.is_some() {
        return Err("Cannot specify both retention_days and retention_ledgers".into());
    }
    
    // At least one retention policy required
    if self.retention_days.is_none() && self.retention_ledgers.is_none() {
        return Err("Must specify either retention_days or retention_ledgers".into());
    }
    
    // Minimum checkpoint safety
    if self.min_checkpoints < 10 {
        return Err("min_checkpoints must be at least 10".into());
    }
    
    Ok(())
}
```

### Checkpoint Safety

Checkpoints are only deleted if they meet ALL criteria:

1. Beyond minimum checkpoint buffer
2. Older than max_age_days
3. Exceed retention policy (days or ledgers)
4. Valid checkpoint structure

### Dry-Run Behavior

In dry-run mode:
- Archives are scanned
- Deletable checkpoints identified
- Results logged and stored
- No actual deletions occur
- Safe for validation

## Future Enhancements

Planned improvements:

1. **Incremental Pruning**: Prune in batches to avoid large operations
2. **Backup Integration**: Coordinate with backup/snapshot operations
3. **Multi-Archive Coordination**: Ensure consistency across archives
4. **Metrics Export**: Detailed Prometheus metrics for pruning operations
5. **Webhook Notifications**: Alert on pruning completion/failure
6. **Archive Verification**: Validate archive integrity after pruning

## Support

For issues or questions:

1. Check operator logs: `kubectl logs -n stellar-system deployment/stellar-operator`
2. Review node status: `kubectl describe stellarnode <name>`
3. Check pruning status: `kubectl get stellarnode <name> -o jsonpath='{.status.pruningStatus}'`
4. File an issue on GitHub with logs and configuration

## References

- [Archive Pruning Documentation](docs/archive-pruning.md)
- [Pruning Implementation](PRUNING_IMPLEMENTATION.md)
- [StellarNode API Reference](docs/api-reference.md)
- [Stellar History Archives](https://developers.stellar.org/docs/learn/storing-data/history-archives)
