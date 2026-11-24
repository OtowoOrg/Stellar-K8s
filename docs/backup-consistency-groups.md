# Namespace-Scoped Backup Consistency Groups

Epic: #1527 · Module: `src/backup/consistency_group.rs`

A consistency group captures interdependent stateful workloads in one
namespace (for example DB + cache + queue) as a single recovery point instead
of snapshotting each volume in isolation.

```yaml
name: app
namespace: stellar
rpoSeconds: 3600
members:
  - name: db
    pvcs: [db-data]
    hook:                      # application-native hook
      quiesce: ["psql", "-c", "CHECKPOINT; SELECT pg_backup_start('group')"]
      unquiesce: ["psql", "-c", "SELECT pg_backup_stop()"]
    verify: ["pg_amcheck", "--all"]
  - name: cache
    pvcs: [cache-data]
    dependsOn: [db]            # no hook: falls back to a filesystem freeze
    verify: ["redis-check-rdb", "/data/dump.rdb"]
  - name: queue
    pvcs: [queue-data]
    dependsOn: [db]
    verify: ["rabbitmq-diagnostics", "check_running"]
```

## Backup

1. Members are quiesced in reverse dependency order (`queue`, `cache`, `db`)
   so nothing writes into a dependency after it is frozen.
2. All volumes are snapshotted in one step. Every `VolumeSnapshot` carries
   `stellar.org/consistency-group` and a shared
   `stellar.org/consistency-capture` label.
3. Members are unquiesced in dependency order. Every quiesced member is
   unquiesced even if a later step fails.

## Restore

Members are restored, started and verified in dependency order (`db`,
`cache`, `queue`). The `verify` command runs automatically after each member
becomes ready; if it fails, members that depend on it are not started and the
report lists every consistency error. A restore is healthy only when
`RestoreReport::is_consistent()` is true.

## RPO

`meets_rpo` measures the group from its single capture timestamp and only
passes when the capture covers every member volume, so the RPO applies to the
group rather than to each volume.
