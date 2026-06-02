//! Maintenance Window Controller logic
//!
//! Manages the lifecycle of maintenance windows and triggers DB tasks.

use super::bloat::BloatDetector;
use super::coordinator::MaintenanceCoordinator;
use super::query_profiler::QueryProfiler;
use crate::crd::{types::DbMaintenanceConfig, StellarNode};
use crate::error::Result;
use chrono::{Local, NaiveTime};
use regex::Regex;
use sqlx::PgPool;
use tracing::{debug, info, warn};

pub struct MaintenanceController {
    coordinator: MaintenanceCoordinator,
}

impl MaintenanceController {
    pub fn new(coordinator: MaintenanceCoordinator) -> Self {
        Self { coordinator }
    }

    /// Check if we are currently in a maintenance window
    pub fn is_in_window(&self, node: &StellarNode) -> bool {
        let config = match &node.spec.db_maintenance_config {
            Some(c) if c.enabled => c,
            _ => return false,
        };

        is_time_in_window(&config, Local::now().time())
    }

    /// Run maintenance tasks for a node if needed
    pub async fn run_maintenance(&self, node: &StellarNode, pool: PgPool) -> Result<()> {
        if !self.is_in_window(node) {
            return Ok(());
        }

        let config = node.spec.db_maintenance_config.as_ref().unwrap();
        let detector = BloatDetector::new(pool.clone());

        // Check for active ledger writes to avoid interference
        if !detector.is_system_quiet().await? {
            debug!(
                "Skipping maintenance for node {} due to active ledger writes",
                node.metadata.name.as_ref().unwrap()
            );
            return Ok(());
        }

        let bloated_tables = detector
            .get_bloated_tables(config.bloat_threshold_percent)
            .await?;

        if bloated_tables.is_empty() {
            debug!(
                "No bloated tables found for node {}",
                node.metadata.name.as_ref().unwrap()
            );
            return Ok(());
        }

        info!(
            "Starting maintenance for node {}: found {} bloated tables",
            node.metadata.name.as_ref().unwrap(),
            bloated_tables.len()
        );

        if config.read_pool_coordination {
            self.coordinator.prepare_node(node).await?;
        }

        for table in bloated_tables {
            info!("Running VACUUM ANALYZE on table {table}");
            sqlx::query(&format!("VACUUM ANALYZE {table}"))
                .execute(&pool)
                .await?;

            // Trigger REPACK if bloat is extremely high (e.g., > 60%)
            let bloat = detector.estimate_table_bloat(&table).await?;
            if bloat > 60.0 {
                info!("High bloat detected ({bloat}%), triggering pg_repack on {table}");
                // Note: pg_repack must be installed in the database
                if let Err(e) = sqlx::query("SELECT pg_repack.repack_table($1)")
                    .bind(&table)
                    .execute(&pool)
                    .await
                {
                    warn!("pg_repack failed for {table} (ensure extension is installed): {e}");
                }
            }

            if config.auto_reindex {
                info!("Reindexing table {table}");
                sqlx::query(&format!("REINDEX TABLE {table}"))
                    .execute(&pool)
                    .await?;
            }
        }

        if config.enable_query_profiling || config.auto_index_maintenance {
            let profiler = QueryProfiler::new(pool.clone());
            let slow_queries = profiler
                .collect_slow_queries(config.slow_query_threshold_ms)
                .await?;

            if slow_queries.is_empty() {
                debug!("No slow queries detected for node {}", node.metadata.name.as_ref().unwrap());
            } else {
                info!(
                    "Query profiling detected {} slow queries for node {}",
                    slow_queries.len(),
                    node.metadata.name.as_ref().unwrap()
                );

                if config.auto_index_maintenance {
                    let suggestions = profiler.recommend_indexes(&slow_queries);
                    if !suggestions.is_empty() {
                        info!(
                            "Applying {} index recommendations for node {}",
                            suggestions.len(),
                            node.metadata.name.as_ref().unwrap()
                        );
                        profiler.ensure_indexes(&suggestions).await?;
                    } else {
                        debug!("No index suggestions generated for node {}", node.metadata.name.as_ref().unwrap());
                    }
                }
            }
        }

        if config.read_pool_coordination {
            self.coordinator.finalize_maintenance(node).await?;
        }

        Ok(())
    }
}

fn parse_window_duration(value: &str) -> chrono::Duration {
    let capture = Regex::new(r"(?i)^(?:(?P<h>\d+)h)?(?:(?P<m>\d+)m)?(?:(?P<s>\d+)s)?$").unwrap();
    if let Some(caps) = capture.captures(value.trim()) {
        let hours = caps.name("h").and_then(|m| m.as_str().parse::<i64>().ok()).unwrap_or(0);
        let minutes = caps.name("m").and_then(|m| m.as_str().parse::<i64>().ok()).unwrap_or(0);
        let seconds = caps.name("s").and_then(|m| m.as_str().parse::<i64>().ok()).unwrap_or(0);
        if hours == 0 && minutes == 0 && seconds == 0 {
            return chrono::Duration::hours(2);
        }
        return chrono::Duration::hours(hours) + chrono::Duration::minutes(minutes) + chrono::Duration::seconds(seconds);
    }
    chrono::Duration::hours(2)
}

fn is_time_in_window(config: &DbMaintenanceConfig, now: NaiveTime) -> bool {
    let start = NaiveTime::parse_from_str(&config.window_start, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(&config.window_start, "%H:%M:%S"))
        .unwrap_or_else(|_| NaiveTime::from_hms_opt(2, 0, 0).unwrap());
    let duration = parse_window_duration(&config.window_duration);
    let end = start + duration;

    if duration.num_seconds() <= 0 {
        return true;
    }

    if duration >= chrono::Duration::hours(24) {
        return true;
    }

    if end >= start {
        now >= start && now <= end
    } else {
        now >= start || now <= end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crd::types::DbMaintenanceConfig;

    #[test]
    fn test_is_time_in_window_basic() {
        let cfg = DbMaintenanceConfig {
            enabled: true,
            window_start: "02:00".to_string(),
            window_duration: "2h".to_string(),
            bloat_threshold_percent: 30,
            auto_reindex: true,
            read_pool_coordination: false,
            enable_query_profiling: false,
            auto_index_maintenance: false,
            slow_query_threshold_ms: 100,
        };

        assert!(is_time_in_window(&cfg, NaiveTime::from_hms_opt(2, 30, 0).unwrap()));
        assert!(!is_time_in_window(&cfg, NaiveTime::from_hms_opt(4, 1, 0).unwrap()));
    }

    #[test]
    fn test_is_time_in_window_wraps_midnight() {
        let cfg = DbMaintenanceConfig {
            enabled: true,
            window_start: "23:00".to_string(),
            window_duration: "3h".to_string(),
            bloat_threshold_percent: 30,
            auto_reindex: true,
            read_pool_coordination: false,
            enable_query_profiling: false,
            auto_index_maintenance: false,
            slow_query_threshold_ms: 100,
        };

        assert!(is_time_in_window(&cfg, NaiveTime::from_hms_opt(23, 30, 0).unwrap()));
        assert!(is_time_in_window(&cfg, NaiveTime::from_hms_opt(0, 30, 0).unwrap()));
        assert!(!is_time_in_window(&cfg, NaiveTime::from_hms_opt(2, 30, 1).unwrap()));
    }

    #[test]
    fn test_parse_window_duration_falls_back_to_default() {
        assert_eq!(parse_window_duration("2h"), chrono::Duration::hours(2));
        assert_eq!(parse_window_duration("90m"), chrono::Duration::minutes(90));
        assert_eq!(parse_window_duration("invalid"), chrono::Duration::hours(2));
    }
}
