# Requirements: Declarative Backup Plans with PITR

## Overview
Implement a declarative backup strategy system using Kubernetes Custom Resources that automatically schedules backups based on RPO targets, supports point-in-time recovery (PITR), performs automated restore verification, and handles cross-region replication with retention tiers.

## Goals
- Declarative backup strategy via BackupPlan CRs
- Automatic PITR capability for supported datastores
- Mandatory restore verification as backup completion gate
- Cross-region disaster recovery with configurable retention

## Non-Goals
- Replace existing backup tools (velero, etcd backup, pg_dump)
- Implement custom backup storage formats
- Support non-Kubernetes workloads
- Provide backup encryption (delegated to storage layer)

## Functional Requirements

### FR1: BackupPlan Custom Resource
- **FR1.1**: Define BackupPlan CRD with RPO target specification
- **FR1.2**: Specify target resources (namespace, labels, or specific resources)
- **FR1.3**: Declare supported datastore types (PostgreSQL, MySQL, MongoDB, etcd)
- **FR1.4**: Configure retention policy (duration, count)
- **FR1.5**: Enable/disable PITR per backup plan
- **FR1.6**: Configure cross-region replication targets

### FR2: RPO-Based Scheduling
- **FR2.1**: Calculate backup cadence from declared RPO target
- **FR2.2**: Support RPO ranges: 5min to 24hr
- **FR2.3**: Adjust schedule dynamically if RPO target changes
- **FR2.4**: Account for backup duration in scheduling
- **FR2.5**: Provide RPO achievement metrics

### FR3: Point-in-Time Recovery
- **FR3.1**: Enable PITR for PostgreSQL using WAL archiving
- **FR3.2**: Enable PITR for MySQL using binary logs
- **FR3.3**: Enable PITR for MongoDB using oplog
- **FR3.4**: Store PITR metadata with backup snapshots
- **FR3.5**: Support PITR target specification (timestamp or transaction ID)
- **FR3.6**: Validate PITR window coverage

### FR4: Automated Backup Execution
- **FR4.1**: Create VolumeSnapshot for stateful workloads
- **FR4.2**: Execute datastore-native backup tools (pg_dump, mysqldump)
- **FR4.3**: Capture application-consistent snapshots
- **FR4.4**: Tag backups with metadata (timestamp, RPO, PITR window)
- **FR4.5**: Update BackupPlan status with last backup time

### FR5: Restore Verification
- **FR5.1**: Automatically trigger test restore after backup completes
- **FR5.2**: Create ephemeral namespace for test restore
- **FR5.3**: Restore data to test environment
- **FR5.4**: Execute verification queries/checks
- **FR5.5**: Compare checksums or row counts
- **FR5.6**: Mark backup as Complete only after verification passes
- **FR5.7**: Cleanup test environment after verification

### FR6: Cross-Region Replication
- **FR6.1**: Copy backup snapshots to configured regions
- **FR6.2**: Verify copy integrity with checksums
- **FR6.3**: Apply different retention policies per region
- **FR6.4**: Track replication lag per region
- **FR6.5**: Support multi-cloud replication (AWS, GCP, Azure)

### FR7: Backup Lifecycle Management
- **FR7.1**: Automatically delete backups exceeding retention policy
- **FR7.2**: Keep minimum number of backups regardless of retention
- **FR7.3**: Prevent deletion of backups required for PITR
- **FR7.4**: Support manual backup retention override
- **FR7.5**: Emit events for backup creation/deletion

### FR8: Restore Operations
- **FR8.1**: RestoreRequest CR for manual restore initiation
- **FR8.2**: Specify target backup by name or PITR timestamp
- **FR8.3**: Specify target namespace/resources for restore
- **FR8.4**: Track restore progress and status
- **FR8.5**: Validate restore RTO achievement

## Non-Functional Requirements

### NFR1: Performance
- **NFR1.1**: Backup initiation within 60 seconds of scheduled time
- **NFR1.2**: Restore verification completes within 2x backup duration
- **NFR1.3**: Cross-region copy within 4x backup size (MB) seconds
- **NFR1.4**: Support 100+ concurrent backup plans per cluster

### NFR2: Reliability
- **NFR2.1**: 99.9% backup success rate for scheduled backups
- **NFR2.2**: 100% restore verification execution (no skips)
- **NFR2.3**: Backup controller leader election for HA
- **NFR2.4**: Retry failed backups with exponential backoff
- **NFR2.5**: Alert on backup/restore failures

### NFR3: Storage
- **NFR3.1**: Support S3-compatible object storage
- **NFR3.2**: Support GCS and Azure Blob storage
- **NFR3.3**: Support in-cluster VolumeSnapshots
- **NFR3.4**: Efficient storage with incremental backups where possible

### NFR4: Security
- **NFR4.1**: RBAC for BackupPlan and RestoreRequest resources
- **NFR4.2**: Backup data encrypted at rest (via storage provider)
- **NFR4.3**: Credentials stored in Kubernetes Secrets
- **NFR4.4**: Audit log for backup/restore operations

### NFR5: Observability
- **NFR5.1**: Metrics for achieved RPO per backup plan
- **NFR5.2**: Metrics for backup success/failure rate
- **NFR5.3**: Metrics for restore verification pass/fail rate
- **NFR5.4**: Metrics for PITR window coverage
- **NFR5.5**: Metrics for cross-region replication lag

## Acceptance Criteria

### AC1: RPO Achievement
- Achieved RPO matches declared target for 30 consecutive days
- Measured by comparing actual backup intervals to declared RPO
- 95% of backups complete within RPO window

### AC2: Restore Verification
- Automated restore verification passes 100% of runs
- Zero backups marked Complete without passing verification
- Test restore environment properly cleaned up after each run

### AC3: PITR Capability
- PITR lands within the declared RTO in drills
- Conduct 10 PITR drills with random timestamps
- All drills complete within RTO target with data integrity verified

### AC4: Cross-Region Integrity
- Cross-region copy verified by checksum for 100% of backups
- Zero checksum mismatches in 1000+ replication events
- Replication lag within 2x of backup duration

## User Stories

### US1: Database Administrator
As a database administrator, I want to declare my backup RPO in a Kubernetes manifest so that the platform automatically handles scheduling and retention.

**Acceptance**: Create BackupPlan CR with RPO=1hr, system schedules hourly backups automatically.

### US2: Disaster Recovery Planner
As a DR planner, I want to restore to a specific point in time before a data corruption incident so that I can recover without data loss.

**Acceptance**: Issue RestoreRequest with PITR timestamp, system restores database to exact transaction state.

### US3: Platform Operator
As a platform operator, I want automatic verification that backups are restorable so that I don't discover restore failures during actual disasters.

**Acceptance**: Every backup triggers test restore, failures alert immediately.

### US4: Compliance Officer
As a compliance officer, I want backups replicated to a separate region so that we meet regulatory disaster recovery requirements.

**Acceptance**: BackupPlan specifies remote regions, checksums confirm integrity.

### US5: Application Developer
As an application developer, I want to test my application against restored data so that I can verify backup compatibility before incidents.

**Acceptance**: Restore to test namespace, run integration tests, teardown.

## Dependencies
- Kubernetes VolumeSnapshot API
- CSI snapshot drivers (AWS EBS, GCE PD, etc.)
- Datastore backup tools (pg_dump, mysqldump, mongodump)
- Object storage providers (S3, GCS, Azure Blob)
- Velero (optional, for Kubernetes resource backup)

## Open Questions
1. Should we support application-level backups (e.g., Kafka topic snapshots)?
2. What is the strategy for backing up CRDs and custom resources?
3. How do we handle backup encryption key rotation?
4. Should restore verification be configurable or always mandatory?
5. What is the maximum PITR window we need to support?
6. How do we handle multi-tenant backup isolation?
