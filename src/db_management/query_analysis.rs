//! Query performance analysis and profiling via pg_stat_statements.

use crate::db_management::types::{DbAlert, HealthStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{info, warn};

/// A slow or expensive query captured from pg_stat_statements
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryStat {
    pub query: String,
    pub calls: i64,
    pub total_exec_time_ms: f64,
    pub mean_exec_time_ms: f64,
    pub rows: i64,
    pub shared_blks_hit: i64,
    pub shared_blks_read: i64,
}

impl QueryStat {
    /// Cache-hit ratio (0.0 – 1.0)
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.shared_blks_hit + self.shared_blks_read;
        if total == 0 { 1.0 } else { self.shared_blks_hit as f64 / total as f64 }
    }
}

/// Result of a full query analysis run
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryAnalysisReport {
    pub analyzed_at: DateTime<Utc>,
    pub top_slow_queries: Vec<QueryStat>,
    pub top_expensive_queries: Vec<QueryStat>,
    pub alerts: Vec<DbAlert>,
}

pub struct QueryAnalyzer {
    slow_threshold_ms: f64,
}

impl QueryAnalyzer {
    pub fn new(slow_threshold_ms: u64) -> Self {
        Self { slow_threshold_ms: slow_threshold_ms as f64 }
    }

    /// Run analysis against pg_stat_statements. Returns an empty report if the
    /// extension is not installed (non-fatal).
    pub async fn analyze(&self, pool: &PgPool) -> crate::error::Result<QueryAnalysisReport> {
        // Check extension availability
        let ext: Option<(bool,)> = sqlx::query_as(
            "SELECT true FROM pg_extension WHERE extname = 'pg_stat_statements' LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        if ext.is_none() {
            warn!("pg_stat_statements not installed; skipping query analysis");
            return Ok(QueryAnalysisReport {
                analyzed_at: Utc::now(),
                top_slow_queries: vec![],
                top_expensive_queries: vec![],
                alerts: vec![DbAlert::warn(
                    "query_analysis",
                    "pg_stat_statements extension not installed",
                )],
            });
        }

        // Top 10 by mean execution time
        let slow: Vec<(String, i64, f64, f64, i64, i64, i64)> = sqlx::query_as(
            r#"SELECT query, calls, total_exec_time, mean_exec_time, rows,
                      shared_blks_hit, shared_blks_read
               FROM pg_stat_statements
               ORDER BY mean_exec_time DESC
               LIMIT 10"#,
        )
        .fetch_all(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        // Top 10 by total execution time
        let expensive: Vec<(String, i64, f64, f64, i64, i64, i64)> = sqlx::query_as(
            r#"SELECT query, calls, total_exec_time, mean_exec_time, rows,
                      shared_blks_hit, shared_blks_read
               FROM pg_stat_statements
               ORDER BY total_exec_time DESC
               LIMIT 10"#,
        )
        .fetch_all(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        let to_stat = |(query, calls, total, mean, rows, hit, read): (String, i64, f64, f64, i64, i64, i64)| {
            QueryStat {
                query,
                calls,
                total_exec_time_ms: total,
                mean_exec_time_ms: mean,
                rows,
                shared_blks_hit: hit,
                shared_blks_read: read,
            }
        };

        let top_slow: Vec<QueryStat> = slow.into_iter().map(to_stat).collect();
        let top_expensive: Vec<QueryStat> = expensive.into_iter().map(to_stat).collect();

        let mut alerts = vec![];
        for q in &top_slow {
            if q.mean_exec_time_ms > self.slow_threshold_ms {
                alerts.push(DbAlert::warn(
                    "query_analysis",
                    format!(
                        "Slow query detected ({:.0}ms avg): {}",
                        q.mean_exec_time_ms,
                        q.query.chars().take(80).collect::<String>()
                    ),
                ));
            }
            if q.cache_hit_ratio() < 0.9 {
                alerts.push(DbAlert::warn(
                    "query_analysis",
                    format!("Low cache-hit ratio ({:.1}%)", q.cache_hit_ratio() * 100.0),
                ));
            }
        }

        info!(
            "query analysis: {} slow, {} expensive queries found",
            top_slow.len(),
            top_expensive.len()
        );

        Ok(QueryAnalysisReport {
            analyzed_at: Utc::now(),
            top_slow_queries: top_slow,
            top_expensive_queries: top_expensive,
            alerts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_ratio_zero_reads() {
        let s = QueryStat {
            query: "SELECT 1".into(),
            calls: 1,
            total_exec_time_ms: 1.0,
            mean_exec_time_ms: 1.0,
            rows: 1,
            shared_blks_hit: 100,
            shared_blks_read: 0,
        };
        assert_eq!(s.cache_hit_ratio(), 1.0);
    }

    #[test]
    fn cache_hit_ratio_mixed() {
        let s = QueryStat {
            query: "SELECT 1".into(),
            calls: 1,
            total_exec_time_ms: 1.0,
            mean_exec_time_ms: 1.0,
            rows: 1,
            shared_blks_hit: 80,
            shared_blks_read: 20,
        };
        assert!((s.cache_hit_ratio() - 0.8).abs() < 1e-9);
    }
}
