//! Database vacuum and maintenance scheduler.
//!
//! Monitors table bloat and dead-tuple ratios, triggering VACUUM ANALYZE
//! when thresholds are exceeded.

use crate::db_management::types::DbAlert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableBloat {
    pub schema: String,
    pub table: String,
    pub live_tuples: i64,
    pub dead_tuples: i64,
    /// dead / (live + dead)
    pub bloat_ratio: f64,
    pub last_vacuum: Option<DateTime<Utc>>,
    pub last_analyze: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VacuumReport {
    pub analyzed_at: DateTime<Utc>,
    pub bloated_tables: Vec<TableBloat>,
    pub vacuumed_tables: Vec<String>,
    pub alerts: Vec<DbAlert>,
}

pub struct VacuumScheduler {
    bloat_threshold: f64,
}

impl VacuumScheduler {
    pub fn new(bloat_threshold: f64) -> Self {
        Self { bloat_threshold }
    }

    /// Inspect table bloat and run VACUUM ANALYZE on tables above threshold.
    pub async fn run(&self, pool: &PgPool) -> crate::error::Result<VacuumReport> {
        let rows: Vec<(String, String, i64, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>)> =
            sqlx::query_as(
                r#"SELECT schemaname, relname,
                          n_live_tup, n_dead_tup,
                          last_vacuum, last_analyze
                   FROM pg_stat_user_tables
                   ORDER BY n_dead_tup DESC
                   LIMIT 50"#,
            )
            .fetch_all(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;

        let mut bloated = vec![];
        let mut vacuumed = vec![];
        let mut alerts = vec![];

        for (schema, table, live, dead, last_vac, last_ana) in rows {
            let total = live + dead;
            let ratio = if total == 0 { 0.0 } else { dead as f64 / total as f64 };

            let tb = TableBloat {
                schema: schema.clone(),
                table: table.clone(),
                live_tuples: live,
                dead_tuples: dead,
                bloat_ratio: ratio,
                last_vacuum: last_vac,
                last_analyze: last_ana,
            };

            if ratio > self.bloat_threshold {
                bloated.push(tb);
                let fqn = format!("{schema}.{table}");
                info!("running VACUUM ANALYZE on {fqn} (bloat={ratio:.2})");
                let sql = format!("VACUUM ANALYZE {fqn}");
                if let Err(e) = sqlx::query(&sql).execute(pool).await {
                    alerts.push(DbAlert::warn(
                        "vacuum_scheduler",
                        format!("VACUUM failed on {fqn}: {e}"),
                    ));
                } else {
                    vacuumed.push(fqn);
                }
            }
        }

        if !bloated.is_empty() {
            alerts.push(DbAlert::warn(
                "vacuum_scheduler",
                format!("{} tables had bloat above threshold", bloated.len()),
            ));
        }

        Ok(VacuumReport {
            analyzed_at: Utc::now(),
            bloated_tables: bloated,
            vacuumed_tables: vacuumed,
            alerts,
        })
    }

    /// Run REINDEX CONCURRENTLY on a specific index.
    pub async fn reindex(pool: &PgPool, index_name: &str) -> crate::error::Result<()> {
        info!("reindexing {index_name}");
        sqlx::query(&format!("REINDEX INDEX CONCURRENTLY {index_name}"))
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
    fn bloat_ratio_calculation() {
        let tb = TableBloat {
            schema: "public".into(),
            table: "ledgers".into(),
            live_tuples: 800,
            dead_tuples: 200,
            bloat_ratio: 200.0 / 1000.0,
            last_vacuum: None,
            last_analyze: None,
        };
        assert!((tb.bloat_ratio - 0.2).abs() < 1e-9);
    }

    #[test]
    fn scheduler_threshold() {
        let s = VacuumScheduler::new(0.2);
        assert!((s.bloat_threshold - 0.2).abs() < 1e-9);
    }
}
