# Design: Declarative Backup Plans with PITR

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Kubernetes API Server                         │
├─────────────────────────────────────────────────────────────────┤
│  BackupPlan CR  │  BackupJob CR  │  RestoreRequest CR           │
└────────┬────────┴────────┬───────┴────────┬────────────────────┘
         │                 │                │
         │                 │                │
    ┌────▼─────────────────▼────────────────▼───────┐
    │         Backup Controller                      │
    │  ┌──────────────────────────────────────────┐ │
    │  │  RPO Scheduler                           │ │
    │  │  - Calculate backup cadence              │ │
    │  │  - Create BackupJob CRs                  │ │
    │  └──────────────────────────────────────────┘ │
    │  ┌──────────────────────────────────────────┐ │
    │  │  Backup Executor                         │ │
    │  │  - VolumeSnapshot creation               │ │
    │  │  - Datastore-native backup               │ │
    │  │  - PITR metadata capture                 │ │
    │  └──────────────────────────────────────────┘ │
    │  ┌──────────────────────────────────────────┐ │
    │  │  Restore Verifier                        │ │
    │  │  - Test restore execution                │ │
    │  │  - Integrity validation                  │ │
    │  │  - Cleanup                               │ │
    │  └──────────────────────────────────────────┘ │
    │  ┌──────────────────────────────────────────┐ │
    │  │  Replication Manager                     │ │
    │  │  - Cross-region copy                     │ │
    │  │  - Checksum verification                 │ │
    │  └──────────────────────────────────────────┘ │
    │  ┌──────────────────────────────────────────┐ │
    │  │  Retention Manager                       │ │
    │  │  - Apply retention policies              │ │
    │  │  - Delete expired backups                │ │
    │  └──────────────────────────────────────────┘ │
    └───────────┬────────────────────┬───────────────┘
                │                    │
    ┌───────────▼──────┐  ┌──────────▼──────────┐
    │  VolumeSnapshot  │  │  Backup Storage     │
    │  (CSI)           │  │  (S3/GCS/Azure)     │
    └──────────────────┘  └─────────────────────┘
```

## Custom Resource Definitions

### 1. BackupPlan CRD

```yaml
apiVersion: backup.stellar.io/v1
kind: BackupPlan
metadata:
  name: postgres-production-backup
  namespace: database
spec:
  # Target workload
  target:
    kind: StatefulSet
    name: postgres
    namespace: database
    # Optional label selector
    selector:
      matchLabels:
        app: postgres
        tier: production
  
  # Backup strategy
  strategy:
    # Recovery Point Objective
    rpo: 1h
    # Recovery Time Objective (for validation)
    rto: 15m
    
    # Datastore type for specialized handling
    datastoreType: postgresql
    
    # Enable PITR
    pitr:
      enabled: true
      retentionDays: 7
      
  # Storage configuration
  storage:
    # Primary storage location
    primary:
      provider: s3
      bucket: stellar-backups
      prefix: postgres/production/
      region: us-west-2
      # Reference to credentials secret
      credentialsSecret:
        name: backup-s3-credentials
        key: credentials
    
    # Cross-region replication
    replicas:
      - provider: s3
        bucket: stellar-backups-dr
        region: us-east-1
        retentionDays: 30
      - provider: gcs
        bucket: stellar-backups-gcp
        project: stellar-prod
        retentionDays: 90
  
  # Retention policy
  retention:
    # Keep backups for 30 days
    days: 30
    # Always keep at least 3 backups
    minimum: 3
  
  # Restore verification
  verification:
    enabled: true
    # Run verification on every Nth backup
    frequency: 1
    # Timeout for verification
    timeout: 30m
    # Custom verification queries
    checks:
      - type: checksum
        query: "SELECT COUNT(*) FROM users;"
        expected: "> 0"
      - type: query
        query: "SELECT pg_is_in_recovery();"
        expected: "false"
  
  # Scheduling constraints
  schedule:
    # Preferred backup windows (UTC)
    windows:
      - start: "02:00"
        end: "04:00"
    # Prevent backups during these periods
    blackoutWindows:
      - start: "12:00"
        end: "13:00"
        reason: "Peak traffic hours"

status:
  # Calculated backup cadence
  calculatedCadence: 1h
  
  # Last backup info
  lastBackup:
    name: postgres-production-20260928-030015
    timestamp: "2026-09-28T03:00:15Z"
    status: Complete
    size: 5.2GB
    duration: 3m42s
  
  # RPO achievement tracking
  rpoAchievement:
    target: 1h
    actual: 58m
    achievementRate: 98.5%
  
  # PITR window
  pitrWindow:
    earliest: "2026-09-21T03:00:00Z"
    latest: "2026-09-28T03:00:15Z"
  
  # Replication status
  replicationStatus:
    - region: us-east-1
      lastReplicated: "2026-09-28T03:05:00Z"
      lag: 4m45s
      status: Healthy
    - region: gcp-us-central1
      lastReplicated: "2026-09-28T03:06:30Z"
      lag: 6m15s
      status: Healthy
  
  conditions:
    - type: BackupScheduled
      status: "True"
      lastTransitionTime: "2026-09-28T03:00:00Z"
    - type: VerificationPassed
      status: "True"
      lastTransitionTime: "2026-09-28T03:10:00Z"
```

### 2. BackupJob CRD

```yaml
apiVersion: backup.stellar.io/v1
kind: BackupJob
metadata:
  name: postgres-production-20260928-030015
  namespace: database
  labels:
    backup-plan: postgres-production-backup
spec:
  # Reference to parent BackupPlan
  backupPlanRef:
    name: postgres-production-backup
  
  # Backup target
  target:
    kind: StatefulSet
    name: postgres
    namespace: database
  
  # Datastore type
  datastoreType: postgresql
  
  # Storage location
  storage:
    provider: s3
    bucket: stellar-backups
    path: postgres/production/20260928-030015/
  
  # PITR configuration
  pitr:
    enabled: true
    walArchiveLocation: s3://stellar-backups/postgres/production/wal/

status:
  phase: Complete
  startTime: "2026-09-28T03:00:15Z"
  completionTime: "2026-09-28T03:03:57Z"
  duration: 3m42s
  
  # Backup metadata
  backup:
    size: 5.2GB
    checksum: sha256:abc123...
    snapshotId: snap-0abc123
    
  # PITR metadata
  pitr:
    latestWAL: 000000010000000000000042
    lsn: 0/42000000
    timestamp: "2026-09-28T03:03:57Z"
  
  # Verification status
  verification:
    phase: Passed
    startTime: "2026-09-28T03:05:00Z"
    completionTime: "2026-09-28T03:10:00Z"
    checks:
      - type: checksum
        status: Passed
        result: "12345"
      - type: query
        status: Passed
        result: "false"
  
  # Replication status
  replication:
    - region: us-east-1
      status: Complete
      checksum: sha256:abc123...
      completionTime: "2026-09-28T03:05:00Z"
```

### 3. RestoreRequest CRD

```yaml
apiVersion: backup.stellar.io/v1
kind: RestoreRequest
metadata:
  name: restore-postgres-20260928
  namespace: database
spec:
  # Source backup
  source:
    # Option 1: Restore from specific backup
    backupJobRef:
      name: postgres-production-20260928-030015
    
    # Option 2: Restore to PITR timestamp
    # pitr:
    #   timestamp: "2026-09-28T02:45:00Z"
    #   backupPlanRef:
    #     name: postgres-production-backup
  
  # Target for restore
  target:
    # Restore to new namespace for testing
    namespace: database-restore-test
    # Optional: rename resources
    namePrefix: restored-
  
  # Restore options
  options:
    # Include PITR recovery
    pitr: true
    # Verify after restore
    verify: true

status:
  phase: Complete
  startTime: "2026-09-28T10:00:00Z"
  completionTime: "2026-09-28T10:12:30Z"
  duration: 12m30s
  
  # Restored resources
  restoredResources:
    - kind: StatefulSet
      name: restored-postgres
      namespace: database-restore-test
    - kind: PersistentVolumeClaim
      name: restored-postgres-data-0
      namespace: database-restore-test
  
  # Verification results
  verification:
    status: Passed
    checks:
      - type: connectivity
        status: Passed
      - type: data-integrity
        status: Passed
```

## Component Design

### 1. RPO Scheduler

**Responsibility**: Calculate backup cadence from RPO and create BackupJob CRs.

```rust
pub struct RPOScheduler {
    k8s_client: Client,
    metrics: SchedulerMetrics,
}

impl RPOScheduler {
    pub async fn reconcile_backup_plan(
        &self,
        plan: &BackupPlan,
    ) -> Result<()> {
        // Calculate backup cadence
        let cadence = self.calculate_cadence(&plan.spec.strategy.rpo);
        
        // Check if backup is due
        let last_backup_time = plan.status
            .as_ref()
            .and_then(|s| s.last_backup.as_ref())
            .map(|b| b.timestamp);
        
        let now = Utc::now();
        let next_backup_time = match last_backup_time {
            Some(last) => last + cadence,
            None => now, // First backup
        };
        
        if now >= next_backup_time {
            // Check if we're in a backup window
            if !self.is_in_backup_window(now, &plan.spec.schedule) {
                tracing::info!(
                    plan = plan.name(),
                    "Backup due but outside backup window"
                );
                return Ok(());
            }
            
            // Create BackupJob
            self.create_backup_job(plan).await?;
            
            self.metrics.backups_scheduled.with_label_values(&[
                plan.name(),
                &plan.spec.strategy.datastore_type,
            ]).inc();
        }
        
        Ok(())
    }
    
    fn calculate_cadence(&self, rpo: &Duration) -> Duration {
        // Backup cadence should be <= RPO / 2 to ensure RPO is met
        // even if one backup fails
        *rpo / 2
    }
    
    fn is_in_backup_window(
        &self,
        now: DateTime<Utc>,
        schedule: &Schedule,
    ) -> bool {
        // Check if current time is in a backup window
        for window in &schedule.windows {
            if self.time_in_window(now, window) {
                // Check not in blackout window
                for blackout in &schedule.blackout_windows {
                    if self.time_in_window(now, blackout) {
                        return false;
                    }
                }
                return true;
            }
        }
        false
    }
}
```

### 2. Backup Executor

**Responsibility**: Execute backup operations based on datastore type.

```rust
pub struct BackupExecutor {
    k8s_client: Client,
    storage_clients: HashMap<String, Box<dyn StorageBackend>>,
    metrics: ExecutorMetrics,
}

impl BackupExecutor {
    pub async fn execute_backup(
        &self,
        job: &BackupJob,
    ) -> Result<BackupResult> {
        let executor = self.get_datastore_executor(&job.spec.datastore_type)?;
        
        // Phase 1: Pre-backup (e.g., flush buffers, lock tables)
        executor.pre_backup().await?;
        
        // Phase 2: Create snapshot
        let snapshot_result = match executor.backup_method() {
            BackupMethod::VolumeSnapshot => {
                self.create_volume_snapshot(job).await?
            }
            BackupMethod::Native => {
                executor.execute_native_backup(job).await?
            }
            BackupMethod::Hybrid => {
                // Both volume snapshot and logical backup
                let snap = self.create_volume_snapshot(job).await?;
                let native = executor.execute_native_backup(job).await?;
                BackupResult::Hybrid { snap, native }
            }
        };
        
        // Phase 3: Capture PITR metadata
        let pitr_metadata = if job.spec.pitr.enabled {
            Some(executor.capture_pitr_metadata().await?)
        } else {
            None
        };
        
        // Phase 4: Upload to storage
        let storage = self.get_storage_backend(&job.spec.storage.provider)?;
        storage.upload_backup(&snapshot_result).await?;
        
        // Phase 5: Post-backup cleanup
        executor.post_backup().await?;
        
        Ok(BackupResult {
            snapshot: snapshot_result,
            pitr: pitr_metadata,
            size: snapshot_result.size,
            checksum: snapshot_result.checksum,
        })
    }
    
    async fn create_volume_snapshot(
        &self,
        job: &BackupJob,
    ) -> Result<SnapshotResult> {
        // Create VolumeSnapshot CR
        let snapshot = VolumeSnapshot {
            metadata: ObjectMeta {
                name: format!("{}-snapshot", job.name()),
                namespace: job.namespace().unwrap(),
                ..Default::default()
            },
            spec: VolumeSnapshotSpec {
                source: VolumeSnapshotSource::PersistentVolumeClaim(
                    job.spec.target.pvc_name.clone()
                ),
                volume_snapshot_class_name: Some("csi-snapshot-class".into()),
            },
        };
        
        let api: Api<VolumeSnapshot> = Api::namespaced(
            self.k8s_client.clone(),
            job.namespace().unwrap(),
        );
        
        api.create(&PostParams::default(), &snapshot).await?;
        
        // Wait for snapshot to be ready
        self.wait_for_snapshot_ready(&snapshot.name(), job.namespace().unwrap()).await?;
        
        Ok(SnapshotResult {
            snapshot_id: snapshot.name(),
            size: snapshot.status.ready_to_use_size,
            checksum: self.calculate_snapshot_checksum(&snapshot).await?,
        })
    }
}
```

### 3. PostgreSQL Backup Executor

```rust
pub struct PostgreSQLExecutor {
    k8s_client: Client,
}

#[async_trait]
impl DatastoreExecutor for PostgreSQLExecutor {
    fn backup_method(&self) -> BackupMethod {
        BackupMethod::Hybrid // Volume snapshot + WAL archiving
    }
    
    async fn pre_backup(&self) -> Result<()> {
        // Start backup mode
        self.exec_sql("SELECT pg_start_backup('stellar-backup', false, false);").await?;
        Ok(())
    }
    
    async fn post_backup(&self) -> Result<()> {
        // Stop backup mode
        self.exec_sql("SELECT pg_stop_backup(false);").await?;
        Ok(())
    }
    
    async fn capture_pitr_metadata(&self) -> Result<PITRMetadata> {
        // Get current WAL position
        let result = self.exec_sql("SELECT pg_current_wal_lsn();").await?;
        let lsn = result.rows[0][0].as_str();
        
        // Get latest WAL file
        let wal_file = self.exec_sql(
            "SELECT pg_walfile_name(pg_current_wal_lsn());"
        ).await?;
        
        Ok(PITRMetadata {
            lsn: lsn.to_string(),
            wal_file: wal_file.rows[0][0].to_string(),
            timestamp: Utc::now(),
        })
    }
    
    async fn execute_native_backup(&self, job: &BackupJob) -> Result<BackupResult> {
        // Execute pg_dump
        let pod_name = self.get_primary_pod(job).await?;
        
        let dump_command = format!(
            "pg_dump -U postgres -Fc -f /tmp/backup.dump"
        );
        
        self.exec_in_pod(&pod_name, &dump_command).await?;
        
        // Copy dump file to object storage
        let dump_data = self.copy_from_pod(&pod_name, "/tmp/backup.dump").await?;
        
        Ok(BackupResult {
            data: dump_data,
            size: dump_data.len(),
            checksum: sha256(&dump_data),
        })
    }
}
```

### 4. Restore Verifier

**Responsibility**: Automatically test restore after backup completion.

```rust
pub struct RestoreVerifier {
    k8s_client: Client,
    restore_executor: RestoreExecutor,
    metrics: VerifierMetrics,
}

impl RestoreVerifier {
    pub async fn verify_backup(
        &self,
        backup_job: &BackupJob,
        plan: &BackupPlan,
    ) -> Result<VerificationResult> {
        // Check if verification needed
        if !plan.spec.verification.enabled {
            return Ok(VerificationResult::Skipped);
        }
        
        let backup_count = self.get_backup_count(plan).await?;
        if backup_count % plan.spec.verification.frequency != 0 {
            return Ok(VerificationResult::Skipped);
        }
        
        // Create ephemeral namespace for test restore
        let test_namespace = format!("backup-verify-{}", backup_job.name());
        self.create_test_namespace(&test_namespace).await?;
        
        // Create RestoreRequest
        let restore_request = RestoreRequest {
            metadata: ObjectMeta {
                name: format!("verify-{}", backup_job.name()),
                namespace: test_namespace.clone(),
                ..Default::default()
            },
            spec: RestoreRequestSpec {
                source: RestoreSource::BackupJobRef {
                    name: backup_job.name(),
                },
                target: RestoreTarget {
                    namespace: test_namespace.clone(),
                    name_prefix: Some("test-".into()),
                },
                options: RestoreOptions {
                    pitr: false,
                    verify: true,
                },
            },
        };
        
        // Execute restore
        let restore_result = self.restore_executor
            .execute_restore(&restore_request)
            .await?;
        
        // Run verification checks
        let check_results = self.run_verification_checks(
            &plan.spec.verification.checks,
            &test_namespace,
        ).await?;
        
        // Cleanup test environment
        self.cleanup_test_namespace(&test_namespace).await?;
        
        // Mark backup as Complete if verification passed
        let passed = check_results.iter().all(|r| r.passed);
        if passed {
            self.mark_backup_complete(backup_job).await?;
            self.metrics.verifications_passed.inc();
        } else {
            self.metrics.verifications_failed.inc();
        }
        
        Ok(VerificationResult {
            passed,
            checks: check_results,
            duration: restore_result.duration,
        })
    }
    
    async fn run_verification_checks(
        &self,
        checks: &[VerificationCheck],
        namespace: &str,
    ) -> Result<Vec<CheckResult>> {
        let mut results = Vec::new();
        
        for check in checks {
            let result = match check.check_type {
                CheckType::Checksum => {
                    self.verify_checksum(check, namespace).await?
                }
                CheckType::Query => {
                    self.verify_query(check, namespace).await?
                }
                CheckType::Connectivity => {
                    self.verify_connectivity(namespace).await?
                }
            };
            results.push(result);
        }
        
        Ok(results)
    }
}
```

### 5. Cross-Region Replication Manager

```rust
pub struct ReplicationManager {
    storage_clients: HashMap<String, Box<dyn StorageBackend>>,
    metrics: ReplicationMetrics,
}

impl ReplicationManager {
    pub async fn replicate_backup(
        &self,
        backup_job: &BackupJob,
        replicas: &[StorageLocation],
    ) -> Result<Vec<ReplicationResult>> {
        let mut results = Vec::new();
        
        // Get source backup data
        let source_storage = self.get_storage_backend(
            &backup_job.spec.storage.provider
        )?;
        let backup_data = source_storage.download_backup(backup_job).await?;
        let source_checksum = sha256(&backup_data);
        
        // Replicate to each target region
        for replica in replicas {
            let target_storage = self.get_storage_backend(&replica.provider)?;
            
            let start = Instant::now();
            target_storage.upload_backup_data(&backup_data, replica).await?;
            let duration = start.elapsed();
            
            // Verify checksum
            let target_data = target_storage.download_backup_from(replica).await?;
            let target_checksum = sha256(&target_data);
            
            let verified = source_checksum == target_checksum;
            if !verified {
                tracing::error!(
                    backup = backup_job.name(),
                    region = replica.region,
                    "Checksum mismatch after replication"
                );
            }
            
            results.push(ReplicationResult {
                region: replica.region.clone(),
                duration,
                verified,
                checksum: target_checksum,
            });
            
            self.metrics.replications_total
                .with_label_values(&[&replica.region])
                .inc();
            
            if verified {
                self.metrics.replication_lag_seconds
                    .with_label_values(&[&replica.region])
                    .set(duration.as_secs() as f64);
            }
        }
        
        Ok(results)
    }
}
```

## Metrics

```prometheus
# Scheduling
backup_plans_total
backup_jobs_scheduled_total{plan, datastore_type}
backup_jobs_completed_total{plan, datastore_type, status="success|failed"}
backup_schedule_drift_seconds{plan}

# RPO Achievement
backup_rpo_target_seconds{plan}
backup_rpo_actual_seconds{plan}
backup_rpo_achievement_rate{plan}

# Backup execution
backup_duration_seconds{plan, datastore_type}
backup_size_bytes{plan, datastore_type}
backup_pitr_window_seconds{plan}

# Restore verification
backup_verifications_total{plan, result="passed|failed|skipped"}
backup_verification_duration_seconds{plan}

# Replication
backup_replications_total{region, status="success|failed"}
backup_replication_lag_seconds{region}
backup_checksum_mismatches_total{region}

# Retention
backups_retained_total{plan}
backups_deleted_total{plan, reason="retention|manual"}
```

## Testing Strategy

### Unit Tests
- RPO to cadence calculation
- Backup window determination
- PITR metadata capture
- Checksum calculation

### Integration Tests
- End-to-end backup and restore for each datastore
- PITR restore accuracy
- Cross-region replication
- Restore verification

### Acceptance Tests
- 30-day RPO achievement tracking
- 1000+ replication checksum verification
- 10 PITR drills with RTO measurement

## Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: backup-controller
  namespace: stellar-system
spec:
  replicas: 2  # HA with leader election
  selector:
    matchLabels:
      app: backup-controller
  template:
    metadata:
      labels:
        app: backup-controller
    spec:
      serviceAccountName: backup-controller
      containers:
      - name: controller
        image: stellar/backup-controller:v1.0.0
        args:
        - --leader-elect
        env:
        - name: RUST_LOG
          value: info
        resources:
          requests:
            cpu: 100m
            memory: 256Mi
          limits:
            cpu: 500m
            memory: 512Mi
```
