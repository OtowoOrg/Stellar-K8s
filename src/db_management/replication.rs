//! Database replication and failover management.
//!
//! Monitors pg_stat_replication lag and pg_is_in_recovery() to detect
//! replication health and trigger failover recommendations.

use crate::db_management::types::DbAlert;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicaStatus {
    pub application_name: String,
    pub client_addr: String,
    pub state: String,
    pub write_lag_ms: Option<f64>,
    pub flush_lag_ms: Option<f64>,
    pub replay_lag_ms: Option<f64>,
    pub sync_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicationReport {
    pub analyzed_at: DateTime<Utc>,
    pub is_primary: bool,
    pub replicas: Vec<ReplicaStatus>,
    pub alerts: Vec<DbAlert>,
}

/// Maximum acceptable replication lag before alerting (ms)
const LAG_WARN_MS: f64 = 5_000.0;
const LAG_CRITICAL_MS: f64 = 30_000.0;

pub struct ReplicationMonitor;

impl ReplicationMonitor {
    pub async fn analyze(pool: &PgPool) -> crate::error::Result<ReplicationReport> {
        let (is_replica,): (bool,) = sqlx::query_as("SELECT pg_is_in_recovery()")
            .fetch_one(pool)
            .await
            .map_err(crate::error::Error::SqlxError)?;

        let is_primary = !is_replica;
        let mut alerts = vec![];

        let replicas = if is_primary {
            let rows: Vec<(String, String, String, Option<f64>, Option<f64>, Option<f64>, String)> =
                sqlx::query_as(
                    r#"SELECT application_name,
                              coalesce(client_addr::text, 'local'),
                              state,
                              EXTRACT(EPOCH FROM write_lag)  * 1000,
                              EXTRACT(EPOCH FROM flush_lag)  * 1000,
                              EXTRACT(EPOCH FROM replay_lag) * 1000,
                              sync_state
                       FROM pg_stat_replication"#,
                )
                .fetch_all(pool)
                .await
                .map_err(crate::error::Error::SqlxError)?;

            rows.into_iter()
                .map(|(app, addr, state, wl, fl, rl, sync)| {
                    let rs = ReplicaStatus {
                        application_name: app,
                        client_addr: addr,
                        state,
                        write_lag_ms: wl,
                        flush_lag_ms: fl,
                        replay_lag_ms: rl,
                        sync_state: sync,
                    };
                    if let Some(lag) = rs.replay_lag_ms {
                        if lag > LAG_CRITICAL_MS {
                            alerts.push(DbAlert::critical(
                                "replication",
                                format!("Replica '{}' replay lag {lag:.0}ms", rs.application_name),
                            ));
                        } else if lag > LAG_WARN_MS {
                            alerts.push(DbAlert::warn(
                                "replication",
                                format!("Replica '{}' replay lag {lag:.0}ms", rs.application_name),
                            ));
                        }
                    }
                    rs
                })
                .collect()
        } else {
            warn!("replication monitor: this node is a replica");
            alerts.push(DbAlert::warn("replication", "Connected to a replica, not primary"));
            vec![]
        };

        info!("replication: primary={is_primary}, replicas={}", replicas.len());
        Ok(ReplicationReport { analyzed_at: Utc::now(), is_primary, replicas, alerts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_thresholds() {
        assert!(LAG_WARN_MS < LAG_CRITICAL_MS);
    }
}
