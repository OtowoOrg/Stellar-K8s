//! Connection pool optimizer.
//!
//! Reads pg_stat_activity to measure active/idle/waiting connections and
//! recommends pool size adjustments.

use crate::db_management::types::DbAlert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::info;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolStats {
    pub total_connections: i64,
    pub active: i64,
    pub idle: i64,
    pub waiting: i64,
    pub max_connections: i64,
    pub utilization_pct: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolRecommendation {
    pub current_max: u32,
    pub recommended_max: u32,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoolReport {
    pub analyzed_at: DateTime<Utc>,
    pub stats: PoolStats,
    pub recommendation: Option<PoolRecommendation>,
    pub alerts: Vec<DbAlert>,
}

pub struct PoolOptimizer {
    pool_min: u32,
    pool_max: u32,
}

impl PoolOptimizer {
    pub fn new(pool_min: u32, pool_max: u32) -> Self {
        Self { pool_min, pool_max }
    }

    pub async fn analyze(&self, pool: &PgPool) -> crate::error::Result<PoolReport> {
        let (total, active, idle, waiting): (i64, i64, i64, i64) = sqlx::query_as(
            r#"SELECT
                 count(*),
                 count(*) FILTER (WHERE state = 'active'),
                 count(*) FILTER (WHERE state = 'idle'),
                 count(*) FILTER (WHERE wait_event_type = 'Lock')
               FROM pg_stat_activity
               WHERE datname = current_database()"#,
        )
        .fetch_one(pool)
        .await
        .map_err(crate::error::Error::SqlxError)?;

        let (max_conn,): (i64,) =
            sqlx::query_as("SELECT setting::bigint FROM pg_settings WHERE name = 'max_connections'")
                .fetch_one(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

        let utilization = if max_conn == 0 { 0.0 } else { total as f64 / max_conn as f64 * 100.0 };

        let stats = PoolStats { total_connections: total, active, idle, waiting, max_connections: max_conn, utilization_pct: utilization };

        let mut alerts = vec![];
        if utilization > 80.0 {
            alerts.push(DbAlert::critical("pool_optimizer", format!("Connection utilization at {utilization:.1}%")));
        } else if utilization > 60.0 {
            alerts.push(DbAlert::warn("pool_optimizer", format!("Connection utilization at {utilization:.1}%")));
        }
        if waiting > 0 {
            alerts.push(DbAlert::warn("pool_optimizer", format!("{waiting} connections waiting on locks")));
        }

        // Recommend pool size: target ~70% utilization headroom
        let recommended = ((active as f64 * 1.5) as u32).clamp(self.pool_min, self.pool_max);
        let recommendation = if recommended != self.pool_max {
            Some(PoolRecommendation {
                current_max: self.pool_max,
                recommended_max: recommended,
                reason: format!("Based on {active} active connections; targeting 50% headroom"),
            })
        } else {
            None
        };

        info!("pool analysis: {total} total, {active} active, {idle} idle, {waiting} waiting");
        Ok(PoolReport { analyzed_at: Utc::now(), stats, recommendation, alerts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utilization_calc() {
        let stats = PoolStats { total_connections: 80, active: 40, idle: 40, waiting: 0, max_connections: 100, utilization_pct: 80.0 };
        assert_eq!(stats.utilization_pct, 80.0);
    }
}
