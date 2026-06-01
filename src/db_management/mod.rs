//! Sophisticated database management system for Horizon and Soroban.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  DbOptimizer (facade)                                    │
//! │                                                          │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
//! │  │ QueryAnalyzer│  │ IndexManager │  │VacuumScheduler│  │
//! │  └──────────────┘  └──────────────┘  └──────────────┘  │
//! │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
//! │  │PoolOptimizer │  │ Replication  │  │ BackupManager│  │
//! │  └──────────────┘  │  Monitor     │  └──────────────┘  │
//! │                    └──────────────┘                     │
//! │  ┌──────────────────────────────────────────────────┐   │
//! │  │  Dashboard (HTML report + alerts)                │   │
//! │  └──────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────┘
//! ```

pub mod backup_pitr;
pub mod dashboard;
pub mod index_manager;
pub mod pool_optimizer;
pub mod query_analysis;
pub mod replication;
pub mod types;
pub mod vacuum_scheduler;

pub use backup_pitr::BackupManager;
pub use dashboard::DashboardSnapshot;
pub use index_manager::IndexManager;
pub use pool_optimizer::PoolOptimizer;
pub use query_analysis::QueryAnalyzer;
pub use replication::ReplicationMonitor;
pub use types::{DbAlert, DbManagementConfig, DbTarget, HealthStatus};
pub use vacuum_scheduler::VacuumScheduler;

use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;

/// Unified database optimizer facade.
pub struct DbOptimizer {
    pool: PgPool,
    config: DbManagementConfig,
    query_analyzer: QueryAnalyzer,
    vacuum_scheduler: VacuumScheduler,
    pool_optimizer: PoolOptimizer,
    backup_manager: Arc<BackupManager>,
}

impl DbOptimizer {
    pub fn new(pool: PgPool, config: DbManagementConfig) -> Self {
        let query_analyzer = QueryAnalyzer::new(config.slow_query_threshold_ms);
        let vacuum_scheduler = VacuumScheduler::new(config.vacuum_bloat_threshold);
        let pool_optimizer = PoolOptimizer::new(config.pool_min, config.pool_max);
        let backup_manager = BackupManager::new();
        Self { pool, config, query_analyzer, vacuum_scheduler, pool_optimizer, backup_manager }
    }

    /// Run all analysis sub-systems and return a full dashboard snapshot.
    pub async fn run_full_analysis(&self) -> crate::error::Result<DashboardSnapshot> {
        info!("db_optimizer: running full analysis for {}", self.config.target);

        let (query, indexes, vacuum, pool, replication, backup) = tokio::try_join!(
            self.query_analyzer.analyze(&self.pool),
            IndexManager::analyze(&self.pool),
            self.vacuum_scheduler.run(&self.pool),
            self.pool_optimizer.analyze(&self.pool),
            ReplicationMonitor::analyze(&self.pool),
            self.backup_manager.report(&self.pool),
        )?;

        Ok(DashboardSnapshot::new(
            &self.config.target,
            query,
            indexes,
            vacuum,
            pool,
            replication,
            backup,
        ))
    }

    /// Convenience: return the HTML dashboard.
    pub async fn html_dashboard(&self) -> crate::error::Result<String> {
        Ok(self.run_full_analysis().await?.render_html())
    }

    /// Expose the backup manager for manual backup operations.
    pub fn backup_manager(&self) -> Arc<BackupManager> {
        self.backup_manager.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let cfg = DbManagementConfig::default();
        assert_eq!(cfg.slow_query_threshold_ms, 1_000);
        assert!((cfg.vacuum_bloat_threshold - 0.2).abs() < 1e-9);
        assert_eq!(cfg.pool_min, 2);
        assert_eq!(cfg.pool_max, 20);
    }

    #[test]
    fn db_target_display() {
        assert_eq!(DbTarget::Horizon.to_string(), "horizon");
        assert_eq!(DbTarget::Soroban.to_string(), "soroban");
        assert_eq!(DbTarget::Custom("mydb".into()).to_string(), "mydb");
    }
}
