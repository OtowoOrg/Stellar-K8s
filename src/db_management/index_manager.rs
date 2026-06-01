//! Automatic index creation and maintenance.
//!
//! Detects missing indexes via pg_stat_user_tables seq-scan heuristics and
//! identifies unused indexes that waste write overhead.

use crate::db_management::types::DbAlert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MissingIndexHint {
    pub table: String,
    pub seq_scans: i64,
    pub seq_tup_read: i64,
    pub suggested_action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnusedIndex {
    pub schema: String,
    pub table: String,
    pub index: String,
    pub index_size: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexReport {
    pub analyzed_at: DateTime<Utc>,
    pub missing_index_hints: Vec<MissingIndexHint>,
    pub unused_indexes: Vec<UnusedIndex>,
    pub alerts: Vec<DbAlert>,
}

pub struct IndexManager;

impl IndexManager {
    /// Analyze index health and return recommendations.
    pub async fn analyze(pool: &PgPool) -> crate::error::Result<IndexReport> {
        // Tables with high sequential scan counts but no index scans → likely missing index
        let missing: Vec<(String, i64, i64)> = sqlx::query_as(
            r#"SELECT relname, seq_scan, seq_tup_read
               FROM pg_stat_user_tables
               WHERE seq_scan > 100 AND idx_scan = 0
               ORDER BY seq_tup_read DESC
               LIMIT 20"#,
        )
        .fetch_all(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        // Indexes that have never been used
        let unused: Vec<(String, String, String, String)> = sqlx::query_as(
            r#"SELECT schemaname, tablename, indexname,
                      pg_size_pretty(pg_relation_size(indexrelid)) AS index_size
               FROM pg_stat_user_indexes
               JOIN pg_index USING (indexrelid)
               WHERE idx_scan = 0
                 AND NOT indisprimary
                 AND NOT indisunique
               ORDER BY pg_relation_size(indexrelid) DESC
               LIMIT 20"#,
        )
        .fetch_all(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        let hints: Vec<MissingIndexHint> = missing
            .into_iter()
            .map(|(table, seq_scans, seq_tup_read)| MissingIndexHint {
                suggested_action: format!(
                    "Consider adding an index on '{table}' (seq_scans={seq_scans})"
                ),
                table,
                seq_scans,
                seq_tup_read,
            })
            .collect();

        let unused_idxs: Vec<UnusedIndex> = unused
            .into_iter()
            .map(|(schema, table, index, index_size)| UnusedIndex {
                schema,
                table,
                index,
                index_size,
            })
            .collect();

        let mut alerts = vec![];
        if !hints.is_empty() {
            alerts.push(DbAlert::warn(
                "index_manager",
                format!("{} tables may benefit from new indexes", hints.len()),
            ));
        }
        if !unused_idxs.is_empty() {
            alerts.push(DbAlert::warn(
                "index_manager",
                format!("{} unused indexes found (wasting write overhead)", unused_idxs.len()),
            ));
        }

        info!(
            "index analysis: {} missing hints, {} unused indexes",
            hints.len(),
            unused_idxs.len()
        );

        Ok(IndexReport {
            analyzed_at: Utc::now(),
            missing_index_hints: hints,
            unused_indexes: unused_idxs,
            alerts,
        })
    }

    /// Create an index concurrently (non-blocking).
    pub async fn create_index_concurrently(
        pool: &PgPool,
        table: &str,
        columns: &[&str],
    ) -> crate::error::Result<String> {
        let col_list = columns.join(", ");
        let idx_name = format!(
            "idx_auto_{}_{}",
            table.replace('.', "_"),
            columns.join("_")
        );
        let ddl = format!(
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS {idx_name} ON {table} ({col_list})"
        );
        info!("creating index: {ddl}");
        sqlx::query(&ddl)
            .execute(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;
        Ok(idx_name)
    }

    /// Drop an unused index concurrently.
    pub async fn drop_index_concurrently(
        pool: &PgPool,
        index_name: &str,
    ) -> crate::error::Result<()> {
        let ddl = format!("DROP INDEX CONCURRENTLY IF EXISTS {index_name}");
        info!("dropping unused index: {index_name}");
        sqlx::query(&ddl)
            .execute(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_index_hint_fields() {
        let h = MissingIndexHint {
            table: "ledgers".into(),
            seq_scans: 500,
            seq_tup_read: 1_000_000,
            suggested_action: "add index".into(),
        };
        assert_eq!(h.table, "ledgers");
        assert_eq!(h.seq_scans, 500);
    }
}
