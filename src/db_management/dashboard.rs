//! Database performance dashboard: aggregates all sub-system reports into a
//! single snapshot and renders an HTML page.

use crate::db_management::{
    backup_pitr::BackupReport,
    index_manager::IndexReport,
    pool_optimizer::PoolReport,
    query_analysis::QueryAnalysisReport,
    replication::ReplicationReport,
    types::{DbAlert, DbTarget, HealthStatus},
    vacuum_scheduler::VacuumReport,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Full dashboard snapshot
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub target: String,
    pub taken_at: DateTime<Utc>,
    pub overall_health: HealthStatus,
    pub all_alerts: Vec<DbAlert>,
    pub query: QueryAnalysisReport,
    pub indexes: IndexReport,
    pub vacuum: VacuumReport,
    pub pool: PoolReport,
    pub replication: ReplicationReport,
    pub backup: BackupReport,
}

impl DashboardSnapshot {
    pub fn new(
        target: &DbTarget,
        query: QueryAnalysisReport,
        indexes: IndexReport,
        vacuum: VacuumReport,
        pool: PoolReport,
        replication: ReplicationReport,
        backup: BackupReport,
    ) -> Self {
        let mut all_alerts: Vec<DbAlert> = vec![];
        all_alerts.extend(query.alerts.clone());
        all_alerts.extend(indexes.alerts.clone());
        all_alerts.extend(vacuum.alerts.clone());
        all_alerts.extend(pool.alerts.clone());
        all_alerts.extend(replication.alerts.clone());
        all_alerts.extend(backup.alerts.clone());

        let overall_health = if all_alerts.iter().any(|a| a.level == HealthStatus::Critical) {
            HealthStatus::Critical
        } else if all_alerts.iter().any(|a| a.level == HealthStatus::Warning) {
            HealthStatus::Warning
        } else {
            HealthStatus::Healthy
        };

        Self {
            target: target.to_string(),
            taken_at: Utc::now(),
            overall_health,
            all_alerts,
            query,
            indexes,
            vacuum,
            pool,
            replication,
            backup,
        }
    }

    /// Render an HTML performance dashboard.
    pub fn render_html(&self) -> String {
        let health_color = match self.overall_health {
            HealthStatus::Healthy => "#2e7d32",
            HealthStatus::Warning => "#f57c00",
            HealthStatus::Critical => "#c62828",
        };
        let health_label = match self.overall_health {
            HealthStatus::Healthy => "HEALTHY",
            HealthStatus::Warning => "WARNING",
            HealthStatus::Critical => "CRITICAL",
        };

        let alert_rows: String = self.all_alerts.iter().map(|a| {
            let color = match a.level { HealthStatus::Critical => "#ffebee", HealthStatus::Warning => "#fff8e1", HealthStatus::Healthy => "#e8f5e9" };
            format!("<tr style='background:{color}'><td>{}</td><td>{}</td><td>{}</td></tr>",
                match a.level { HealthStatus::Critical => "🔴 CRITICAL", HealthStatus::Warning => "🟡 WARNING", HealthStatus::Healthy => "🟢 OK" },
                a.subsystem, a.message)
        }).collect();

        let slow_rows: String = self.query.top_slow_queries.iter().take(5).map(|q| {
            format!("<tr><td>{:.0}ms</td><td>{}</td><td>{}</td></tr>",
                q.mean_exec_time_ms, q.calls,
                q.query.chars().take(100).collect::<String>())
        }).collect();

        let pool_stats = &self.pool.stats;
        let repl_rows: String = self.replication.replicas.iter().map(|r| {
            format!("<tr><td>{}</td><td>{}</td><td>{:.0}ms</td></tr>",
                r.application_name, r.state,
                r.replay_lag_ms.unwrap_or(0.0))
        }).collect();

        format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8"/>
  <title>Stellar-K8s DB Dashboard — {target}</title>
  <style>
    body{{font-family:sans-serif;margin:2rem;background:#fafafa}}
    h1{{color:#1a73e8}} h2{{color:#333;border-bottom:2px solid #eee;padding-bottom:4px}}
    .badge{{display:inline-block;padding:6px 16px;border-radius:4px;color:#fff;font-weight:bold;background:{health_color}}}
    table{{border-collapse:collapse;width:100%;margin-bottom:1.5rem}}
    th,td{{border:1px solid #ddd;padding:8px;text-align:left;font-size:.9rem}}
    th{{background:#f2f2f2}} .card{{background:#fff;border:1px solid #ddd;border-radius:6px;padding:1rem;margin-bottom:1rem}}
    .metric{{display:inline-block;margin:0 1rem;text-align:center}} .metric .val{{font-size:2rem;font-weight:bold;color:#1a73e8}}
  </style>
</head>
<body>
  <h1>Stellar-K8s Database Dashboard</h1>
  <div class="card">
    <strong>Target:</strong> {target} &nbsp;|&nbsp;
    <strong>Generated:</strong> {taken_at} &nbsp;|&nbsp;
    <span class="badge">{health_label}</span>
  </div>

  <h2>Connection Pool</h2>
  <div class="card">
    <div class="metric"><div class="val">{total}</div>Total</div>
    <div class="metric"><div class="val">{active}</div>Active</div>
    <div class="metric"><div class="val">{idle}</div>Idle</div>
    <div class="metric"><div class="val">{waiting}</div>Waiting</div>
    <div class="metric"><div class="val">{util:.1}%</div>Utilization</div>
  </div>

  <h2>Alerts ({alert_count})</h2>
  <table><thead><tr><th>Level</th><th>Subsystem</th><th>Message</th></tr></thead>
  <tbody>{alert_rows}</tbody></table>

  <h2>Top Slow Queries</h2>
  <table><thead><tr><th>Avg Time</th><th>Calls</th><th>Query</th></tr></thead>
  <tbody>{slow_rows}</tbody></table>

  <h2>Replication</h2>
  <p><strong>Role:</strong> {role}</p>
  <table><thead><tr><th>Replica</th><th>State</th><th>Replay Lag</th></tr></thead>
  <tbody>{repl_rows}</tbody></table>

  <h2>Backup / PITR</h2>
  <div class="card">
    <strong>Current LSN:</strong> {lsn} &nbsp;|&nbsp;
    <strong>WAL Level:</strong> {wal_level} &nbsp;|&nbsp;
    <strong>Archive Mode:</strong> {archive_mode}
  </div>
</body>
</html>"#,
            target = self.target,
            taken_at = self.taken_at.format("%Y-%m-%d %H:%M UTC"),
            health_color = health_color,
            health_label = health_label,
            total = pool_stats.total_connections,
            active = pool_stats.active,
            idle = pool_stats.idle,
            waiting = pool_stats.waiting,
            util = pool_stats.utilization_pct,
            alert_count = self.all_alerts.len(),
            alert_rows = alert_rows,
            slow_rows = slow_rows,
            repl_rows = repl_rows,
            role = if self.replication.is_primary { "Primary" } else { "Replica" },
            lsn = self.backup.current_lsn,
            wal_level = self.backup.wal_level,
            archive_mode = self.backup.archive_mode,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_management::{
        backup_pitr::BackupReport,
        index_manager::IndexReport,
        pool_optimizer::{PoolReport, PoolStats},
        query_analysis::QueryAnalysisReport,
        replication::ReplicationReport,
        vacuum_scheduler::VacuumReport,
    };

    fn empty_snapshot() -> DashboardSnapshot {
        DashboardSnapshot::new(
            &DbTarget::Horizon,
            QueryAnalysisReport { analyzed_at: Utc::now(), top_slow_queries: vec![], top_expensive_queries: vec![], alerts: vec![] },
            IndexReport { analyzed_at: Utc::now(), missing_index_hints: vec![], unused_indexes: vec![], alerts: vec![] },
            VacuumReport { analyzed_at: Utc::now(), bloated_tables: vec![], vacuumed_tables: vec![], alerts: vec![] },
            PoolReport { analyzed_at: Utc::now(), stats: PoolStats { total_connections: 5, active: 2, idle: 3, waiting: 0, max_connections: 100, utilization_pct: 5.0 }, recommendation: None, alerts: vec![] },
            ReplicationReport { analyzed_at: Utc::now(), is_primary: true, replicas: vec![], alerts: vec![] },
            BackupReport { analyzed_at: Utc::now(), current_lsn: "0/1000000".into(), wal_level: "replica".into(), archive_mode: "on".into(), recent_backups: vec![], alerts: vec![] },
        )
    }

    #[test]
    fn healthy_when_no_alerts() {
        let snap = empty_snapshot();
        assert_eq!(snap.overall_health, HealthStatus::Healthy);
    }

    #[test]
    fn html_contains_target() {
        let snap = empty_snapshot();
        assert!(snap.render_html().contains("horizon"));
    }
}
