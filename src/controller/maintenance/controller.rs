//! Maintenance Window Controller logic
//!
//! Manages the lifecycle of maintenance windows and triggers DB tasks.

use super::bloat::BloatDetector;
use super::coordinator::MaintenanceCoordinator;
use crate::crd::StellarNode;
use crate::error::Result;
use chrono::{Local, NaiveTime};
use sqlx::PgPool;
use tracing::{debug, info};

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

        let now = Local::now().time();
        let start = NaiveTime::parse_from_str(&config.window_start, "%H:%M")
            .unwrap_or_else(|_| NaiveTime::from_hms_opt(2, 0, 0).unwrap());

        // Simplistic window check
        now >= start && now <= (start + chrono::Duration::hours(2)) // Default 2h window if duration not parsed
    }

    /// Run maintenance tasks for a node if needed
    pub async fn run_maintenance(&self, node: &StellarNode, pool: PgPool) -> Result<()> {
        if !self.is_in_window(node) {
            return Ok(());
        }

        let config = node.spec.db_maintenance_config.as_ref().unwrap();
        let detector = BloatDetector::new(pool.clone());

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
            info!("Running VACUUM FULL on table {}", table);
            sqlx::query(&format!("VACUUM FULL {}", table))
                .execute(&pool)
                .await?;

            if config.auto_reindex {
                info!("Reindexing table {}", table);
                sqlx::query(&format!("REINDEX TABLE {}", table))
                    .execute(&pool)
                    .await?;
            }
        }

        if config.read_pool_coordination {
            self.coordinator.finalize_maintenance(node).await?;
        }

        Ok(())
    }
}
