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
//! API deprecation timeline and stakeholder reporting (epic #1528).
//!
//! The timeline is built from the gateway's own deprecation metadata
//! ([`VersioningConfig`]) so every deprecated or sunset version is covered.
//! Consumer adoption is derived from runtime usage already collected by the
//! gateway ([`RequestEvent`]s from [`AnalyticsStore`](super::analytics::AnalyticsStore)),
//! never from self-reported status: a consumer (API key) counts as migrated
//! only when it sent no traffic to the deprecated version in the window.
//!
//! [`due_reminders`] yields one reminder per configured interval before the
//! sunset date, and [`render_csv`] / [`render_html`] / JSON (via `serde`)
//! export the report for leadership review.

use super::analytics::RequestEvent;
use super::config::VersioningConfig;
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Consumer id used for requests without an API key.
pub const ANONYMOUS_CONSUMER: &str = "anonymous";

/// Lifecycle state of a deprecated API version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeprecationState {
    Deprecated,
    Sunset,
}

/// Migration status of one consumer of a deprecated API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AdoptionStatus {
    /// Only the successor version was called in the window.
    Migrated,
    /// Both the deprecated and successor versions were called.
    InProgress,
    /// Only the deprecated version was called.
    NotStarted,
}

/// Usage of one consumer, taken directly from request events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerUsage {
    pub consumer: String,
    pub deprecated_requests: u64,
    pub successor_requests: u64,
    pub status: AdoptionStatus,
}

/// One row of the timeline.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    pub api_version: String,
    pub successor_version: String,
    pub state: DeprecationState,
    pub sunset_date: Option<NaiveDate>,
    /// Days until sunset relative to the report date (negative once past).
    pub days_remaining: Option<i64>,
    pub deprecated_requests: u64,
    pub successor_requests: u64,
    pub consumers: Vec<ConsumerUsage>,
    /// Migrated consumers / all consumers seen, in percent. `100` when no
    /// consumer was observed.
    pub adoption_pct: f64,
}

/// The full stakeholder report.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeprecationReport {
    pub generated_on: NaiveDate,
    pub entries: Vec<TimelineEntry>,
}

/// Build the timeline for every deprecated and sunset version in `cfg`,
/// overlaying adoption derived from `events`.
pub fn build_report(
    cfg: &VersioningConfig,
    events: &[RequestEvent],
    today: NaiveDate,
) -> DeprecationReport {
    let successor = cfg.current_version.as_str();
    let tracked: BTreeMap<&str, DeprecationState> = cfg
        .deprecated_versions
        .iter()
        .map(|v| (v.as_str(), DeprecationState::Deprecated))
        .chain(
            cfg.sunset_versions
                .iter()
                .map(|v| (v.as_str(), DeprecationState::Sunset)),
        )
        .collect();

    // (version, consumer) -> requests
    let mut counts: BTreeMap<(&str, &str), u64> = BTreeMap::new();
    for e in events {
        let consumer = e.api_key_id.as_deref().unwrap_or(ANONYMOUS_CONSUMER);
        *counts.entry((e.version.as_str(), consumer)).or_default() += 1;
    }
    let successor_by_consumer: BTreeMap<&str, u64> = counts
        .iter()
        .filter(|((v, _), _)| *v == successor)
        .map(|((_, c), n)| (*c, *n))
        .collect();

    let entries = tracked
        .into_iter()
        .map(|(version, state)| {
            let sunset_date = cfg
                .sunset_dates
                .get(version)
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
            let deprecated_by_consumer: BTreeMap<&str, u64> = counts
                .iter()
                .filter(|((v, _), _)| *v == version)
                .map(|((_, c), n)| (*c, *n))
                .collect();
            // Consumers of this API family: anyone who called the deprecated
            // version, plus successor callers (they may already have migrated).
            let all: BTreeSet<&str> = deprecated_by_consumer
                .keys()
                .chain(successor_by_consumer.keys())
                .copied()
                .collect();
            let consumers: Vec<ConsumerUsage> = all
                .into_iter()
                .map(|c| {
                    let dep = deprecated_by_consumer.get(c).copied().unwrap_or(0);
                    let succ = successor_by_consumer.get(c).copied().unwrap_or(0);
                    let status = match (dep, succ) {
                        (0, _) => AdoptionStatus::Migrated,
                        (_, 0) => AdoptionStatus::NotStarted,
                        _ => AdoptionStatus::InProgress,
                    };
                    ConsumerUsage {
                        consumer: c.to_string(),
                        deprecated_requests: dep,
                        successor_requests: succ,
                        status,
                    }
                })
                .collect();
            let migrated = consumers
                .iter()
                .filter(|c| c.status == AdoptionStatus::Migrated)
                .count();
            let adoption_pct = if consumers.is_empty() {
                100.0
            } else {
                migrated as f64 * 100.0 / consumers.len() as f64
            };
            TimelineEntry {
                api_version: version.to_string(),
                successor_version: successor.to_string(),
                state,
                sunset_date,
                days_remaining: sunset_date.map(|d| (d - today).num_days()),
                deprecated_requests: deprecated_by_consumer.values().sum(),
                successor_requests: successor_by_consumer.values().sum(),
                consumers,
                adoption_pct,
            }
        })
        .collect();

    DeprecationReport {
        generated_on: today,
        entries,
    }
}

/// Days-before-sunset at which reminders fire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderPolicy {
    pub intervals_days: Vec<i64>,
}

impl Default for ReminderPolicy {
    fn default() -> Self {
        Self {
            intervals_days: vec![90, 60, 30, 14, 7, 1],
        }
    }
}

/// A stakeholder reminder for one API at one interval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub api_version: String,
    pub interval_days: i64,
    pub days_remaining: i64,
    /// Consumers that still send traffic to the deprecated version.
    pub pending_consumers: Vec<String>,
}

impl Reminder {
    /// Plain-text body suitable for Slack/email notification channels.
    pub fn message(&self) -> String {
        format!(
            "API {} sunsets in {} day(s). {} consumer(s) still on it: {}",
            self.api_version,
            self.days_remaining,
            self.pending_consumers.len(),
            if self.pending_consumers.is_empty() {
                "none".to_string()
            } else {
                self.pending_consumers.join(", ")
            }
        )
    }
}

/// Reminders due as of the report date. Each `(api, interval)` fires once:
/// callers pass the pairs already sent in `sent` and record the returned ones.
/// An interval is due once `days_remaining <= interval`, so a missed run
/// catches up on the next one (only the tightest unsent interval fires).
pub fn due_reminders(
    report: &DeprecationReport,
    policy: &ReminderPolicy,
    sent: &HashSet<(String, i64)>,
) -> Vec<Reminder> {
    let mut out = Vec::new();
    for e in &report.entries {
        let Some(days) = e.days_remaining else {
            continue;
        };
        if e.state == DeprecationState::Sunset || days < 0 {
            continue;
        }
        let tightest = policy
            .intervals_days
            .iter()
            .copied()
            .filter(|i| days <= *i)
            .min();
        let Some(interval) = tightest else {
            continue;
        };
        if sent.contains(&(e.api_version.clone(), interval)) {
            continue;
        }
        out.push(Reminder {
            api_version: e.api_version.clone(),
            interval_days: interval,
            days_remaining: days,
            pending_consumers: e
                .consumers
                .iter()
                .filter(|c| c.status != AdoptionStatus::Migrated)
                .map(|c| c.consumer.clone())
                .collect(),
        });
    }
    out
}

/// CSV export: one row per (API, consumer).
pub fn render_csv(report: &DeprecationReport) -> String {
    let mut out = String::from(
        "api_version,state,sunset_date,days_remaining,adoption_pct,consumer,status,deprecated_requests,successor_requests\n",
    );
    for e in &report.entries {
        let sunset = e.sunset_date.map(|d| d.to_string()).unwrap_or_default();
        let days = e.days_remaining.map(|d| d.to_string()).unwrap_or_default();
        for c in &e.consumers {
            out.push_str(&format!(
                "{},{:?},{},{},{:.2},{},{:?},{},{}\n",
                e.api_version,
                e.state,
                sunset,
                days,
                e.adoption_pct,
                csv_field(&c.consumer),
                c.status,
                c.deprecated_requests,
                c.successor_requests
            ));
        }
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Self-contained HTML timeline for leadership review. Each API is a row
/// with an adoption bar; consumers are listed in an expandable section.
pub fn render_html(report: &DeprecationReport) -> String {
    let mut rows = String::new();
    for e in &report.entries {
        let mut consumers = String::new();
        for c in &e.consumers {
            consumers.push_str(&format!(
                "<li>{} — {:?} ({} deprecated / {} successor requests)</li>",
                html_escape(&c.consumer),
                c.status,
                c.deprecated_requests,
                c.successor_requests
            ));
        }
        rows.push_str(&format!(
            "<tr><td>{api}</td><td>{state:?}</td><td>{sunset}</td><td>{days}</td>\
             <td><div class=\"bar\"><span style=\"width:{pct:.1}%\"></span></div>{pct:.1}%</td>\
             <td><details><summary>{n} consumer(s)</summary><ul>{consumers}</ul></details></td></tr>",
            api = html_escape(&e.api_version),
            state = e.state,
            sunset = e.sunset_date.map(|d| d.to_string()).unwrap_or_else(|| "—".into()),
            days = e.days_remaining.map(|d| d.to_string()).unwrap_or_else(|| "—".into()),
            pct = e.adoption_pct,
            n = e.consumers.len(),
        ));
    }
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>API Deprecation Timeline</title>\
         <style>body{{font-family:sans-serif;margin:2rem}}table{{border-collapse:collapse;width:100%}}\
         td,th{{border:1px solid #ccc;padding:.4rem;text-align:left;vertical-align:top}}\
         .bar{{display:inline-block;width:120px;height:10px;background:#eee;margin-right:.5rem}}\
         .bar span{{display:block;height:100%;background:#2e7d32}}</style></head><body>\
         <h1>API Deprecation Timeline</h1><p>Generated {date}. Adoption is derived from gateway request telemetry.</p>\
         <table><tr><th>API</th><th>State</th><th>Sunset</th><th>Days left</th><th>Adoption</th><th>Consumers</th></tr>\
         {rows}</table></body></html>",
        date = report.generated_on,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg() -> VersioningConfig {
        VersioningConfig {
            current_version: "v3".into(),
            deprecated_versions: vec!["v2".into()],
            sunset_versions: vec!["v1".into()],
            sunset_dates: HashMap::from([
                ("v2".to_string(), "2026-12-31".to_string()),
                ("v1".to_string(), "2026-01-01".to_string()),
            ]),
            ..Default::default()
        }
    }

    fn ev(version: &str, key: Option<&str>) -> RequestEvent {
        RequestEvent {
            timestamp: "2026-09-01T00:00:00Z".into(),
            route_id: "r".into(),
            method: "GET".into(),
            path: format!("/api/{version}/nodes"),
            status: 200,
            latency_ms: 1,
            api_key_id: key.map(String::from),
            version: version.into(),
        }
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn timeline_covers_every_deprecated_and_sunset_api() {
        let r = build_report(&cfg(), &[], day("2026-09-24"));
        let apis: Vec<_> = r.entries.iter().map(|e| e.api_version.as_str()).collect();
        assert_eq!(apis, vec!["v1", "v2"]);
        assert_eq!(r.entries[1].days_remaining, Some(98));
        assert_eq!(r.entries[1].adoption_pct, 100.0);
    }

    #[test]
    fn adoption_matches_raw_usage_exactly() {
        let mut events = Vec::new();
        events.extend((0..5).map(|_| ev("v2", Some("a")))); // not started
        events.extend((0..3).map(|_| ev("v2", Some("b")))); // in progress
        events.extend((0..4).map(|_| ev("v3", Some("b"))));
        events.extend((0..2).map(|_| ev("v3", Some("c")))); // migrated
        events.push(ev("v2", None)); // anonymous, not started

        let r = build_report(&cfg(), &events, day("2026-09-24"));
        let v2 = r.entries.iter().find(|e| e.api_version == "v2").unwrap();

        let raw_dep = events.iter().filter(|e| e.version == "v2").count() as u64;
        let raw_succ = events.iter().filter(|e| e.version == "v3").count() as u64;
        assert_eq!(v2.deprecated_requests, raw_dep);
        assert_eq!(v2.successor_requests, raw_succ);

        let status: HashMap<_, _> = v2
            .consumers
            .iter()
            .map(|c| (c.consumer.as_str(), c.status))
            .collect();
        assert_eq!(status["a"], AdoptionStatus::NotStarted);
        assert_eq!(status["b"], AdoptionStatus::InProgress);
        assert_eq!(status["c"], AdoptionStatus::Migrated);
        assert_eq!(status[ANONYMOUS_CONSUMER], AdoptionStatus::NotStarted);
        assert!((v2.adoption_pct - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reminders_fire_once_per_interval() {
        let events = vec![ev("v2", Some("a")), ev("v3", Some("c"))];
        let policy = ReminderPolicy::default();
        let mut sent = HashSet::new();
        let mut fired = Vec::new();

        let mut d = day("2026-09-01");
        while d <= day("2026-12-31") {
            let r = build_report(&cfg(), &events, d);
            for rem in due_reminders(&r, &policy, &sent) {
                assert_eq!(rem.pending_consumers, vec!["a".to_string()]);
                sent.insert((rem.api_version.clone(), rem.interval_days));
                fired.push(rem.interval_days);
            }
            d = d.succ_opt().unwrap();
        }
        assert_eq!(fired, vec![90, 60, 30, 14, 7, 1]);
    }

    #[test]
    fn missed_run_catches_up_with_tightest_interval() {
        let r = build_report(&cfg(), &[], day("2026-12-26"));
        let due = due_reminders(&r, &ReminderPolicy::default(), &HashSet::new());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].interval_days, 7);
        assert!(due[0].message().contains("sunsets in 5 day(s)"));
    }

    #[test]
    fn exports_contain_every_consumer() {
        let events = vec![ev("v2", Some("a,b")), ev("v3", Some("<c>"))];
        let r = build_report(&cfg(), &events, day("2026-09-24"));
        let csv = render_csv(&r);
        assert!(csv.contains("v2,Deprecated,2026-12-31,98,50.00,\"a,b\",NotStarted,1,0"));
        let html = render_html(&r);
        assert!(html.contains("&lt;c&gt;"));
        assert!(serde_json::to_string(&r)
            .unwrap()
            .contains("\"adoptionPct\""));
    }

    #[test]
    fn full_estate_report_is_fast() {
        let events: Vec<_> = (0..500_000)
            .map(|i| {
                ev(
                    if i % 3 == 0 { "v2" } else { "v3" },
                    Some(&format!("k{}", i % 1000)),
                )
            })
            .collect();
        let start = std::time::Instant::now();
        let r = build_report(&cfg(), &events, day("2026-09-24"));
        let _ = render_html(&r);
        assert!(start.elapsed() < std::time::Duration::from_secs(120));
    }
}
