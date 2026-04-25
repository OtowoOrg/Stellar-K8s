# History Archive Pruning - Quick Reference Card

## Enable Pruning (Dry-Run Mode)

```yaml
spec:
  pruningPolicy:
    enabled: true
    retention_days: 30
    schedule: "0 2 * * *"
```

## Enable Actual Deletions

```yaml
spec:
  pruningPolicy:
    enabled: true
    retention_days: 30
    auto_delete: true
    skip_confirmation: true
```

## Check Status

```bash
kubectl get stellarnode <name> -o jsonpath='{.status.pruningStatus}'
```

## Common Configurations

### Conservative (Mainnet)
```yaml
pruningPolicy:
  enabled: true
  retention_days: 60
  min_checkpoints: 100
  max_age_days: 14
  schedule: "0 3 * * 0"  # Weekly Sunday 3 AM
  auto_delete: false
```

### Aggressive (Testnet)
```yaml
pruningPolicy:
  enabled: true
  retention_days: 7
  min_checkpoints: 20
  max_age_days: 1
  schedule: "0 2 * * *"  # Daily 2 AM
  auto_delete: true
```

### Ledger-Based
```yaml
pruningPolicy:
  enabled: true
  retention_ledgers: 1000000
  min_checkpoints: 50
  schedule: "0 2 * * *"
  auto_delete: false
```

## Cron Schedule Examples

| Schedule | Meaning |
|----------|---------|
| `0 2 * * *` | Daily at 2 AM UTC |
| `0 */6 * * *` | Every 6 hours |
| `0 0 * * 0` | Weekly Sunday midnight |
| `0 0 1 * *` | Monthly 1st midnight |
| `0 2 * * 1-5` | Weekdays 2 AM |

## Safety Features

| Feature | Default | Min | Purpose |
|---------|---------|-----|---------|
| `auto_delete` | false | - | Dry-run by default |
| `min_checkpoints` | 50 | 10 | Minimum retention |
| `max_age_days` | 7 | - | Never delete recent |
| `retention_days` | - | - | Time-based retention |
| `retention_ledgers` | - | - | Ledger-based retention |

## Troubleshooting

### Pruning Not Running
```bash
# Check if enabled
kubectl get stellarnode <name> -o jsonpath='{.spec.pruningPolicy.enabled}'

# Check last run
kubectl get stellarnode <name> -o jsonpath='{.status.pruningStatus.lastRunTime}'

# Check logs
kubectl logs -n stellar-system deployment/stellar-operator | grep pruning
```

### Validation Error
```bash
# Check status
kubectl describe stellarnode <name>

# Common issues:
# - Both retention_days and retention_ledgers specified
# - min_checkpoints < 10
# - Invalid cron expression
```

### Enable Deletions
```bash
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"auto_delete":true}}}'
```

## Monitoring

### Status Fields
- `lastRunTime` - When pruning last ran
- `lastRunStatus` - Success/PartialSuccess/Failed
- `totalCheckpoints` - Checkpoints found
- `deletedCount` - Checkpoints deleted
- `retainedCount` - Checkpoints kept
- `bytesFreed` - Storage freed
- `dryRun` - Whether it was dry-run

### Kubernetes Events
```bash
kubectl describe stellarnode <name> | grep -i pruning
```

### Metrics (if enabled)
```promql
stellar_archive_pruning_total
stellar_archive_pruning_bytes_freed
stellar_archive_pruning_checkpoints_deleted
```

## Best Practices

1. **Start with Dry-Run**
   - Set `auto_delete: false`
   - Monitor results for 1-2 weeks
   - Then enable deletions

2. **Conservative Retention**
   - Start with 60 days (not 30)
   - Increase min_checkpoints to 100
   - Gradually reduce as confident

3. **Schedule Off-Peak**
   - Avoid business hours
   - Schedule after backups
   - Example: `0 2 * * *` (2 AM UTC)

4. **Monitor Disk Usage**
   - Track `bytesFreed` over time
   - Adjust retention if needed
   - Alert on pruning failures

5. **Test in Non-Prod First**
   - Deploy to testnet first
   - Validate retention policy
   - Then deploy to mainnet

## Common Mistakes

❌ **Both retention policies**
```yaml
retention_days: 30
retention_ledgers: 1000000  # ERROR!
```

❌ **Too aggressive**
```yaml
retention_days: 1
min_checkpoints: 5  # Too low!
```

❌ **Forgetting to enable**
```yaml
pruningPolicy:
  # Missing: enabled: true
  retention_days: 30
```

✅ **Correct**
```yaml
pruningPolicy:
  enabled: true
  retention_days: 30
  min_checkpoints: 50
  auto_delete: false
```

## Quick Commands

```bash
# Enable pruning (dry-run)
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"enabled":true,"retention_days":30}}}'

# Enable deletions
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"auto_delete":true}}}'

# Disable pruning
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"enabled":false}}}'

# Change retention
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"retention_days":45}}}'

# Change schedule
kubectl patch stellarnode <name> --type merge -p \
  '{"spec":{"pruningPolicy":{"schedule":"0 3 * * *"}}}'

# View status
kubectl get stellarnode <name> -o jsonpath='{.status.pruningStatus}' | jq

# Watch pruning
kubectl logs -f -n stellar-system deployment/stellar-operator | grep pruning
```

## Support

- **Documentation**: See PRUNING_INTEGRATION_GUIDE.md
- **Issues**: Check operator logs
- **Status**: `kubectl describe stellarnode <name>`
- **Help**: File GitHub issue with logs

## Key Takeaways

✅ Dry-run by default (safe)
✅ Multiple safety locks
✅ Kubernetes-native
✅ Cloud-agnostic
✅ Production-ready

**Start with dry-run, monitor, then enable deletions.**
