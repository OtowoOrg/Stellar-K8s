//! Automated Database Compaction Daemon.
//!
//! Stellar Core and Horizon database sizes expand continuously, degrading
//! disk I/O over time. This daemon coordinates scheduled database vacuums
//! (compaction) and ledger pruning across nodes without causing downtime.
//!
//! # Lifecycle of a compaction cycle
//!
//! 1. **Schedule** — a cron expression (or the `windowStart`/`windowDuration`
//!    fallback) decides when a node is due for maintenance.
//! 2. **Evaluate** — [`db::evaluate_fragmentation`] measures dead-tuple
//!    ratios; nothing runs when fragmentation is below the threshold.
//! 3. **Drain** — [`super::coordinator::MaintenanceCoordinator`] removes the
//!    node from the Service's endpoints so no new API traffic is routed to it.
//! 4. **Compact** — `VACUUM (FULL)` plus `REINDEX` on bloated tables, with
//!    `pg_repack` for extreme bloat. Optional ledger pruning runs too.
//! 5. **Verify** — [`db::DatabaseIntegrityVerifier`] recomputes table
//!    checksums and compares them to the pre-compaction snapshot. The node is
//!    only returned to rotation **after** integrity is confirmed.
//! 6. **Rejoin** — traffic is restored and the maintenance marker is cleared.
//!
//! # Quorum safety
//!
//! Compaction is serialized across **primary validator nodes**: at most one
//! validator compaction runs at any moment (in-process mutex plus a
//! `stellar.org/compaction-in-progress` annotation on the `StellarNode` for
//! cross-replica visibility). Horizon / Soroban RPC nodes may compact
//! concurrently.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveTime, Utc};
use cron::Schedule;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::runtime::events::{Event as K8sEvent, EventType, Recorder, Reporter};
use kube::{Client, Resource, ResourceExt};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::controller::background_jobs::{JobHandle, JobKind, JobRegistry};
use crate::crd::{DbMaintenanceConfig, NodeType, StellarNode};
use crate::error::{Error, Result};

use super::bloat::BloatDetector;
use super::coordinator::MaintenanceCoordinator;
use super::db::{
    self, DatabaseIntegrityVerifier, FragmentationMetrics, IntegrityReport, LedgerPruner,
};

/// Annotation marking a node that is currently being compacted. Used for
/// cross-replica coordination so a primary validator is never compacted twice
/// at the same time.
pub const COMPACTION_MARKER_ANNOTATION: &str = "stellar.org/compaction-in-progress";

/// Only tables at least this large (64 MiB) are considered for compaction,
/// so small tables with noisy dead-tuple statistics don't trigger VACUUM FULL.
pub const MIN_TABLE_SIZE_BYTES: i64 = 64 * 1024 * 1024;

/// Bloat percentage above which `pg_repack` is attempted in addition to
/// VACUUM FULL (the extension may not be installed).
pub const REPACK_BLOAT_THRESHOLD_PCT: f64 = 60.0;

/// Interval between daemon sweeps when checking for due maintenance.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Scheduling helpers
// ---------------------------------------------------------------------------

/// Parse a simple duration string such as `"2h"`, `"90m"` or `"3600s"`.
pub fn parse_duration_str(s: &str) -> Option<ChronoDuration> {
    let s = s.trim();
    if let Some(h) = s.strip_suffix('h') {
        h.parse::<i64>().ok().map(ChronoDuration::hours)
    } else if let Some(m) = s.strip_suffix('m') {
        m.parse::<i64>().ok().map(ChronoDuration::minutes)
    } else if let Some(sec) = s.strip_suffix('s') {
        sec.parse::<i64>().ok().map(ChronoDuration::seconds)
    } else {
        None
    }
}

/// Resolve the effective window from a maintenance config.
///
/// Returns `(start_time, duration)` with sensible defaults (`02:00`, 2h).
pub fn window_config(config: &DbMaintenanceConfig) -> (NaiveTime, ChronoDuration) {
    let start = NaiveTime::parse_from_str(&config.window_start, "%H:%M")
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(2, 0, 0).unwrap());
    let duration = parse_duration_str(&config.window_duration).unwrap_or_else(|| {
        warn!(
            "Unparsable window_duration {:?} for maintenance; defaulting to 2h",
            config.window_duration
        );
        ChronoDuration::hours(2)
    });
    (start, duration)
}

/// True when the local time is inside `[start, start + duration)`, including
/// windows that wrap past midnight.
pub fn is_in_window(start: NaiveTime, duration: ChronoDuration) -> bool {
    let now = Local::now().naive_local();
    let today = now.date();

    // Today's window.
    let start_today = today.and_time(start);
    let end_today = start_today + duration;
    if now >= start_today && now < end_today {
        return true;
    }

    // Yesterday's window — covers windows that wrap past midnight
    // (e.g. start 23:00 with a 2h duration ends at 01:00 the next day).
    let start_yesterday = (today - chrono::Days::new(1)).and_time(start);
    let end_yesterday = start_yesterday + duration;
    now >= start_yesterday && now < end_yesterday
}

/// True when a cron schedule is due at `now` given the previous run time.
///
/// A `None` last run (never executed) is always due, mirroring the pruning
/// worker's semantics.
pub fn cron_schedule_due(
    schedule_str: &str,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    let Ok(schedule) = Schedule::from_str(schedule_str) else {
        warn!("Invalid cron expression: {schedule_str}");
        return false;
    };
    match last_run {
        None => true,
        Some(last) => schedule.after(&last).next().is_some_and(|next| next <= now),
    }
}

// ---------------------------------------------------------------------------
// Quorum-safe coordination
// ---------------------------------------------------------------------------

/// Serializes compaction across primary validator nodes.
///
/// Validator compaction is guarded by a process-wide mutex, so at most one
/// validator compacts at a time. Horizon / Soroban RPC nodes acquire an
/// uncontended guard and may compact concurrently.
pub struct CompactionCoordinator {
    validator_lock: Mutex<()>,
}

impl Default for CompactionCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactionCoordinator {
    pub fn new() -> Self {
        Self {
            validator_lock: Mutex::new(()),
        }
    }

    /// Try to acquire the compaction right for `node`.
    ///
    /// Returns `None` when another validator is currently being compacted.
    pub async fn try_acquire(&self, node: &StellarNode) -> Result<Option<CompactionGuard<'_>>> {
        if node.spec.node_type == NodeType::Validator {
            match self.validator_lock.try_lock() {
                Ok(guard) => Ok(Some(CompactionGuard {
                    _validator: Some(guard),
                })),
                Err(_) => {
                    debug!(
                        "Compaction for validator {} skipped: another validator compaction is in progress",
                        node.name_any()
                    );
                    Ok(None)
                }
            }
        } else {
            Ok(Some(CompactionGuard { _validator: None }))
        }
    }
}

/// RAII guard returned by [`CompactionCoordinator::try_acquire`]. Holding the
/// guard keeps validator compaction serialized for its lifetime.
pub struct CompactionGuard<'a> {
    _validator: Option<tokio::sync::MutexGuard<'a, ()>>,
}

// ---------------------------------------------------------------------------
// Compaction daemon
// ---------------------------------------------------------------------------

/// Result of a single compaction cycle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactionReport {
    /// Node name.
    pub node: String,
    /// Tables that were compacted (`VACUUM FULL`).
    pub tables_compacted: Vec<String>,
    /// Total relation size before compaction (bytes).
    pub bytes_before: u64,
    /// Total relation size after compaction (bytes).
    pub bytes_after: u64,
    /// Bytes freed (negative means the store grew).
    pub bytes_freed: i64,
    /// Post-compaction checksum verification outcome.
    pub integrity_valid: bool,
    /// Ledgers pruned from `history_ledgers`.
    pub ledgers_pruned: u64,
    /// True when API traffic was drained and restored.
    pub traffic_drained: bool,
    /// When set, the cycle did no work (e.g. no fragmentation, not due).
    pub skipped_reason: Option<String>,
}

impl CompactionReport {
    pub fn skipped(node: &str, reason: impl Into<String>) -> Self {
        Self {
            node: node.to_string(),
            skipped_reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

/// Cron-triggered compaction daemon.
///
/// Spawned in the background by the controller; sweeps all `StellarNode`s
/// periodically and runs a compaction cycle for each node that is due.
pub struct CompactionDaemon {
    client: Client,
    reporter: Reporter,
    coordinator: Arc<CompactionCoordinator>,
    /// Optional database pool. When `None`, the daemon runs in dry mode and
    /// skips all database work (useful when the operator has no `DATABASE_URL`).
    pool: Option<PgPool>,
    /// Namespace restriction, mirroring the reconciler's watch namespace.
    watch_namespace: Option<String>,
    /// Optional job registry for dashboard visibility.
    job_registry: Option<Arc<JobRegistry>>,
    /// In-memory last-run timestamps per node (for cron scheduling).
    last_run: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Poll interval between sweeps.
    poll_interval: Duration,
}

impl CompactionDaemon {
    pub fn new(
        client: Client,
        reporter: Reporter,
        pool: Option<PgPool>,
        watch_namespace: Option<String>,
        job_registry: Option<Arc<JobRegistry>>,
    ) -> Self {
        Self {
            client,
            reporter,
            coordinator: Arc::new(CompactionCoordinator::new()),
            pool,
            watch_namespace,
            job_registry,
            last_run: Mutex::new(HashMap::new()),
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }

    /// Set the sweep interval (mainly for tests).
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Run the daemon loop until the task is cancelled.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("Starting DB Compaction Daemon");
        if self.pool.is_none() {
            warn!(
                "No DATABASE_URL configured; compaction daemon is in dry mode \
                 (schedules are evaluated but no database work is performed)"
            );
        }

        loop {
            if let Err(e) = self.run_sweep().await {
                error!("Compaction daemon sweep failed: {e}");
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }

    /// Evaluate every `StellarNode` and run compaction for due nodes.
    pub async fn run_sweep(&self) -> Result<()> {
        let api: Api<StellarNode> = match &self.watch_namespace {
            Some(ns) => Api::namespaced(self.client.clone(), ns),
            None => Api::all(self.client.clone()),
        };

        let nodes = api.list(&ListParams::default()).await?;
        for node in nodes.items {
            if let Err(e) = self.maybe_compact(&node).await {
                warn!("Compaction cycle failed for node {}: {e}", node.name_any());
            }
        }
        Ok(())
    }

    /// Run a compaction cycle for `node` when it is due and enabled.
    pub async fn maybe_compact(&self, node: &StellarNode) -> Result<()> {
        let name = node.name_any();

        let Some(config) = &node.spec.db_maintenance_config else {
            return Ok(());
        };
        if !config.enabled {
            return Ok(());
        }
        if node.spec.suspended || node.spec.maintenance_mode {
            debug!("Skipping compaction for {name}: node suspended or in maintenance mode");
            return Ok(());
        }

        // Cross-replica coordination: another instance may already be
        // compacting this node.
        if node
            .annotations()
            .get(COMPACTION_MARKER_ANNOTATION)
            .is_some_and(|v| v == "true")
        {
            debug!("Skipping compaction for {name}: marker already set");
            return Ok(());
        }

        if !self.is_due(node).await {
            return Ok(());
        }

        // Record this run regardless of outcome so cron schedules advance.
        let mut last_run = self.last_run.lock().await;
        last_run.insert(name.clone(), Utc::now());

        let Some(pool) = &self.pool else {
            info!("Compaction due for {name} but no database pool available (dry mode)");
            return Ok(());
        };

        let _job = self.register_job(&name, node.namespace().as_deref());
        let report = match run_compaction_cycle(
            &self.client,
            Some(&self.reporter),
            &self.coordinator,
            node,
            pool,
        )
        .await
        {
            Ok(report) => report,
            Err(e) => {
                // A failed cycle must never leave the marker set, otherwise
                // every future sweep would skip this node forever.
                if let Err(clear_err) = set_compaction_marker(&self.client, node, false).await {
                    warn!("Failed to clear compaction marker on {name} after error: {clear_err}");
                }
                return Err(e);
            }
        };

        if let Some(skipped) = &report.skipped_reason {
            debug!("Compaction skipped for {name}: {skipped}");
        } else {
            info!(
                "Compaction complete for {name}: {} tables compacted, {} bytes freed, integrity={}, ledgers pruned={}",
                report.tables_compacted.len(),
                report.bytes_freed,
                report.integrity_valid,
                report.ledgers_pruned
            );
        }
        Ok(())
    }

    /// Cron/window scheduling check for a node.
    async fn is_due(&self, node: &StellarNode) -> bool {
        let Some(config) = &node.spec.db_maintenance_config else {
            return false;
        };
        if let Some(schedule) = &config.schedule {
            let last = self.last_run.lock().await.get(&node.name_any()).copied();
            return cron_schedule_due(schedule, last, Utc::now());
        }
        let (start, duration) = window_config(config);
        is_in_window(start, duration)
    }

    fn register_job(&self, node_name: &str, namespace: Option<&str>) -> Option<JobHandle> {
        let registry = self.job_registry.as_ref()?;
        Some(registry.register(
            format!("db-compaction/{node_name}"),
            JobKind::MaintenanceWindow,
            namespace.map(str::to_string),
        ))
    }
}

// ---------------------------------------------------------------------------
// Compaction cycle
// ---------------------------------------------------------------------------

/// Execute a single compaction cycle for `node`.
///
/// The caller is responsible for schedule evaluation; this function runs the
/// full drain → compact → verify → rejoin lifecycle.
pub async fn run_compaction_cycle(
    client: &Client,
    reporter: Option<&Reporter>,
    coordinator: &CompactionCoordinator,
    node: &StellarNode,
    pool: &PgPool,
) -> Result<CompactionReport> {
    let name = node.name_any();
    let config = match &node.spec.db_maintenance_config {
        Some(c) if c.enabled => c,
        _ => {
            return Ok(CompactionReport::skipped(&name, "maintenance not enabled"));
        }
    };

    let detector = BloatDetector::new(pool.clone());
    let mut report = CompactionReport {
        node: name.clone(),
        ..Default::default()
    };

    // 1. Quiet check — never compact while ledgers are being written.
    if !detector.is_system_quiet().await? {
        return Ok(CompactionReport::skipped(
            &name,
            "active ledger writes detected",
        ));
    }

    // 2. Fragmentation evaluation.
    let metrics =
        db::evaluate_fragmentation(pool, config.bloat_threshold_percent, MIN_TABLE_SIZE_BYTES)
            .await?;
    let fragmented: Vec<&FragmentationMetrics> = metrics.iter().filter(|m| m.fragmented).collect();

    let should_prune = config.enable_ledger_pruning;
    if fragmented.is_empty() && !should_prune {
        return Ok(CompactionReport::skipped(
            &name,
            "no fragmented tables above threshold",
        ));
    }

    // 3. Quorum-safe acquisition (validators serialize).
    let Some(_guard) = coordinator.try_acquire(node).await? else {
        return Ok(CompactionReport::skipped(
            &name,
            "another primary validator compaction in progress",
        ));
    };

    // Cross-replica marker — best effort; failures are logged, not fatal.
    if let Err(e) = set_compaction_marker(client, node, true).await {
        warn!("Failed to set compaction marker on {name}: {e}");
    }

    publish_event(
        client,
        reporter,
        node,
        EventType::Normal,
        "CompactionStarting",
        "Compacting",
        &format!(
            "Starting DB compaction: {} fragmented table(s), ledger pruning={should_prune}",
            fragmented.len()
        ),
    )
    .await?;

    // 4. Drain API traffic before touching the database.
    let mut traffic_drained = false;
    if config.read_pool_coordination {
        let coordinator = MaintenanceCoordinator::new(client.clone());
        coordinator.prepare_node(node).await?;
        traffic_drained = true;
        info!("Traffic drained for node {name}");
    }

    // 5. Pre-compaction snapshot: checksums and sizes.
    let tables: Vec<String> = fragmented
        .iter()
        .map(|m| m.qualified_name.clone())
        .collect();
    let verifier = DatabaseIntegrityVerifier::new(pool.clone());
    let before_checksums = verifier.compute_checksums(&tables).await?;
    let bytes_before = db::total_relation_size(pool, &tables).await?;
    report.bytes_before = bytes_before;

    // 6. Compaction routines.
    for metrics in &fragmented {
        compact_table(pool, metrics, config.auto_reindex).await?;
        report.tables_compacted.push(metrics.qualified_name.clone());
    }

    // 7. Optional ledger pruning.
    if should_prune {
        let pruner = LedgerPruner::new(pool.clone(), config.pruning_retention_days);
        let prune_report = pruner.prune_ledgers().await?;
        report.ledgers_pruned = prune_report.ledgers_deleted;
        info!(
            "Ledger pruning for {name}: boundary={}, ledgers deleted={}, note={:?}",
            prune_report.boundary_sequence, prune_report.ledgers_deleted, prune_report.note
        );
    }

    // 8. Post-compaction integrity verification. The node stays drained until
    //    the checksums prove the data is intact.
    let after_checksums = verifier.compute_checksums(&tables).await?;
    let integrity = verifier.verify(&before_checksums, &after_checksums);
    report.integrity_valid = integrity.valid;
    report.bytes_after = db::total_relation_size(pool, &tables).await?;
    // Freed bytes are positive when the store shrank.
    report.bytes_freed = report.bytes_before as i64 - report.bytes_after as i64;

    if !integrity.valid {
        let message = format_integrity_message(&integrity);
        error!("Compaction integrity verification failed for {name}: {message}");
        publish_event(
            client,
            reporter,
            node,
            EventType::Warning,
            "CompactionIntegrityFailed",
            "Verify",
            &message,
        )
        .await?;
        // Leave the node drained so it never serves corrupted data; clear the
        // marker so the next sweep can retry.
        let _ = set_compaction_marker(client, node, false).await;
        return Err(Error::MaintenanceError(message));
    }

    // 9. Rejoin traffic only after verification passes.
    if traffic_drained {
        let coordinator = MaintenanceCoordinator::new(client.clone());
        coordinator.finalize_maintenance(node).await?;
        info!("Traffic restored for node {name}");
    }

    let _ = set_compaction_marker(client, node, false).await;

    publish_event(
        client,
        reporter,
        node,
        EventType::Normal,
        "CompactionSucceeded",
        "Compacting",
        &format!(
            "DB compaction succeeded: {} tables, {} bytes freed, integrity verified, {} ledgers pruned",
            report.tables_compacted.len(),
            report.bytes_freed,
            report.ledgers_pruned
        ),
    )
    .await?;

    Ok(report)
}

/// Run the actual compaction routines for one table: `VACUUM (FULL, ANALYZE)`,
/// optional `REINDEX`, and `pg_repack` for extreme bloat.
async fn compact_table(
    pool: &PgPool,
    metrics: &FragmentationMetrics,
    auto_reindex: bool,
) -> Result<()> {
    let qualified = db::quote_qualified_ident(&metrics.qualified_name)?;
    info!(
        "VACUUM FULL on {} (dead ratio {:.1}%, size {} MiB)",
        metrics.qualified_name,
        metrics.dead_ratio_pct,
        metrics.size_mib()
    );
    sqlx::query(&format!("VACUUM (FULL, ANALYZE) {qualified}"))
        .execute(pool)
        .await
        .map_err(Error::SqlxError)?;

    if auto_reindex {
        info!("REINDEX TABLE {}", metrics.qualified_name);
        sqlx::query(&format!("REINDEX TABLE {qualified}"))
            .execute(pool)
            .await
            .map_err(Error::SqlxError)?;
    }

    if metrics.dead_ratio_pct > REPACK_BLOAT_THRESHOLD_PCT {
        info!(
            "High bloat ({:.1}%) detected on {}, attempting pg_repack",
            metrics.dead_ratio_pct, metrics.qualified_name
        );
        if let Err(e) = sqlx::query("SELECT pg_repack.repack_table($1)")
            .bind(&metrics.qualified_name)
            .execute(pool)
            .await
        {
            // pg_repack is optional; without the extension installed this is
            // expected and VACUUM FULL already handled the bloat.
            warn!(
                "pg_repack failed for {} (ensure extension is installed): {e}",
                metrics.qualified_name
            );
        }
    }

    Ok(())
}

fn format_integrity_message(report: &IntegrityReport) -> String {
    if report.mismatches.is_empty() {
        return format!(
            "integrity verification passed for {} tables",
            report.tables_verified
        );
    }
    let mut msg = format!(
        "integrity verification failed for {} table(s):",
        report.mismatches.len()
    );
    for m in &report.mismatches {
        msg.push_str("\n- ");
        msg.push_str(m);
    }
    msg
}

/// Set or clear the `stellar.org/compaction-in-progress` annotation.
async fn set_compaction_marker(client: &Client, node: &StellarNode, active: bool) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    let name = node.name_any();
    let annotation_value = if active {
        json!("true")
    } else {
        // JSON merge patch: null removes the key.
        json!(null)
    };
    let patch = json!({
        "metadata": {
            "annotations": {
                COMPACTION_MARKER_ANNOTATION: annotation_value
            }
        }
    });
    api.patch(&name, &PatchParams::default(), &Patch::Merge(patch))
        .await?;
    Ok(())
}

/// Publish a Kubernetes event attached to the StellarNode (best effort when no
/// reporter is available).
async fn publish_event(
    client: &Client,
    reporter: Option<&Reporter>,
    node: &StellarNode,
    type_: EventType,
    reason: &str,
    action: &str,
    note: &str,
) -> Result<()> {
    let Some(reporter) = reporter else {
        return Ok(());
    };
    let recorder = Recorder::new(client.clone(), reporter.clone(), node.object_ref(&()));
    recorder
        .publish(K8sEvent {
            type_,
            reason: reason.to_string(),
            action: action.to_string(),
            note: Some(note.to_string()),
            secondary: None,
        })
        .await
        .map_err(Error::KubeError)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::StellarNodeSpec;

    fn spec_with_maintenance() -> StellarNodeSpec {
        StellarNodeSpec {
            db_maintenance_config: Some(DbMaintenanceConfig {
                enabled: true,
                window_start: "02:00".to_string(),
                window_duration: "2h".to_string(),
                schedule: None,
                bloat_threshold_percent: 30,
                auto_reindex: true,
                read_pool_coordination: true,
                enable_ledger_pruning: false,
                pruning_retention_days: 30,
            }),
            ..Default::default()
        }
    }

    fn node_with_type(node_type: NodeType) -> StellarNode {
        let spec = StellarNodeSpec {
            node_type,
            ..spec_with_maintenance()
        };
        crate::crd::StellarNode::new("test-node", spec)
    }

    #[test]
    fn test_parse_duration_str() {
        assert_eq!(parse_duration_str("2h"), Some(ChronoDuration::hours(2)));
        assert_eq!(parse_duration_str("90m"), Some(ChronoDuration::minutes(90)));
        assert_eq!(
            parse_duration_str("3600s"),
            Some(ChronoDuration::seconds(3600))
        );
        assert_eq!(parse_duration_str("nonsense"), None);
        assert_eq!(parse_duration_str(""), None);
    }

    #[test]
    fn test_window_config_defaults() {
        let config = DbMaintenanceConfig {
            enabled: true,
            window_start: "03:30".to_string(),
            window_duration: "90m".to_string(),
            schedule: None,
            bloat_threshold_percent: 30,
            auto_reindex: true,
            read_pool_coordination: true,
            enable_ledger_pruning: false,
            pruning_retention_days: 30,
        };
        let (start, duration) = window_config(&config);
        assert_eq!(start, NaiveTime::from_hms_opt(3, 30, 0).unwrap());
        assert_eq!(duration, ChronoDuration::minutes(90));
    }

    #[test]
    fn test_cron_schedule_due() {
        // cron crate uses 6 fields: seconds minutes hours day month weekday.
        let schedule = "0 0 2 * * *"; // daily at 02:00:00 UTC
        let now = DateTime::parse_from_rfc3339("2026-08-30T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Never run → due.
        assert!(cron_schedule_due(schedule, None, now));

        // Ran today at 01:00 UTC → next run (02:00) has passed → due.
        let last = DateTime::parse_from_rfc3339("2026-08-30T01:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(cron_schedule_due(schedule, Some(last), now));

        // Ran after the scheduled time → not due until tomorrow 02:00.
        let last = DateTime::parse_from_rfc3339("2026-08-30T03:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!cron_schedule_due(schedule, Some(last), now));

        // Invalid schedule → never due.
        assert!(!cron_schedule_due("not a cron", Some(now), now));
    }

    #[test]
    fn test_invalid_cron_never_due() {
        assert!(!cron_schedule_due("bogus", None, Utc::now()));
    }

    #[test]
    fn test_validator_compaction_serialized() {
        let coordinator = CompactionCoordinator::new();
        let validator_a = node_with_type(NodeType::Validator);
        let validator_b = node_with_type(NodeType::Validator);
        let horizon = node_with_type(NodeType::Horizon);

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // First validator acquires the lock.
            let guard = coordinator.try_acquire(&validator_a).await.unwrap();
            assert!(guard.is_some());

            // Second validator must wait — no concurrent compaction.
            let second = coordinator.try_acquire(&validator_b).await.unwrap();
            assert!(second.is_none());

            // Horizon nodes are not serialized.
            let horizon_guard = coordinator.try_acquire(&horizon).await.unwrap();
            assert!(horizon_guard.is_some());

            drop(guard);

            // After release, a validator can acquire again.
            let retry = coordinator.try_acquire(&validator_b).await.unwrap();
            assert!(retry.is_some());
        });
    }

    #[test]
    fn test_skipped_report_helper() {
        let report = CompactionReport::skipped("n1", "no work");
        assert_eq!(report.node, "n1");
        assert_eq!(report.skipped_reason.as_deref(), Some("no work"));
        assert!(report.tables_compacted.is_empty());
    }
}
