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
//! First-class cost-savings metrics and hourly realized-savings aggregation
//! (#1484). Integrates with the existing cost dashboard Prometheus contract.

use chrono::{DateTime, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::crd::CapacityClass;

use super::capacity::{realized_hourly_savings, PlacementReport, DEFAULT_ON_DEMAND_HOURLY_USD};

/// First-class Prometheus metric name for realized cost savings.
pub const COST_SAVINGS_METRIC: &str = "stellar_cost_savings_usd";
/// Hourly realized savings (existing spot dashboard alias kept in sync).
pub const HOURLY_SAVINGS_METRIC: &str = "stellar_cost_savings_hourly_usd";
/// Backward-compatible alias already referenced by the cost dashboard.
pub const SPOT_SAVINGS_METRIC: &str = "stellar_spot_savings_usd";
pub const SPOT_PLACEMENT_RATIO_METRIC: &str = "stellar_best_effort_spot_placement_ratio";
pub const CRITICAL_ON_SPOT_METRIC: &str = "stellar_critical_on_spot_pods";

/// One hourly bucket of realized savings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HourlySavingsBucket {
    pub hour_start: DateTime<Utc>,
    pub realized_usd: f64,
    pub best_effort_on_spot: u64,
    pub best_effort_total: u64,
}

/// In-memory aggregator consumed by the dashboard / metrics exporter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavingsAggregator {
    buckets: BTreeMap<i64, HourlySavingsBucket>,
    cumulative_usd: f64,
}

impl SavingsAggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Floor an instant to the UTC hour.
    pub fn hour_floor(ts: DateTime<Utc>) -> DateTime<Utc> {
        ts.with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(ts)
    }

    /// Record one hour of placement-derived savings.
    pub fn record_placement(&mut self, ts: DateTime<Utc>, report: &PlacementReport) {
        let hour = Self::hour_floor(ts);
        let key = hour.timestamp();
        let mut realized = 0.0;
        let mut be_spot = 0u64;
        let mut be_total = 0u64;
        for p in &report.placements {
            if p.tier == crate::crd::WorkloadTier::BestEffort {
                be_total += 1;
                if p.capacity_class == CapacityClass::Spot {
                    be_spot += 1;
                    realized +=
                        realized_hourly_savings(CapacityClass::Spot, DEFAULT_ON_DEMAND_HOURLY_USD);
                }
            }
        }
        self.cumulative_usd += realized;
        self.buckets
            .entry(key)
            .and_modify(|b| {
                b.realized_usd += realized;
                b.best_effort_on_spot += be_spot;
                b.best_effort_total += be_total;
            })
            .or_insert(HourlySavingsBucket {
                hour_start: hour,
                realized_usd: realized,
                best_effort_on_spot: be_spot,
                best_effort_total: be_total,
            });
    }

    /// Replace the current hour's run-rate snapshot (live scrape; not additive).
    ///
    /// Polling every few seconds must not inflate hourly savings. The bucket
    /// stores the current USD/hour run-rate of live spot placements.
    pub fn set_hourly_run_rate(
        &mut self,
        ts: DateTime<Utc>,
        realized_usd: f64,
        best_effort_on_spot: u64,
        best_effort_total: u64,
    ) {
        let hour = Self::hour_floor(ts);
        let key = hour.timestamp();
        self.buckets.insert(
            key,
            HourlySavingsBucket {
                hour_start: hour,
                realized_usd,
                best_effort_on_spot,
                best_effort_total,
            },
        );
        self.cumulative_usd = self.buckets.values().map(|b| b.realized_usd).sum();
    }

    /// Record a raw hourly increment (used by the controller scrape).
    pub fn record_raw(&mut self, ts: DateTime<Utc>, realized_usd: f64) {
        let hour = Self::hour_floor(ts);
        let key = hour.timestamp();
        self.cumulative_usd += realized_usd;
        self.buckets
            .entry(key)
            .and_modify(|b| b.realized_usd += realized_usd)
            .or_insert(HourlySavingsBucket {
                hour_start: hour,
                realized_usd,
                best_effort_on_spot: 0,
                best_effort_total: 0,
            });
    }

    pub fn cumulative_usd(&self) -> f64 {
        self.cumulative_usd
    }

    pub fn bucket_at(&self, ts: DateTime<Utc>) -> Option<&HourlySavingsBucket> {
        self.buckets.get(&Self::hour_floor(ts).timestamp())
    }

    pub fn latest_hourly_usd(&self) -> f64 {
        self.buckets
            .values()
            .next_back()
            .map(|b| b.realized_usd)
            .unwrap_or(0.0)
    }

    /// Drop buckets older than `retain`.
    pub fn retain_since(&mut self, cutoff: DateTime<Utc>) {
        let cut = cutoff.timestamp();
        self.buckets.retain(|k, _| *k >= cut);
    }

    /// Prometheus exposition for the existing cost dashboard.
    pub fn prometheus_metrics(&self, spot_ratio: f64) -> String {
        self.prometheus_metrics_with_critical(spot_ratio, 0)
    }

    pub fn prometheus_metrics_with_critical(
        &self,
        spot_ratio: f64,
        critical_on_spot: u64,
    ) -> String {
        format!(
            "# HELP {COST_SAVINGS_METRIC} Realized cost savings versus on-demand (USD)\n\
             # TYPE {COST_SAVINGS_METRIC} gauge\n\
             {COST_SAVINGS_METRIC} {:.4}\n\
             # HELP {HOURLY_SAVINGS_METRIC} Realized cost savings in the current UTC hour (USD)\n\
             # TYPE {HOURLY_SAVINGS_METRIC} gauge\n\
             {HOURLY_SAVINGS_METRIC} {:.4}\n\
             # HELP {SPOT_SAVINGS_METRIC} Spot-instance savings (alias of realized cost savings)\n\
             # TYPE {SPOT_SAVINGS_METRIC} gauge\n\
             {SPOT_SAVINGS_METRIC} {:.4}\n\
             # HELP {SPOT_PLACEMENT_RATIO_METRIC} Share of best-effort pods on spot capacity\n\
             # TYPE {SPOT_PLACEMENT_RATIO_METRIC} gauge\n\
             {SPOT_PLACEMENT_RATIO_METRIC} {:.4}\n\
             # HELP {CRITICAL_ON_SPOT_METRIC} Critical-tier pods currently on spot (must be 0)\n\
             # TYPE {CRITICAL_ON_SPOT_METRIC} gauge\n\
             {CRITICAL_ON_SPOT_METRIC} {critical_on_spot}\n",
            self.cumulative_usd,
            self.latest_hourly_usd(),
            self.cumulative_usd,
            spot_ratio,
        )
    }
}

/// Hours in a sliding window used by the dashboard (default 24h).
pub fn window_start(now: DateTime<Utc>, hours: i64) -> DateTime<Utc> {
    now - Duration::hours(hours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::capacity::{
        best_effort_workload, critical_workload, defined_test_cluster, place_workloads,
    };

    #[test]
    fn cost_savings_metric_emitted() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..10)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let mut agg = SavingsAggregator::new();
        agg.record_placement(Utc::now(), &report);
        let text = agg.prometheus_metrics(report.best_effort_spot_ratio());
        assert!(text.contains(COST_SAVINGS_METRIC));
        assert!(text.contains(&format!("{COST_SAVINGS_METRIC} ")));
        assert!(agg.cumulative_usd() > 0.0);
    }

    #[test]
    fn hourly_aggregation_is_correct() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..5)
            .map(|i| best_effort_workload(&format!("be-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let mut agg = SavingsAggregator::new();
        let t0 = DateTime::parse_from_rfc3339("2026-09-25T10:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t1 = DateTime::parse_from_rfc3339("2026-09-25T10:45:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let t2 = DateTime::parse_from_rfc3339("2026-09-25T11:05:00Z")
            .unwrap()
            .with_timezone(&Utc);
        agg.record_placement(t0, &report);
        agg.record_placement(t1, &report);
        agg.record_placement(t2, &report);

        let hour10 = agg.bucket_at(t0).unwrap();
        let hour11 = agg.bucket_at(t2).unwrap();
        let single = {
            let mut tmp = SavingsAggregator::new();
            tmp.record_placement(t0, &report);
            tmp.latest_hourly_usd()
        };
        assert!((hour10.realized_usd - single * 2.0).abs() < 1e-9);
        assert!((hour11.realized_usd - single).abs() < 1e-9);
        assert_eq!(agg.bucket_at(t0).unwrap().hour_start.hour(), 10);
        assert_eq!(agg.bucket_at(t2).unwrap().hour_start.hour(), 11);
    }

    #[test]
    fn critical_placement_adds_no_spot_savings() {
        let nodes = defined_test_cluster();
        let workloads: Vec<_> = (0..4)
            .map(|i| critical_workload(&format!("c-{i}")))
            .collect();
        let report = place_workloads(&workloads, &nodes, &[]);
        let mut agg = SavingsAggregator::new();
        agg.record_placement(Utc::now(), &report);
        assert_eq!(agg.cumulative_usd(), 0.0);
        assert_eq!(report.critical_on_spot(), 0);
    }

    #[test]
    fn hourly_run_rate_snapshot_does_not_inflate_on_repeat() {
        let mut agg = SavingsAggregator::new();
        let ts = DateTime::parse_from_rfc3339("2026-09-25T10:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        agg.set_hourly_run_rate(ts, 1.344, 20, 20);
        agg.set_hourly_run_rate(ts, 1.344, 20, 20);
        agg.set_hourly_run_rate(ts, 1.344, 20, 20);
        assert!((agg.latest_hourly_usd() - 1.344).abs() < 1e-9);
        assert!((agg.cumulative_usd() - 1.344).abs() < 1e-9);
    }
}
