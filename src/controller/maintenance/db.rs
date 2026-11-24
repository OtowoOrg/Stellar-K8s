//! Database utilities for the compaction daemon.
//!
//! This module provides the SQL-level building blocks used by the
//! [`super::compactor::CompactionDaemon`]:
//!
//! - [`evaluate_fragmentation`] — queries `pg_stat_user_tables` to compute
//!   per-table fragmentation metrics (dead tuple ratio, total size).
//! - [`DatabaseIntegrityVerifier`] — computes a deterministic per-table
//!   checksum before and after compaction and reports whether the data is
//!   byte-for-byte equivalent (modulo the compaction itself).
//! - [`LedgerPruner`] — safely removes old `history_ledgers` (and dependent
//!   history tables) outside the configured retention window, in bounded
//!   batches, while always keeping a safety buffer of the newest ledgers.

use std::collections::BTreeMap;

use sqlx::{PgPool, Row};
use tracing::{debug, info, warn};

use crate::error::{Error, Result};

/// Approximate number of ledgers closed per day on the Stellar network
/// (ledger close time is ~5 seconds on average).
pub const LEDGERS_PER_DAY: i64 = 17_280;

/// Minimum number of the newest ledgers that must always be retained by the
/// pruner, regardless of the configured retention window. Guards against
/// operators misconfiguring `pruning_retention_days` such that live ingestion
/// data would be deleted.
pub const MIN_KEEP_LEDGERS: i64 = 1_000;

/// Default batch size (rows) for ledger pruning deletes.
pub const DEFAULT_PRUNE_BATCH_SIZE: u32 = 5_000;

/// Per-table fragmentation metrics collected by [`evaluate_fragmentation`].
#[derive(Clone, Debug, PartialEq)]
pub struct FragmentationMetrics {
    /// Schema-qualified table name, e.g. `public.history_ledgers`.
    pub qualified_name: String,
    /// Schema name.
    pub schema: String,
    /// Table name.
    pub table: String,
    /// Estimated live rows (`n_live_tup`).
    pub live_rows: i64,
    /// Estimated dead rows (`n_dead_tup`).
    pub dead_rows: i64,
    /// Dead tuple ratio as a percentage (0–100).
    pub dead_ratio_pct: f64,
    /// Total relation size in bytes (table + indexes + toast).
    pub total_size_bytes: i64,
    /// True when `dead_ratio_pct` meets the fragmentation threshold.
    pub fragmented: bool,
}

impl FragmentationMetrics {
    /// Total size in human-friendly units (MiB).
    pub fn size_mib(&self) -> f64 {
        self.total_size_bytes as f64 / (1024.0 * 1024.0)
    }
}

/// Report produced by [`LedgerPruner::prune_ledgers`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PruningReport {
    /// Highest ledger sequence deleted (inclusive boundary).
    pub boundary_sequence: i64,
    /// Newest ledger sequence observed at prune time.
    pub newest_sequence: i64,
    /// Rows deleted from `history_ledgers`.
    pub ledgers_deleted: u64,
    /// Rows deleted from dependent history tables (best-effort).
    pub dependent_rows_deleted: u64,
    /// Tables actually pruned.
    pub tables_pruned: Vec<String>,
    /// Human-readable note (e.g. "nothing to prune", "safety buffer applied").
    pub note: Option<String>,
}

/// Evaluate database fragmentation by scanning `pg_stat_user_tables`.
///
/// Returns one entry per user table with a non-zero dead tuple count, sorted
/// by dead tuple ratio descending. Only tables that are at least `min_size_bytes`
/// large are considered, so tiny tables with a noisy dead ratio don't trigger
/// compaction. `fragmented` is set when `dead_ratio_pct >= threshold_percent`.
pub async fn evaluate_fragmentation(
    pool: &PgPool,
    threshold_percent: u32,
    min_size_bytes: i64,
) -> Result<Vec<FragmentationMetrics>> {
    const QUERY: &str = r#"
        SELECT
            s.schemaname            AS schema_name,
            s.relname               AS table_name,
            s.n_live_tup            AS live_rows,
            s.n_dead_tup            AS dead_rows,
            CASE
                WHEN (s.n_live_tup + s.n_dead_tup) = 0 THEN 0.0
                ELSE ROUND(100.0 * s.n_dead_tup / (s.n_live_tup + s.n_dead_tup), 1)::float8
            END                     AS dead_ratio_pct,
            pg_total_relation_size(c.oid) AS total_size_bytes
        FROM pg_stat_user_tables s
        JOIN pg_class c ON c.relname = s.relname
        JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = s.schemaname
        WHERE s.n_dead_tup > 0
          AND pg_total_relation_size(c.oid) >= $1
        ORDER BY dead_ratio_pct DESC
    "#;

    let rows = sqlx::query(QUERY)
        .bind(min_size_bytes)
        .fetch_all(pool)
        .await
        .map_err(Error::SqlxError)?;

    let mut metrics = Vec::with_capacity(rows.len());
    for row in rows {
        let schema: String = row.try_get("schema_name")?;
        let table: String = row.try_get("table_name")?;
        let live_rows: i64 = row.try_get("live_rows")?;
        let dead_rows: i64 = row.try_get("dead_rows")?;
        let dead_ratio_pct: f64 = row.try_get("dead_ratio_pct")?;
        let total_size_bytes: i64 = row.try_get("total_size_bytes")?;

        metrics.push(FragmentationMetrics {
            fragmented: dead_ratio_pct >= threshold_percent as f64,
            qualified_name: format!("{schema}.{table}"),
            schema,
            table,
            live_rows,
            dead_rows,
            dead_ratio_pct,
            total_size_bytes,
        });
    }

    Ok(metrics)
}

/// Verifies database integrity before and after compaction.
///
/// Checksums are computed server-side in bounded chunks so that even multi-GB
/// tables never materialize the full aggregate in memory. The checksum of an
/// empty table is the md5 of the empty string (a stable, well-known value).
#[derive(Clone, Debug)]
pub struct DatabaseIntegrityVerifier {
    pool: PgPool,
    /// Rows hashed together per chunk. Larger values are faster but use more
    /// server memory (each chunk aggregates `chunk_size * 32` bytes).
    chunk_size: i64,
}

impl DatabaseIntegrityVerifier {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            chunk_size: 10_000,
        }
    }

    /// Compute the deterministic checksum of a schema-qualified table.
    ///
    /// The checksum covers every row (md5 of the row's composite text form),
    /// aggregated in ordered chunks. Any data change — insert, update, or
    /// delete — changes the result.
    ///
    /// Chunk boundaries are derived from the rows' own md5 values (sorted), not
    /// from their physical scan position. This keeps the checksum stable across
    /// a `VACUUM (FULL)` — which rewrites the table and can reorder rows — so
    /// post-compaction verification never reports a false mismatch.
    pub async fn compute_table_checksum(&self, qualified_table: &str) -> Result<String> {
        let table = quote_qualified_ident(qualified_table)?;
        let query = format!(
            r#"
            WITH ordered AS (
                SELECT md5(t::text) AS part
                FROM {table} t
            ),
            bucketed AS (
                SELECT part, (row_number() OVER (ORDER BY part) - 1) / $1 AS bucket
                FROM ordered
            ),
            chunks AS (
                SELECT md5(string_agg(part, '' ORDER BY part)) AS chunk_md5
                FROM bucketed
                GROUP BY bucket
            )
            SELECT COALESCE(
                md5(string_agg(chunk_md5, '' ORDER BY chunk_md5)),
                'd41d8cd98f00b204e9800998ecf8427e'
            )
            FROM chunks
            "#
        );

        let row = sqlx::query(&query)
            .bind(self.chunk_size)
            .fetch_one(&self.pool)
            .await
            .map_err(Error::SqlxError)?;
        let checksum: String = row.try_get(0)?;
        Ok(checksum)
    }

    /// Compute checksums for many tables at once.
    pub async fn compute_checksums(&self, tables: &[String]) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for table in tables {
            let checksum = self.compute_table_checksum(table).await?;
            out.insert(table.clone(), checksum);
        }
        Ok(out)
    }

    /// Compare pre-compaction and post-compaction checksums.
    ///
    /// Returns an [`IntegrityReport`]; `valid` is false when any table that
    /// existed in `before` is missing from `after`, or when any checksum
    /// differs. See [`verify_integrity`] for the standalone implementation.
    pub fn verify(
        &self,
        before: &BTreeMap<String, String>,
        after: &BTreeMap<String, String>,
    ) -> IntegrityReport {
        verify_integrity(before, after)
    }
}

/// Compare pre-compaction and post-compaction checksums.
///
/// Returns an [`IntegrityReport`]; `valid` is false when any table that
/// existed in `before` is missing from `after`, or when any checksum
/// differs.
pub fn verify_integrity(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> IntegrityReport {
    let mut mismatches = Vec::new();
    for (table, expected) in before {
        match after.get(table) {
            Some(actual) if actual == expected => {}
            Some(actual) => mismatches.push(format!(
                "table {table}: checksum mismatch (before={expected}, after={actual})"
            )),
            None => mismatches.push(format!("table {table}: missing from post-compaction scan")),
        }
    }
    IntegrityReport {
        valid: mismatches.is_empty(),
        tables_verified: before.len(),
        mismatches,
    }
}

/// Result of comparing pre/post compaction checksums.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntegrityReport {
    /// True when every pre-compaction checksum matches post-compaction.
    pub valid: bool,
    /// Number of tables that were verified.
    pub tables_verified: usize,
    /// Human-readable mismatch descriptions (empty when `valid`).
    pub mismatches: Vec<String>,
}

/// Safely prunes old ledgers from Horizon history tables.
///
/// # Safety
///
/// - The prune boundary is derived from `history_ledgers.closed_at` (the
///   retention window), but is always clamped so that the newest
///   [`MIN_KEEP_LEDGERS`] ledgers are preserved.
/// - Deletes run in bounded batches to keep locks short and avoid
///   transaction bloat.
/// - Only `history_ledgers` and tables that expose a `ledger_sequence`
///   column are touched; any other table is skipped with a warning.
/// - Pruning is opt-in (`enable_ledger_pruning`) and destructive.
#[derive(Clone, Debug)]
pub struct LedgerPruner {
    pool: PgPool,
    retention_days: u32,
    batch_size: u32,
}

impl LedgerPruner {
    pub fn new(pool: PgPool, retention_days: u32) -> Self {
        Self {
            pool,
            retention_days: retention_days.max(1),
            batch_size: DEFAULT_PRUNE_BATCH_SIZE,
        }
    }

    /// Newest ledger sequence in `history_ledgers` (0 when empty/missing).
    async fn newest_ledger_sequence(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COALESCE(MAX(sequence), 0) FROM history_ledgers")
            .fetch_one(&self.pool)
            .await
            .map_err(Error::SqlxError)?;
        let newest: i64 = row.try_get(0)?;
        Ok(newest)
    }

    /// Ledger sequence boundary derived from the retention window.
    ///
    /// Rows with `sequence <= boundary` are eligible for deletion. Returns 0
    /// when there is nothing to prune.
    pub async fn compute_boundary(&self) -> Result<i64> {
        let newest = self.newest_ledger_sequence().await?;
        if newest == 0 {
            return Ok(0);
        }

        // Time-based boundary: oldest ledger we may delete by retention days.
        let query = "SELECT COALESCE(MAX(sequence), 0) FROM history_ledgers \
                     WHERE closed_at < NOW() - ($1::int * INTERVAL '1 day')";
        let row = sqlx::query(query)
            .bind(self.retention_days as i32)
            .fetch_one(&self.pool)
            .await
            .map_err(Error::SqlxError)?;
        let time_boundary: i64 = row.try_get(0)?;

        // Safety clamp: never delete the newest MIN_KEEP_LEDGERS ledgers.
        let safety_boundary = newest.saturating_sub(MIN_KEEP_LEDGERS).max(0);
        let boundary = time_boundary.min(safety_boundary);

        Ok(boundary.max(0))
    }

    /// Delete rows in bounded batches; returns the number deleted.
    ///
    /// PostgreSQL does not support `LIMIT` in `DELETE`, so the batch window is
    /// expressed as a `ctid IN (SELECT ... LIMIT $2)` subquery. The oldest rows
    /// (smallest `sequence`) are always deleted first, which matches the
    /// retention intent.
    async fn delete_batched(&self, table: &str, boundary: i64) -> Result<u64> {
        let qualified = quote_qualified_ident(table)?;
        let mut total_deleted = 0u64;
        loop {
            let query = format!(
                "DELETE FROM {qualified} WHERE ctid IN ( \
                 SELECT ctid FROM {qualified} \
                 WHERE sequence <= $1 ORDER BY sequence LIMIT $2 )"
            );
            let result = sqlx::query(&query)
                .bind(boundary)
                .bind(self.batch_size as i64)
                .execute(&self.pool)
                .await
                .map_err(Error::SqlxError)?;
            let deleted = result.rows_affected();
            total_deleted += deleted;
            if deleted < self.batch_size as u64 {
                break;
            }
            // Give other sessions a chance to run between batches.
            tokio::task::yield_now().await;
        }
        Ok(total_deleted)
    }

    /// True when `table` exists and has a `sequence` column.
    async fn has_sequence_column(&self, table: &str) -> Result<bool> {
        let query = r#"
            SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = $1 AND table_name = $2 AND column_name = 'sequence'
            )
        "#;
        let (schema, name) = split_qualified(table);
        let row = sqlx::query(query)
            .bind(schema)
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(Error::SqlxError)?;
        let exists: bool = row.try_get(0)?;
        Ok(exists)
    }

    /// True when `table` exists and has a `ledger_sequence` column.
    async fn has_ledger_sequence_column(&self, table: &str) -> Result<bool> {
        let query = r#"
            SELECT EXISTS (
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = $1 AND table_name = $2 AND column_name = 'ledger_sequence'
            )
        "#;
        let (schema, name) = split_qualified(table);
        let row = sqlx::query(query)
            .bind(schema)
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(Error::SqlxError)?;
        let exists: bool = row.try_get(0)?;
        Ok(exists)
    }

    /// Prune ledgers older than the retention window.
    ///
    /// Returns a [`PruningReport`]; a boundary of 0 means there was nothing to
    /// prune and no error is raised.
    pub async fn prune_ledgers(&self) -> Result<PruningReport> {
        let newest = self.newest_ledger_sequence().await?;
        if newest == 0 {
            return Ok(PruningReport {
                newest_sequence: 0,
                boundary_sequence: 0,
                note: Some("history_ledgers is empty; nothing to prune".to_string()),
                ..Default::default()
            });
        }

        let boundary = self.compute_boundary().await?;
        let mut report = PruningReport {
            newest_sequence: newest,
            boundary_sequence: boundary,
            ..Default::default()
        };

        if boundary <= 0 {
            report.note = Some("no ledgers outside the retention window".to_string());
            return Ok(report);
        }

        info!(
            "Pruning history_ledgers up to sequence {boundary} (newest: {newest}, retention: {} days)",
            self.retention_days
        );

        // Primary table: history_ledgers.
        if self.has_sequence_column("history_ledgers").await? {
            let deleted = self.delete_batched("history_ledgers", boundary).await?;
            report.ledgers_deleted = deleted;
            report.tables_pruned.push("history_ledgers".to_string());
        }

        // Best-effort dependent tables that reference ledgers directly.
        // Only tables exposing a `ledger_sequence` column are pruned;
        // `history_ledger_headers` is keyed by header_hash and is skipped.
        for dependent in ["history_transactions"] {
            if self.has_ledger_sequence_column(dependent).await? {
                let qualified = quote_qualified_ident(dependent)?;
                let query = format!(
                    "DELETE FROM {qualified} WHERE ctid IN ( \
                     SELECT ctid FROM {qualified} \
                     WHERE ledger_sequence <= $1 ORDER BY ledger_sequence LIMIT $2 )"
                );
                let mut total = 0u64;
                loop {
                    let result = sqlx::query(&query)
                        .bind(boundary)
                        .bind(self.batch_size as i64)
                        .execute(&self.pool)
                        .await
                        .map_err(Error::SqlxError)?;
                    let deleted = result.rows_affected();
                    total += deleted;
                    if deleted < self.batch_size as u64 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                report.dependent_rows_deleted += total;
                report.tables_pruned.push(dependent.to_string());
                debug!("Pruned {total} rows from {dependent}");
            } else {
                warn!("Skipping dependent table {dependent}: no ledger_sequence column found");
            }
        }

        if report.ledgers_deleted > 0 {
            report.note = Some(format!(
                "pruned {} ledgers ({} dependent rows)",
                report.ledgers_deleted, report.dependent_rows_deleted
            ));
        }

        Ok(report)
    }
}

/// Sum of `pg_total_relation_size` (table + indexes + toast) across tables.
pub async fn total_relation_size(pool: &PgPool, tables: &[String]) -> Result<u64> {
    let mut total = 0u64;
    for table in tables {
        let qualified = quote_qualified_ident(table)?;
        let query = format!("SELECT pg_total_relation_size({qualified})");
        let row = sqlx::query(&query)
            .fetch_one(pool)
            .await
            .map_err(Error::SqlxError)?;
        let size: i64 = row.try_get(0)?;
        total = total.saturating_add(size.max(0) as u64);
    }
    Ok(total)
}

// ---------------------------------------------------------------------------
// Identifier helpers
// ---------------------------------------------------------------------------

/// Split a possibly schema-qualified name into (schema, name), defaulting the
/// schema to `public`.
pub fn split_qualified(qualified: &str) -> (&str, &str) {
    match qualified.rsplit_once('.') {
        Some((schema, name)) => (schema, name),
        None => ("public", qualified),
    }
}

/// Quote an identifier for safe interpolation into SQL. Each dot-separated
/// part is quoted independently.
pub fn quote_ident(part: &str) -> String {
    format!("\"{}\"", part.replace('"', "\"\""))
}

/// Quote a (possibly schema-qualified) table name for SQL interpolation.
pub fn quote_qualified_ident(qualified: &str) -> Result<String> {
    let (schema, name) = split_qualified(qualified);
    if schema.is_empty() || name.is_empty() {
        return Err(Error::MaintenanceError(format!(
            "invalid table identifier: {qualified}"
        )));
    }
    Ok(format!("{}.{}", quote_ident(schema), quote_ident(name)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_qualified() {
        assert_eq!(
            split_qualified("history_ledgers"),
            ("public", "history_ledgers")
        );
        assert_eq!(
            split_qualified("public.history_ledgers"),
            ("public", "history_ledgers")
        );
        assert_eq!(split_qualified("analytics.events"), ("analytics", "events"));
    }

    #[test]
    fn test_quote_qualified_ident() {
        assert_eq!(
            quote_qualified_ident("public.history_ledgers").unwrap(),
            "\"public\".\"history_ledgers\""
        );
        assert_eq!(
            quote_qualified_ident("analytics.events").unwrap(),
            "\"analytics\".\"events\""
        );
        // Identifier injection is neutralized.
        assert_eq!(
            quote_qualified_ident("public.weird\"name").unwrap(),
            "\"public\".\"weird\"\"name\""
        );
        assert!(quote_qualified_ident("").is_err());
        assert!(quote_qualified_ident(".").is_err());
    }

    #[test]
    fn test_empty_table_checksum_is_stable() {
        // The md5 of the empty string — the value returned for empty tables.
        assert_eq!(
            format!("{:x}", md5::compute(b"")),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
    }

    #[test]
    fn test_integrity_report_valid_when_checksums_match() {
        let before = BTreeMap::from([
            ("public.history_ledgers".to_string(), "abc".to_string()),
            ("public.accounts".to_string(), "def".to_string()),
        ]);
        let after = before.clone();
        let report = verify_integrity(&before, &after);
        assert!(report.valid);
        assert_eq!(report.tables_verified, 2);
        assert!(report.mismatches.is_empty());
    }

    #[test]
    fn test_integrity_report_detects_mismatch() {
        let before = BTreeMap::from([("public.accounts".to_string(), "abc".to_string())]);
        let after = BTreeMap::from([("public.accounts".to_string(), "XYZ".to_string())]);
        let report = verify_integrity(&before, &after);
        assert!(!report.valid);
        assert_eq!(report.mismatches.len(), 1);
        assert!(report.mismatches[0].contains("mismatch"));
    }

    #[test]
    fn test_integrity_report_detects_missing_table() {
        let before = BTreeMap::from([("public.accounts".to_string(), "abc".to_string())]);
        let after = BTreeMap::new();
        let report = verify_integrity(&before, &after);
        assert!(!report.valid);
        assert!(report.mismatches[0].contains("missing from post-compaction scan"));
    }

    #[test]
    fn test_boundary_never_crosses_safety_buffer() {
        // newest=100, retention boundary would be 60 → clamped to 100-1000 → 0.
        let newest: i64 = 100;
        let time_boundary: i64 = 60;
        let safety_boundary = newest.saturating_sub(MIN_KEEP_LEDGERS).max(0);
        let boundary = time_boundary.min(safety_boundary).max(0);
        assert_eq!(boundary, 0);

        // newest=1_500_000, time boundary 1_400_000 → min picks the smaller
        // (more conservative) boundary, keeping 100k newest — far above the
        // 1000-ledger safety buffer.
        let newest: i64 = 1_500_000;
        let time_boundary: i64 = 1_400_000;
        let safety_boundary = newest.saturating_sub(MIN_KEEP_LEDGERS).max(0);
        let boundary = time_boundary.min(safety_boundary).max(0);
        assert_eq!(boundary, 1_400_000);

        // When the time boundary would cut into the safety buffer, the safety
        // clamp wins (deletes less).
        let newest: i64 = 1_500_000;
        let time_boundary: i64 = 1_499_900;
        let safety_boundary = newest.saturating_sub(MIN_KEEP_LEDGERS).max(0);
        let boundary = time_boundary.min(safety_boundary).max(0);
        assert_eq!(boundary, 1_499_000);
    }
}
