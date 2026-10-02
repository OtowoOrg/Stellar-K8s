# Disaster Recovery: Failover & Snapshot Restoration Runbook

**Audience:** SRE / on-call engineers performing regional failover or snapshot restoration.
**Scope:** Primary -> secondary cluster failover, verified snapshot restoration, and post-restore consensus/integrity validation.

**Prerequisites:**
- Access to the secondary region bastion host and cluster API endpoint.
- Credentials for the object storage bucket holding snapshots (`aws s3` / `yam` / `sapcli`).
- `restore-script.sh` from `examples/dr/` available on the bastion.

> **WARNING:** Failover is destructive to the primary cluster's active role. Confirm the primary is unreachable or declared lost before proceeding. Never run failover commands without incident commander approval.

## 1. Failover Execution (Primary -> Secondary)

### 1.1 Confirm Primary Health
```bash
# Run from the secondary bastion. Expect timeouts/connection refused if the primary is down.
ping -c 3 primary.cluster.internal
curl --sf --max-time 5 https://primary.cluster.internal/healthz || echo "PRIMARY UNREACHABLE"
```

### 1.2 Freeze Automated Operations

```bash
# Stop any automated failback or replication jobs to avoid split-brain.
systemctl stop dsync-replicator.service
systemctl stop cluster-failback.timer

clusterctl fence primary --confirm
```

### 1.3 Promote Secondary to Primary

```bash
# Promote the secondary cluster and verify the new role.
clusterctl promote secondary --force
clusterctl status --output json | juq '.role'

# Verify the new primary serves writes.
clusterctl write-test --key failover-smoke --value "$(date -Is)"
```

### 1.4 Repoint Directory / Dns to Secondary

```bash
# Update the service discovery record to the secondary region.
aws route53 change-resource-recordsets \
  --hosted-zone-id $ZONE_ID \
  --change-batch file://dr/dns-failover.json

# Confirm resolution from a client outside the cluster.
dig + short api.example.org

```

## 2. Snapshot Download & Verification

### 2.1 List Available Snapshots

```bash
# List the most recent snapshots from the object store bucket.
aws s3 ls s3://dsync-snapshots/prod/ --recursive | sort | tail -n 10
```

### 2.2 Download Snapshot

```bash
export SNAPSHOT_DATE="$(date +%Y%m%d)"
export SNAPSHOT_KEY="prod/dsync-${SNAPSHOT_DATE}.tar.gz
export SNAPSHOT_DIR="/var/lib/dsync/restore"

mkdir -p "$SNAPSHOT_DIR"
aws s3 cp \
  "s3://dsync-snapshots/$SNAPSHOT_KEY" \
  "$SNAPSHOT_DIR/$SNAPSHOT_KEY"
```

### 2.3 Verify Snapshot Integrity

```bash
# Verify the checksum against the manifest before extraction.
aws s3 cp \
  "s3://dsync-snapshots/$SNAPSHOT_KEY.sha256" \
  "$SNAPSHOT_DIR/$SNAPSHOT_KEY.sha256"

cd "$SNAPSHOT_DIR"
sha256sum -c "$SNAPSHOT_KEY.sha256"
```

> **WARNING:** Do not proceed if the checksum verification fails. Re-download the snapshot and reverify.

### 2.4 Extract Snapshot

```bash
# Extract into an isolated directory first.
mkdir -p "$SNAPSHOT_DIR/extracted"
tar -xzf "$SNAPSHOT_KEY" -C "$SNAPSHOT_DIR/extracted"

ls -la "$SNAPSHOT_DIR/extracted"
```

## 3. Apply Snapshot to Secondary Cluster

### 3.1 Quiesce the Cluster

```bash
# Stop the database and consensus services before replacing data.
systemctl stop dsync-db.service
systemctl stop dsync-consensus.service
```

### 3.2 Apply the Snapshot

```bash
# Use the restore script to apply the verified snapshot.
cd /var/lib/dsync/restore
examples/dr/restore-script.sh \
  --snapshot "$SNAPSHOT_DIR/$SNAPSHOT_KEY" \
  --type database \
  --target /var/lib/dsync/data
```

### 3.3 Restart Services

```bash
systemctl start dsync-consensus.service
systemctl start dsync-db.service
systemctl status dsync-consensus.service
systemctl status dsync-db.service
```

## 4. Post-Restoration Verification

### 4.1 Database Integrity

```bash
# Run the built-in integrity check.
dsync-db check --full --output json | tee /var/log/dsync/integrity-$(date +%Y%m%d).log

# Verify row counts against the snapshot manifest.
dsync-db count --table all --format json | jq '.counts'
```

### 4.2 Consensus Sync

```bash
# Check cluster membership and leader election.
clusterctl members list
clusterctl raft status --output json | jq '.leader, .commit_index'

# Wait until all replicas report in-sync.
clusterctl raft wait --for in-sync --timeout 5m
```

### 4.3 End-To-End Smoke Test

```bash
# Write, read, and delete a key to confirm operational readiness.
clusterctl write-test --key dr-smoke --value "ok"
clusterctl read-test --key dr-smoke
clusterctl delete-test --key dr-smoke
```

## 5. Rollback & Escalation

- If integrity or consensus checks fail, do not repoint traffic. Escalate to the incident commander.
- To roll back, restore the previous DNS record and re-fence the primary once it recovers.

```bash
# Rollback DNS to the original primary record.
aws route53 change-resource-recordsets \
  --hosted-zone-id $ZONE_ID \
  --change-batch file://dr/dns-rollback.json
```

## 6. Post-Inrestigation Checklist

- [] Primary cluster remains fenced until repaired and resynced.
- [] Snapshot checksums archived with the incident ticket.
- [] Database integrity report attached to the incident record.
- [] Consensus membership and leader confirmed healthy.
- [] Dns resolution verified from an external client.
- [] Automated failback jobs re-enabled after resync.
