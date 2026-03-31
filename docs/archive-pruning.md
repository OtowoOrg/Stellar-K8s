# History Archive Pruning Guide

This guide explains how to safely prune old Stellar history archive checkpoints to manage storage costs while maintaining data integrity and availability.

## Overview

Stellar history archives grow continuously as new checkpoints are created every 64 ledgers (~5 minutes on mainnet). Over time, this can become expensive, especially for long-running validators maintaining full history.

The `prune-archive` utility provides safe, automated pruning with multiple safety guarantees to prevent accidental data loss.

## Safety Guarantees

The pruning utility implements multiple layers of protection:

### 1. **Dry-Run Mode (Default)**
- **All operations are dry-run by default**
- No deletions occur without explicit `--force` flag
- Preview exactly what would be deleted before committing

### 2. **Minimum Retention Buffer**
- Always retains at least the most recent **50 checkpoints** (configurable via `--min-checkpoints`)
- Hard minimum of **10 checkpoints** enforced regardless of configuration
- Ensures recent history is always available for node catch-up

### 3. **Maximum Age Protection**
- Checkpoints newer than `--max-age-days` (default: 7 days) are **never deleted**
- Prevents accidental deletion of recent checkpoints due to misconfiguration
- Provides time window to detect and correct pruning policy errors

### 4. **Checkpoint Validation**
- Validates checkpoint file structure before marking for deletion
- Skips corrupted or invalid checkpoints
- Reports validation errors in summary

### 5. **Atomic Operations**
- Each deletion is logged and auditable
- Errors during deletion don't stop the entire operation
- Failed deletions are reported for manual review

### 6. **Active Checkpoint Protection**
- Never deletes the current active checkpoint
- Preserves checkpoint metadata files (`.well-known/stellar-history.json`)
- Maintains archive integrity for ongoing operations

## Installation

The `prune-archive` subcommand is built into the `stellar-operator` binary:

```bash
# Build from source
cargo build --release --bin stellar-operator

# The command is available at:
./target/release/stellar-operator prune-archive --help
```

## Usage

### Basic Usage (Dry-Run Mode)

**Time-based retention (30 days):**
```bash
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-days 30
```

**Ledger-based retention (1 million ledgers):**
```bash
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-ledgers 1000000
```

This will:
1. Scan the archive for all checkpoints
2. Identify which checkpoints are eligible for deletion
3. Display a detailed summary of what would be deleted
4. **No actual deletions occur**

### Executing Actual Deletions

To perform actual deletions, add the `--force` flag:

```bash
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-days 30 \
  --force
```

With `--force`:
- Displays detailed list of checkpoints to be deleted
- Shows total storage space to be freed
- **Requires interactive confirmation** (unless `--yes` is also set)
- Performs deletions with progress reporting

### Skip Confirmation Prompt

For automated scripts, use `--yes` to skip the confirmation prompt:

```bash
stellar-operator prune-archive \
  --archive-url s3://my-bucket/stellar-history \
  --retention-days 30 \
  --force \
  --yes
```

⚠️ **Warning:** Use `--yes` with extreme caution in production environments.

## Configuration Options

### Archive URL (`--archive-url`)

**Required.** Specifies the archive location to prune.

Supported formats:
- **S3:** `s3://bucket-name/prefix/path`
- **GCS:** `gs://bucket-name/prefix/path`
- **Local:** `file:///path/to/archive`

Examples:
```bash
--archive-url s3://stellar-prod/history/mainnet
--archive-url gs://my-gcs-bucket/stellar/archive
--archive-url file:///var/lib/stellar/history
```

### Retention Period

Choose **one** of the following:

#### Time-Based Retention (`--retention-days`)

Delete checkpoints older than N days:

```bash
--retention-days 30    # Keep 30 days of history
--retention-days 90    # Keep 90 days of history
--retention-days 365   # Keep 1 year of history
```

#### Ledger-Based Retention (`--retention-ledgers`)

Delete checkpoints older than N ledgers:

```bash
--retention-ledgers 100000    # Keep ~100k ledgers (~5.5 days at 5s/ledger)
--retention-ledgers 1000000   # Keep ~1M ledgers (~55 days)
--retention-ledgers 10000000  # Keep ~10M ledgers (~1.8 years)
```

### Minimum Checkpoints (`--min-checkpoints`)

Always retain at least this many recent checkpoints, regardless of age:

```bash
--min-checkpoints 50   # Default: always keep 50 most recent
--min-checkpoints 100  # More conservative: keep 100
--min-checkpoints 20   # Less conservative (but min 10 enforced)
```

**Default:** 50  
**Minimum:** 10 (hardcoded safety limit)

### Maximum Age (`--max-age-days`)

Never delete checkpoints newer than this age, even if they exceed retention:

```bash
--max-age-days 7   # Default: never delete checkpoints < 7 days old
--max-age-days 14  # More conservative: 14 days
--max-age-days 1   # Less conservative: 1 day
```

**Default:** 7 days

This provides a safety buffer to catch configuration errors before data is lost.

### Concurrency (`--concurrency`)

Number of parallel deletion operations:

```bash
--concurrency 10   # Default: 10 concurrent deletions
--concurrency 20   # Faster but may hit API rate limits
--concurrency 5    # Slower but more conservative
```

**Default:** 10

Higher values speed up pruning but may trigger S3/GCS rate limiting.

## Examples

### Example 1: Conservative Pruning (Recommended for Production)

Keep 90 days of history, always retain 100 checkpoints, 14-day safety buffer:

```bash
stellar-operator prune-archive \
  --archive-url s3://stellar-prod/history \
  --retention-days 90 \
  --min-checkpoints 100 \
  --max-age-days 14 \
  --force
```

### Example 2: Ledger-Based Retention

Keep last 2 million ledgers (~110 days on mainnet):

```bash
stellar-operator prune-archive \
  --archive-url s3://stellar-prod/history \
  --retention-ledgers 2000000 \
  --min-checkpoints 50 \
  --force
```

### Example 3: Aggressive Pruning (Development/Testing)

Keep only 7 days, minimal safety buffers:

```bash
stellar-operator prune-archive \
  --archive-url file:///data/stellar/history \
  --retention-days 7 \
  --min-checkpoints 20 \
  --max-age-days 1 \
  --concurrency 20 \
  --force \
  --yes
```

### Example 4: Scheduled Pruning (Cron Job)

Add to crontab for weekly automated pruning:

```bash
# Run pruning every Sunday at 3 AM UTC
0 3 * * 0 /usr/local/bin/stellar-operator prune-archive \
  --archive-url s3://stellar-history \
  --retention-days 30 \
  --min-checkpoints 50 \
  --max-age-days 7 \
  --force \
  --yes \
  >> /var/log/stellar-prune.log 2>&1
```

### Example 5: Dry-Run Validation

Before running in production, validate the pruning policy:

```bash
# First, run dry-run to see what would be deleted
stellar-operator prune-archive \
  --archive-url s3://stellar-prod/history \
  --retention-days 30 \
  --min-checkpoints 50

# Review the output, then if satisfied:
stellar-operator prune-archive \
  --archive-url s3://stellar-prod/history \
  --retention-days 30 \
  --min-checkpoints 50 \
  --force \
  --yes
```

## Understanding the Output

### Dry-Run Output Example

```
Scanning archive for checkpoints...
Found 1250 checkpoints

Pruning Analysis:
  Total checkpoints:      1250
  Eligible for deletion:  750
  Will be retained:       500
  Space to be freed:    45.67 GB

=== Archive Pruning Summary ===
Total checkpoints found:      1250
Eligible for deletion:        750
Deleted:                      0
Retained:                     500
Space freed:                  45.67 GB
Dry-run mode:                 true

Deleted ledger sequences:
  (none - dry-run mode)
```

### Actual Deletion Output Example

```
⚠️  WARNING: You are about to permanently delete 750 checkpoints.
This will free 45.67 GB of storage space.

Deleted ledger sequences:
  - Ledger 1000
  - Ledger 1064
  - Ledger 1128
  ...

This operation CANNOT be undone.

Starting deletion of 750 checkpoints...
[====================] 100% (750/750)

=== Archive Pruning Summary ===
Total checkpoints found:      1250
Eligible for deletion:        750
Deleted:                      750
Retained:                     500
Space freed:                  45.67 GB
Dry-run mode:                 false

Deleted ledger sequences:
  1000, 1064, 1128, ..., 48500
```

## Archive Structure

Stellar history archives follow a standard directory structure:

```
archive/
├── .well-known/
│   └── stellar-history.json    # Archive metadata (NEVER deleted)
├── 00/
│   ├── 00/
│   │   ├── 00/
│   │   │   └── history-abc123...xdr.gz
│   │   └── ...
│   └── ...
└── ...
```

Each checkpoint file is named `history-{hash}.xdr.gz` and contains:
- Transaction set for the checkpoint ledger
- Merkle tree proofs
- Bucket list hash

The pruning utility understands this structure and safely navigates it.

## Best Practices

### DO ✅

- **Always run dry-run first** before using `--force`
- **Start with conservative retention** (90+ days) and adjust based on needs
- **Use `--max-age-days`** as a safety buffer (7-14 days recommended)
- **Monitor pruning operations** via logs and metrics
- **Test pruning in non-production** before deploying to production
- **Keep backups** of critical historical data
- **Document your retention policy** for team alignment

### DON'T ❌

- **Never use `--force --yes`** without running dry-run first
- **Don't set retention too aggressively** (< 30 days may break node catch-up)
- **Don't prune below minimum checkpoints** (always keep at least 50)
- **Don't ignore error messages** during pruning
- **Don't run concurrent pruning operations** on the same archive
- **Don't prune active archives** during critical operations

## Troubleshooting

### Issue: "No checkpoints found to prune"

**Cause:** Archive may be empty or path is incorrect.

**Solution:**
```bash
# Verify archive path
stellar-operator prune-archive \
  --archive-url s3://my-bucket/correct-path \
  --retention-days 30

# Check archive manually
aws s3 ls s3://my-bucket/correct-path/
```

### Issue: "Failed to delete checkpoint"

**Cause:** Permission issues, network errors, or rate limiting.

**Solution:**
1. Check IAM permissions (S3) or service account (GCS)
2. Reduce concurrency: `--concurrency 5`
3. Review error messages in output
4. Retry failed deletions manually

### Issue: "Retention policy too aggressive"

**Cause:** Attempting to delete more checkpoints than safety allows.

**Solution:**
- Increase `--min-checkpoints`
- Increase `--max-age-days`
- Use longer `--retention-days` or `--retention-ledgers`

### Issue: Pruning takes too long

**Cause:** Large archive, low concurrency, or network latency.

**Solution:**
- Increase concurrency: `--concurrency 20`
- Run during off-peak hours
- Consider more aggressive retention to reduce future pruning time

## Monitoring and Alerting

### Prometheus Metrics

Track pruning operations with these metrics:

```promql
# Last pruning timestamp
stellar_archive_prune_last_run_timestamp

# Checkpoints deleted per operation
stellar_archive_prune_checkpoints_deleted_total

# Bytes freed per operation
stellar_archive_prune_bytes_freed_total

# Pruning errors
stellar_archive_prune_errors_total
```

### Recommended Alerts

```yaml
# Alert if pruning fails
- alert: ArchivePruningFailed
  expr: increase(stellar_archive_prune_errors_total[1h]) > 0
  for: 5m
  labels:
    severity: warning
  annotations:
    summary: "Archive pruning encountered errors"

# Alert if archive grows too large
- alert: ArchiveSizeExcessive
  expr: stellar_archive_size_bytes > 1099511627776  # 1TB
  for: 1h
  labels:
    severity: warning
  annotations:
    summary: "History archive size exceeds 1TB"
```

## Security Considerations

### IAM Permissions (S3)

Minimum required permissions:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:ListBucket",
        "s3:GetObject",
        "s3:DeleteObject"
      ],
      "Resource": [
        "arn:aws:s3:::my-bucket",
        "arn:aws:s3:::my-bucket/stellar-history/*"
      ]
    }
  ]
}
```

### Service Account Permissions (GCS)

Minimum required roles:
- `storage.objects.list`
- `storage.objects.get`
- `storage.objects.delete`

### Audit Logging

All pruning operations are logged. Example log output:

```
INFO Starting archive pruning operation...
INFO Archive URL: s3://my-bucket/stellar-history
INFO Retention: Some(30) days, None ledgers
INFO Found 1250 checkpoints
INFO Identified 750 deletable and 500 retained checkpoints
INFO Starting deletion of 750 checkpoints...
INFO Deleted checkpoint: ledger 1000
INFO Deleted checkpoint: ledger 1064
...
INFO Pruning complete: 750 deleted, 0 errors
```

## Related Documentation

- [Stellar History Archive Documentation](https://developers.stellar.org/docs/learn/encyclopedia/history-archives)
- [Stellar-K8s Health Checks](health-checks.md)
- [Stellar-K8s Volume Snapshots](volume-snapshots.md)

## Support

For issues or questions about archive pruning:
- Open an issue on [GitHub](https://github.com/stellar/stellar-k8s/issues)
- Check existing documentation in the `docs/` directory
- Review error logs for detailed diagnostics
