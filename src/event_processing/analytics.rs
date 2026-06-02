//! Event analytics and visualization.
//!
//! Tracks per-type counters, rates, and latency histograms, and can render
//! an HTML report for human consumption.

use crate::event_processing::schema::{EventSeverity, ProcessingEvent};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// ── Metrics ───────────────────────────────────────────────────────────────────

/// Per-event-type statistics
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EventTypeStats {
    pub event_type: String,
    pub total_count: u64,
    pub error_count: u64,
    pub warning_count: u64,
    /// Sum of processing latency in microseconds (for average calculation)
    pub latency_sum_us: u64,
    pub latency_count: u64,
    pub last_seen: Option<DateTime<Utc>>,
}

impl EventTypeStats {
    fn record(&mut self, event: &ProcessingEvent, latency_us: u64) {
        self.total_count += 1;
        self.latency_sum_us += latency_us;
        self.latency_count += 1;
        self.last_seen = Some(event.timestamp);
        match event.severity {
            EventSeverity::Error | EventSeverity::Critical => self.error_count += 1,
            EventSeverity::Warning => self.warning_count += 1,
            _ => {}
        }
    }

    pub fn avg_latency_us(&self) -> f64 {
        if self.latency_count == 0 {
            0.0
        } else {
            self.latency_sum_us as f64 / self.latency_count as f64
        }
    }
}

/// Aggregate analytics snapshot
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticsSnapshot {
    pub taken_at: DateTime<Utc>,
    pub total_events: u64,
    pub by_type: Vec<EventTypeStats>,
    pub top_sources: Vec<(String, u64)>,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct AnalyticsEngine {
    by_type: RwLock<HashMap<String, EventTypeStats>>,
    by_source: RwLock<HashMap<String, u64>>,
    total: RwLock<u64>,
}

impl AnalyticsEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            by_type: RwLock::new(HashMap::new()),
            by_source: RwLock::new(HashMap::new()),
            total: RwLock::new(0),
        })
    }

    /// Record an event with an optional processing latency.
    pub async fn record(&self, event: &ProcessingEvent, latency_us: u64) {
        *self.total.write().await += 1;

        let mut by_type = self.by_type.write().await;
        let stats = by_type
            .entry(event.event_type.clone())
            .or_insert_with(|| EventTypeStats {
                event_type: event.event_type.clone(),
                ..Default::default()
            });
        stats.record(event, latency_us);

        let mut by_source = self.by_source.write().await;
        *by_source.entry(event.source.to_string()).or_insert(0) += 1;
    }

    /// Take a snapshot of current analytics.
    pub async fn snapshot(&self) -> AnalyticsSnapshot {
        let by_type = self.by_type.read().await;
        let by_source = self.by_source.read().await;
        let total = *self.total.read().await;

        let mut by_type_vec: Vec<_> = by_type.values().cloned().collect();
        by_type_vec.sort_by(|a, b| b.total_count.cmp(&a.total_count));

        let mut top_sources: Vec<_> = by_source.iter().map(|(k, v)| (k.clone(), *v)).collect();
        top_sources.sort_by(|a, b| b.1.cmp(&a.1));
        top_sources.truncate(10);

        AnalyticsSnapshot {
            taken_at: Utc::now(),
            total_events: total,
            by_type: by_type_vec,
            top_sources,
        }
    }

    /// Render an HTML analytics report.
    pub async fn render_html_report(&self) -> String {
        let snap = self.snapshot().await;
        render_html(&snap)
    }
}

impl Default for AnalyticsEngine {
    fn default() -> Self {
        Self {
            by_type: RwLock::new(HashMap::new()),
            by_source: RwLock::new(HashMap::new()),
            total: RwLock::new(0),
        }
    }
}

// ── HTML renderer ─────────────────────────────────────────────────────────────

fn render_html(snap: &AnalyticsSnapshot) -> String {
    let rows: String = snap
        .by_type
        .iter()
        .map(|s| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{}</td></tr>",
                s.event_type,
                s.total_count,
                s.error_count,
                s.warning_count,
                s.avg_latency_us(),
                s.last_seen.map(|t| t.to_rfc3339()).unwrap_or_default(),
            )
        })
        .collect();

    let source_rows: String = snap
        .top_sources
        .iter()
        .map(|(src, cnt)| format!("<tr><td>{src}</td><td>{cnt}</td></tr>"))
        .collect();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8"/>
  <title>Stellar-K8s Event Analytics</title>
  <style>
    body {{ font-family: sans-serif; margin: 2rem; }}
    h1 {{ color: #1a73e8; }}
    table {{ border-collapse: collapse; width: 100%; margin-bottom: 2rem; }}
    th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
    th {{ background: #f2f2f2; }}
    tr:nth-child(even) {{ background: #fafafa; }}
    .summary {{ background: #e8f0fe; padding: 1rem; border-radius: 4px; margin-bottom: 1rem; }}
  </style>
</head>
<body>
  <h1>Stellar-K8s Event Analytics</h1>
  <div class="summary">
    <strong>Report generated:</strong> {taken_at}<br/>
    <strong>Total events processed:</strong> {total}
  </div>

  <h2>Events by Type</h2>
  <table>
    <thead>
      <tr><th>Event Type</th><th>Total</th><th>Errors</th><th>Warnings</th><th>Avg Latency (µs)</th><th>Last Seen</th></tr>
    </thead>
    <tbody>{rows}</tbody>
  </table>

  <h2>Top Sources</h2>
  <table>
    <thead><tr><th>Source</th><th>Count</th></tr></thead>
    <tbody>{source_rows}</tbody>
  </table>
</body>
</html>"#,
        taken_at = snap.taken_at.to_rfc3339(),
        total = snap.total_events,
        rows = rows,
        source_rows = source_rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;

    #[tokio::test]
    async fn records_and_snapshots() {
        let engine = AnalyticsEngine::new();
        let ev = ProcessingEvent::new(
            "stellar.node.created",
            EventSource::Controller,
            "agg",
            "ns",
            serde_json::json!({}),
        );
        engine.record(&ev, 500).await;
        engine.record(&ev, 1000).await;

        let snap = engine.snapshot().await;
        assert_eq!(snap.total_events, 2);
        let stats = &snap.by_type[0];
        assert_eq!(stats.total_count, 2);
        assert!((stats.avg_latency_us() - 750.0).abs() < 1.0);
    }

    #[tokio::test]
    async fn html_report_contains_event_type() {
        let engine = AnalyticsEngine::new();
        let ev = ProcessingEvent::new(
            "stellar.disk.warning",
            EventSource::DiskScaler,
            "agg",
            "ns",
            serde_json::json!({}),
        );
        engine.record(&ev, 100).await;
        let html = engine.render_html_report().await;
        assert!(html.contains("stellar.disk.warning"));
    }
}
