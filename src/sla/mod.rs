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
//! Uptime SLA tracking for validator operators.
//!
//! Probe results are accumulated into a rolling window and compared against
//! a configurable SLA target. The target is passed in as [`SlaConfig`] on every
//! evaluation, so it can be changed (e.g. from `spec.slaTarget`) without an
//! operator restart.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// SLA configuration for one node.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SlaConfig {
    /// Target availability in percent, e.g. `99.9`.
    pub target_percent: f64,
    /// Rolling window length in seconds used for breach detection.
    pub window_secs: i64,
}

impl Default for SlaConfig {
    fn default() -> Self {
        Self {
            target_percent: 99.9,
            window_secs: 30 * 24 * 3600,
        }
    }
}

/// A single probe result.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProbeSample {
    /// Unix timestamp (seconds) of the probe.
    pub ts: i64,
    pub up: bool,
}

/// Breach alert emitted when uptime falls below target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlaBreach {
    pub uptime_percent: f64,
    pub target_percent: f64,
    pub detected_at: i64,
}

/// One row of the monthly report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthlyUptime {
    pub year: i32,
    pub month: u32,
    pub uptime_percent: f64,
    pub samples: usize,
    pub met_target: bool,
    /// Change in uptime versus the previous month, in percentage points.
    pub trend: Option<f64>,
}

/// Accumulates probe samples and evaluates them against an SLA.
#[derive(Debug, Default)]
pub struct SlaTracker {
    samples: VecDeque<ProbeSample>,
    /// Samples are kept for at most this long (seconds); 0 keeps everything.
    retention_secs: i64,
}

impl SlaTracker {
    pub fn new(retention_secs: i64) -> Self {
        Self {
            samples: VecDeque::new(),
            retention_secs,
        }
    }

    /// Record a probe result. Samples should arrive in timestamp order.
    pub fn record(&mut self, ts: i64, up: bool) {
        self.samples.push_back(ProbeSample { ts, up });
        if self.retention_secs > 0 {
            let cutoff = ts - self.retention_secs;
            while self.samples.front().is_some_and(|s| s.ts < cutoff) {
                self.samples.pop_front();
            }
        }
    }

    /// Time-weighted uptime (percent) over `[now - window_secs, now]`.
    ///
    /// Each sample is considered valid until the next sample (or `now` for the
    /// last one), so irregular probe intervals do not skew the result.
    pub fn uptime_percent(&self, now: i64, window_secs: i64) -> Option<f64> {
        let start = now - window_secs;
        let pts: Vec<&ProbeSample> = self.samples.iter().filter(|s| s.ts <= now).collect();
        weighted_uptime(&pts, start, now)
    }

    /// Returns a breach if uptime in the configured window is below target.
    pub fn check_breach(&self, cfg: &SlaConfig, now: i64) -> Option<SlaBreach> {
        let uptime = self.uptime_percent(now, cfg.window_secs)?;
        (uptime < cfg.target_percent).then_some(SlaBreach {
            uptime_percent: uptime,
            target_percent: cfg.target_percent,
            detected_at: now,
        })
    }

    /// Monthly uptime report (UTC calendar months) with month-over-month trend.
    pub fn monthly_report(&self, cfg: &SlaConfig, now: i64) -> Vec<MonthlyUptime> {
        let mut by_month: BTreeMap<(i32, u32), Vec<&ProbeSample>> = BTreeMap::new();
        for s in self.samples.iter().filter(|s| s.ts <= now) {
            if let Some(dt) = Utc.timestamp_opt(s.ts, 0).single() {
                by_month.entry((dt.year(), dt.month())).or_default().push(s);
            }
        }
        let mut out: Vec<MonthlyUptime> = Vec::new();
        for ((year, month), pts) in by_month {
            let start = month_start(year, month);
            let end = month_start_next(year, month).min(now);
            let Some(uptime) = weighted_uptime(&pts, start, end) else {
                continue;
            };
            let trend = out.last().map(|prev| uptime - prev.uptime_percent);
            out.push(MonthlyUptime {
                year,
                month,
                uptime_percent: uptime,
                samples: pts.len(),
                met_target: uptime >= cfg.target_percent,
                trend,
            });
        }
        out
    }
}

fn month_start(year: i32, month: u32) -> i64 {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .map(|d: DateTime<Utc>| d.timestamp())
        .unwrap_or(0)
}

fn month_start_next(year: i32, month: u32) -> i64 {
    if month == 12 {
        month_start(year + 1, 1)
    } else {
        month_start(year, month + 1)
    }
}

/// Time-weighted uptime over `[start, end]` from ordered samples.
fn weighted_uptime(pts: &[&ProbeSample], start: i64, end: i64) -> Option<f64> {
    if end <= start || pts.is_empty() {
        return None;
    }
    let mut up_secs = 0i64;
    let mut total = 0i64;
    for (i, s) in pts.iter().enumerate() {
        let seg_end = pts.get(i + 1).map_or(end, |n| n.ts).min(end);
        let seg_start = s.ts.max(start);
        if seg_end <= seg_start {
            continue;
        }
        let len = seg_end - seg_start;
        total += len;
        if s.up {
            up_secs += len;
        }
    }
    (total > 0).then(|| up_secs as f64 * 100.0 / total as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    fn feed(t: &mut SlaTracker, minutes: i64, down: std::ops::Range<i64>) {
        for m in 0..minutes {
            t.record(T0 + m * 60, !down.contains(&m));
        }
    }

    #[test]
    fn uptime_reflects_downtime() {
        let mut t = SlaTracker::new(0);
        feed(&mut t, 1000, 100..110); // 10 of 1000 minutes down
        let now = T0 + 1000 * 60;
        let u = t.uptime_percent(now, 1000 * 60).unwrap();
        assert!((u - 99.0).abs() < 0.01, "got {u}");
    }

    #[test]
    fn breach_detected_and_target_reconfigurable() {
        let mut t = SlaTracker::new(0);
        feed(&mut t, 1000, 100..110);
        let now = T0 + 1000 * 60;
        let strict = SlaConfig {
            target_percent: 99.9,
            window_secs: 1000 * 60,
        };
        let breach = t.check_breach(&strict, now).expect("should breach");
        assert!(breach.uptime_percent < 99.9);
        let relaxed = SlaConfig {
            target_percent: 98.0,
            ..strict
        };
        assert!(t.check_breach(&relaxed, now).is_none());
    }

    #[test]
    fn monthly_report_shows_trend() {
        let mut t = SlaTracker::new(0);
        // Jan 2024 fully up, Feb 2024 half down.
        let jan = month_start(2024, 1);
        let feb = month_start(2024, 2);
        let mar = month_start(2024, 3);
        t.record(jan, true);
        t.record(feb, true);
        t.record(feb + (mar - feb) / 2, false);
        let cfg = SlaConfig::default();
        let r = t.monthly_report(&cfg, mar);
        assert_eq!(r.len(), 2);
        assert!(r[0].met_target && r[0].trend.is_none());
        assert!((r[1].uptime_percent - 50.0).abs() < 0.01);
        assert!(!r[1].met_target);
        assert!(r[1].trend.unwrap() < -49.0);
    }

    #[test]
    fn empty_tracker_has_no_data() {
        let t = SlaTracker::default();
        assert!(t.uptime_percent(T0, 60).is_none());
        assert!(t.check_breach(&SlaConfig::default(), T0).is_none());
    }
}
