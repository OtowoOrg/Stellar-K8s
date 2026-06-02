//! Pipeline monitoring, metrics, and error handling

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Prometheus-style metrics for the data pipeline
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PipelineMetrics {
    // Ingestion
    pub records_ingested_total: u64,
    pub records_dropped_total: u64,
    pub ingestion_lag_ms: u64,
    // ETL
    pub records_transformed_total: u64,
    pub transform_errors_total: u64,
    pub transform_duration_ms_p99: f64,
    // Quality
    pub quality_violations_total: u64,
    pub quality_pass_rate_pct: f64,
    // Warehouse
    pub records_written_total: u64,
    pub write_errors_total: u64,
    pub bytes_written_total: u64,
    // DLQ
    pub dlq_depth: u64,
    pub dlq_exhausted_total: u64,
    // Overall
    pub pipeline_uptime_secs: u64,
    pub last_ledger_processed: u64,
    pub last_updated: Option<DateTime<Utc>>,
}

impl PipelineMetrics {
    /// Render as Prometheus text format
    pub fn to_prometheus(&self) -> String {
        let ts = self.last_updated.map(|t| t.timestamp_millis()).unwrap_or(0);
        format!(
            "# HELP stellar_pipeline_records_ingested_total Total records ingested\n\
             # TYPE stellar_pipeline_records_ingested_total counter\n\
             stellar_pipeline_records_ingested_total {}\n\
             # HELP stellar_pipeline_records_dropped_total Total records dropped (buffer overflow)\n\
             # TYPE stellar_pipeline_records_dropped_total counter\n\
             stellar_pipeline_records_dropped_total {}\n\
             # HELP stellar_pipeline_transform_errors_total ETL transform errors\n\
             # TYPE stellar_pipeline_transform_errors_total counter\n\
             stellar_pipeline_transform_errors_total {}\n\
             # HELP stellar_pipeline_quality_pass_rate Data quality pass rate %%\n\
             # TYPE stellar_pipeline_quality_pass_rate gauge\n\
             stellar_pipeline_quality_pass_rate {:.2}\n\
             # HELP stellar_pipeline_dlq_depth Current dead-letter queue depth\n\
             # TYPE stellar_pipeline_dlq_depth gauge\n\
             stellar_pipeline_dlq_depth {}\n\
             # HELP stellar_pipeline_last_ledger Last processed ledger sequence\n\
             # TYPE stellar_pipeline_last_ledger gauge\n\
             stellar_pipeline_last_ledger {}\n",
            self.records_ingested_total,
            self.records_dropped_total,
            self.transform_errors_total,
            self.quality_pass_rate_pct,
            self.dlq_depth,
            self.last_ledger_processed,
        )
    }
}

/// Active pipeline monitor
pub struct PipelineMonitor {
    metrics: Arc<RwLock<PipelineMetrics>>,
    alert_thresholds: AlertThresholds,
}

#[derive(Clone, Debug)]
pub struct AlertThresholds {
    pub max_dlq_depth: u64,
    pub min_quality_pass_rate_pct: f64,
    pub max_ingestion_lag_ms: u64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            max_dlq_depth: 1_000,
            min_quality_pass_rate_pct: 95.0,
            max_ingestion_lag_ms: 30_000,
        }
    }
}

impl PipelineMonitor {
    pub fn new(thresholds: AlertThresholds) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(PipelineMetrics::default())),
            alert_thresholds: thresholds,
        }
    }

    pub async fn record_ingestion(&self, ingested: u64, dropped: u64, lag_ms: u64, ledger: u64) {
        let mut m = self.metrics.write().await;
        m.records_ingested_total += ingested;
        m.records_dropped_total += dropped;
        m.ingestion_lag_ms = lag_ms;
        m.last_ledger_processed = ledger;
        m.last_updated = Some(Utc::now());

        if lag_ms > self.alert_thresholds.max_ingestion_lag_ms {
            warn!(lag_ms, "ALERT: ingestion lag exceeds threshold");
        }
    }

    pub async fn record_transform(&self, transformed: u64, errors: u64) {
        let mut m = self.metrics.write().await;
        m.records_transformed_total += transformed;
        m.transform_errors_total += errors;
    }

    pub async fn record_quality(&self, pass_rate_pct: f64, violations: u64) {
        let mut m = self.metrics.write().await;
        m.quality_pass_rate_pct = pass_rate_pct;
        m.quality_violations_total += violations;

        if pass_rate_pct < self.alert_thresholds.min_quality_pass_rate_pct {
            warn!(
                pass_rate_pct,
                threshold = self.alert_thresholds.min_quality_pass_rate_pct,
                "ALERT: data quality pass rate below threshold"
            );
        }
    }

    pub async fn record_write(&self, rows: u64, bytes: u64, errors: u64) {
        let mut m = self.metrics.write().await;
        m.records_written_total += rows;
        m.bytes_written_total += bytes;
        m.write_errors_total += errors;
    }

    pub async fn record_dlq(&self, depth: u64, exhausted: u64) {
        let mut m = self.metrics.write().await;
        m.dlq_depth = depth;
        m.dlq_exhausted_total += exhausted;

        if depth > self.alert_thresholds.max_dlq_depth {
            warn!(depth, "ALERT: DLQ depth exceeds threshold");
        }
    }

    pub async fn metrics(&self) -> PipelineMetrics {
        self.metrics.read().await.clone()
    }

    pub async fn prometheus_metrics(&self) -> String {
        self.metrics.read().await.to_prometheus()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_record_ingestion_updates_metrics() {
        let mon = PipelineMonitor::new(AlertThresholds::default());
        mon.record_ingestion(100, 5, 1000, 500).await;
        let m = mon.metrics().await;
        assert_eq!(m.records_ingested_total, 100);
        assert_eq!(m.records_dropped_total, 5);
        assert_eq!(m.last_ledger_processed, 500);
    }

    #[tokio::test]
    async fn test_prometheus_output_contains_key_metrics() {
        let mon = PipelineMonitor::new(AlertThresholds::default());
        mon.record_ingestion(50, 0, 500, 100).await;
        let output = mon.prometheus_metrics().await;
        assert!(output.contains("stellar_pipeline_records_ingested_total 50"));
        assert!(output.contains("stellar_pipeline_last_ledger 100"));
    }
}
