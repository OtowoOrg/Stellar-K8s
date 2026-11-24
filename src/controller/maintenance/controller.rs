// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Maintenance Window Controller logic.
//!
//! Manages the lifecycle of maintenance windows and triggers DB compaction.
//! This is a thin facade over [`super::compactor`]: the heavy lifting (drain,
//! compact, verify, rejoin) lives in [`super::compactor::run_compaction_cycle`].

use std::sync::Arc;

use chrono::NaiveTime;
use kube::{Client, ResourceExt};
use regex::Regex;
use sqlx::PgPool;
use tracing::{debug, info};

use super::compactor::{self, CompactionCoordinator};
use crate::crd::{DbMaintenanceConfig, StellarNode};
use crate::error::Result;

pub struct MaintenanceController {
    client: Client,
    coordinator: Arc<CompactionCoordinator>,
}

impl MaintenanceController {
    pub fn new(client: Client, coordinator: Arc<CompactionCoordinator>) -> Self {
        Self {
            client,
            coordinator,
        }
    }

    /// Check if we are currently in a maintenance window.
    pub fn is_in_window(&self, node: &StellarNode) -> bool {
        let config = match &node.spec.db_maintenance_config {
            Some(c) if c.enabled => c,
            _ => return false,
        };

        is_time_in_window(config, chrono::Local::now().time())
    }

    /// Run maintenance tasks for a node if needed.
    ///
    /// Executes the full compaction cycle: quiet check, fragmentation
    /// evaluation, traffic drain, compaction, ledger pruning, integrity
    /// verification, and traffic rejoin.
    pub async fn run_maintenance(&self, node: &StellarNode, pool: PgPool) -> Result<()> {
        if !self.is_in_window(node) {
            debug!(
                "Maintenance skipped for node {}: outside maintenance window",
                node.name_any()
            );
            return Ok(());
        }

        let report =
            compactor::run_compaction_cycle(&self.client, None, &self.coordinator, node, &pool)
                .await?;

        if let Some(skipped) = &report.skipped_reason {
            debug!(
                "Maintenance skipped for node {}: {skipped}",
                node.name_any()
            );
        } else {
            info!(
                "Maintenance complete for node {}: {} table(s) compacted, {} bytes freed, integrity={}, ledgers pruned={}",
                node.name_any(),
                report.tables_compacted.len(),
                report.bytes_freed,
                report.integrity_valid,
                report.ledgers_pruned
            );
        }

        Ok(())
    }
}

fn parse_window_duration(value: &str) -> chrono::Duration {
    let capture = Regex::new(r"(?i)^(?:(?P<h>\d+)h)?(?:(?P<m>\d+)m)?(?:(?P<s>\d+)s)?$").unwrap();
    if let Some(caps) = capture.captures(value.trim()) {
        let hours = caps
            .name("h")
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0);
        let minutes = caps
            .name("m")
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0);
        let seconds = caps
            .name("s")
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0);
        if hours == 0 && minutes == 0 && seconds == 0 {
            return chrono::Duration::hours(2);
        }
        return chrono::Duration::hours(hours)
            + chrono::Duration::minutes(minutes)
            + chrono::Duration::seconds(seconds);
    }
    chrono::Duration::hours(2)
}

pub fn is_time_in_window(config: &DbMaintenanceConfig, now: NaiveTime) -> bool {
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

    #[test]
    fn test_is_time_in_window_basic() {
        let cfg = DbMaintenanceConfig {
            enabled: true,
            window_start: "02:00".to_string(),
            window_duration: "2h".to_string(),
            schedule: None,
            bloat_threshold_percent: 30,
            auto_reindex: true,
            read_pool_coordination: false,
            enable_ledger_pruning: false,
            pruning_retention_days: 30,
        };

        assert!(is_time_in_window(
            &cfg,
            NaiveTime::from_hms_opt(2, 30, 0).unwrap()
        ));
        assert!(!is_time_in_window(
            &cfg,
            NaiveTime::from_hms_opt(4, 1, 0).unwrap()
        ));
    }

    #[test]
    fn test_is_time_in_window_wraps_midnight() {
        let cfg = DbMaintenanceConfig {
            enabled: true,
            window_start: "23:00".to_string(),
            window_duration: "3h".to_string(),
            schedule: None,
            bloat_threshold_percent: 30,
            auto_reindex: true,
            read_pool_coordination: false,
            enable_ledger_pruning: false,
            pruning_retention_days: 30,
        };

        assert!(is_time_in_window(
            &cfg,
            NaiveTime::from_hms_opt(23, 30, 0).unwrap()
        ));
        assert!(is_time_in_window(
            &cfg,
            NaiveTime::from_hms_opt(0, 30, 0).unwrap()
        ));
        assert!(!is_time_in_window(
            &cfg,
            NaiveTime::from_hms_opt(2, 30, 1).unwrap()
        ));
    }

    #[test]
    fn test_parse_window_duration_falls_back_to_default() {
        assert_eq!(parse_window_duration("2h"), chrono::Duration::hours(2));
        assert_eq!(parse_window_duration("90m"), chrono::Duration::minutes(90));
        assert_eq!(parse_window_duration("invalid"), chrono::Duration::hours(2));
    }
}
