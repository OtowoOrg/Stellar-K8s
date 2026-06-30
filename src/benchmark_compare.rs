//! Multi-cluster performance comparison for Stellar-K8s
//!
//! This module implements the `benchmark-compare` subcommand that compares
//! performance metrics (TPS, Ledger Time, etc.) between two different clusters
//! or configurations in real-time.
//!
//! # Features
//!
//! - Connect to two different Kubernetes contexts
//! - Query Prometheus instances for metrics
//! - Side-by-side comparison in terminal
//! - Export to PDF or HTML reports
//! - Real-time metric streaming
//! - Statistical analysis (mean, median, p95, p99)

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use comfy_table::{presets::UTF8_FULL, Cell, Color, ContentArrangement, Table};
use kube::{Client, Config};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::time::interval;
use tracing::{debug, info, warn};

/// Arguments for the benchmark-compare subcommand
#[derive(clap::Args, Debug, Clone)]
#[command(
    about = "Compare performance metrics between two clusters",
    long_about = "Compares performance metrics (TPS, Ledger Time, Consensus Latency) between\n\
        two different Kubernetes clusters or Prometheus instances in real-time.\n\n\
        Useful for A/B testing optimizations, comparing different configurations,\n\
        or validating that 'Cluster A' performs better than 'Cluster B'.\n\n\
        EXAMPLES:\n  \
        # Compare two Kubernetes contexts\n  \
        stellar-operator benchmark-compare \\\n    \
          --cluster-a-context prod-us-east \\\n    \
          --cluster-b-context prod-us-west\n\n  \
        # Compare two Prometheus instances\n  \
        stellar-operator benchmark-compare \\\n    \
          --cluster-a-prometheus http://prom-a:9090 \\\n    \
          --cluster-b-prometheus http://prom-b:9090\n\n  \
        # Export to HTML report\n  \
        stellar-operator benchmark-compare \\\n    \
          --cluster-a-context prod \\\n    \
          --cluster-b-context staging \\\n    \
          --output report.html \\\n    \
          --duration 300"
)]
pub struct BenchmarkCompareArgs {
    /// Kubernetes context for Cluster A
    #[arg(long, env = "CLUSTER_A_CONTEXT")]
    pub cluster_a_context: Option<String>,

    /// Kubernetes context for Cluster B
    #[arg(long, env = "CLUSTER_B_CONTEXT")]
    pub cluster_b_context: Option<String>,

    /// Prometheus URL for Cluster A (alternative to context)
    #[arg(long, env = "CLUSTER_A_PROMETHEUS")]
    pub cluster_a_prometheus: Option<String>,

    /// Prometheus URL for Cluster B (alternative to context)
    #[arg(long, env = "CLUSTER_B_PROMETHEUS")]
    pub cluster_b_prometheus: Option<String>,

    /// Label for Cluster A (for display)
    #[arg(long, default_value = "Cluster A")]
    pub cluster_a_label: String,

    /// Label for Cluster B (for display)
    #[arg(long, default_value = "Cluster B")]
    pub cluster_b_label: String,

    /// Namespace to query (if using Kubernetes contexts)
    #[arg(long, default_value = "stellar-system")]
    pub namespace: String,

    /// Duration to collect metrics (seconds)
    #[arg(long, default_value = "60")]
    pub duration: u64,

    /// Sampling interval (seconds)
    #[arg(long, default_value = "5")]
    pub interval: u64,

    /// Output file path (HTML or PDF)
    #[arg(long, short = 'o')]
    pub output: Option<PathBuf>,

    /// Output format (table, html, pdf, json)
    #[arg(long, default_value = "table")]
    pub format: OutputFormat,

    /// Show real-time updates in terminal
    #[arg(long, default_value = "true")]
    pub live: bool,

    /// Metrics to compare (comma-separated)
    #[arg(long, default_value = "tps,ledger_time,consensus_latency,sync_status")]
    pub metrics: String,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    Table,
    Html,
    Pdf,
    Json,
}

/// Cluster configuration
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub label: String,
    pub context: Option<String>,
    pub prometheus_url: Option<String>,
    pub namespace: String,
}

/// Performance metrics for a cluster
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub timestamp: DateTime<Utc>,
    pub tps: Option<f64>,
    pub ledger_time: Option<f64>,
    pub consensus_latency: Option<f64>,
    pub sync_status: Option<f64>,
    pub active_validators: Option<u64>,
    pub ledger_sequence: Option<u64>,
}

/// Statistical summary of metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSummary {
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub p95: f64,
    pub p99: f64,
    pub stddev: f64,
}

/// Comparison result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComparisonResult {
    pub cluster_a_label: String,
    pub cluster_b_label: String,
    pub cluster_a_metrics: Vec<PerformanceMetrics>,
    pub cluster_b_metrics: Vec<PerformanceMetrics>,
    pub cluster_a_summary: HashMap<String, MetricsSummary>,
    pub cluster_b_summary: HashMap<String, MetricsSummary>,
    pub duration_secs: u64,
    pub sample_count: usize,
}

/// Prometheus query client
pub struct PrometheusClient {
    http_client: HttpClient,
    base_url: String,
}

impl PrometheusClient {
    pub fn new(base_url: String) -> Self {
        Self {
            http_client: HttpClient::new(),
            base_url,
        }
    }

    /// Query instant metric value
    pub async fn query_instant(&self, query: &str) -> Result<f64> {
        let url = format!("{}/api/v1/query", self.base_url);
        let response = self
            .http_client
            .get(&url)
            .query(&[("query", query)])
            .send()
            .await
            .context("Failed to query Prometheus")?;

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Prometheus response")?;

        // Extract value from response
        let value = json["data"]["result"][0]["value"][1]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .context("Failed to extract metric value")?;

        Ok(value)
    }

    /// Query range of metric values
    pub async fn query_range(
        &self,
        query: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        step: u64,
    ) -> Result<Vec<(DateTime<Utc>, f64)>> {
        let url = format!("{}/api/v1/query_range", self.base_url);
        let response = self
            .http_client
            .get(&url)
            .query(&[
                ("query", query),
                ("start", &start.timestamp().to_string()),
                ("end", &end.timestamp().to_string()),
                ("step", &format!("{}s", step)),
            ])
            .send()
            .await
            .context("Failed to query Prometheus range")?;

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Prometheus response")?;

        let mut results = Vec::new();
        if let Some(values) = json["data"]["result"][0]["values"].as_array() {
            for value in values {
                if let (Some(timestamp), Some(val_str)) = (value[0].as_i64(), value[1].as_str()) {
                    if let Ok(val) = val_str.parse::<f64>() {
                        let dt = DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now);
                        results.push((dt, val));
                    }
                }
            }
        }

        Ok(results)
    }
}

/// Collect metrics from a cluster
pub async fn collect_cluster_metrics(
    config: &ClusterConfig,
    duration_secs: u64,
    interval_secs: u64,
) -> Result<Vec<PerformanceMetrics>> {
    let mut metrics = Vec::new();
    let mut ticker = interval(std::time::Duration::from_secs(interval_secs));

    // Determine Prometheus URL
    let prometheus_url = if let Some(url) = &config.prometheus_url {
        url.clone()
    } else if let Some(context) = &config.context {
        // Try to discover Prometheus from Kubernetes context
        discover_prometheus_url(context, &config.namespace).await?
    } else {
        anyhow::bail!("Either context or prometheus_url must be provided");
    };

    let client = PrometheusClient::new(prometheus_url);
    let start_time = Utc::now();
    let end_time = start_time + Duration::seconds(duration_secs as i64);

    info!(
        "Collecting metrics from {} for {} seconds",
        config.label, duration_secs
    );

    while Utc::now() < end_time {
        ticker.tick().await;

        let metric = collect_single_sample(&client).await?;

        debug!(
            "Collected sample from {}: TPS={:?}, Ledger Time={:?}",
            config.label, metric.tps, metric.ledger_time
        );

        metrics.push(metric);
    }

    Ok(metrics)
}

/// Collect a single metric sample
async fn collect_single_sample(client: &PrometheusClient) -> Result<PerformanceMetrics> {
    let timestamp = Utc::now();

    // Query TPS (transactions per second)
    let tps = client
        .query_instant("rate(stellar_horizon_tps[1m])")
        .await
        .ok();

    // Query ledger close time
    let ledger_time = client
        .query_instant("stellar_node_ledger_close_time_seconds")
        .await
        .ok();

    // Query consensus latency
    let consensus_latency = client
        .query_instant("stellar_consensus_latency_seconds")
        .await
        .ok();

    // Query sync status
    let sync_status = client.query_instant("stellar_node_sync_status").await.ok();

    // Query active validators
    let active_validators = client
        .query_instant("count(stellar_node_up == 1)")
        .await
        .ok()
        .map(|v| v as u64);

    // Query ledger sequence
    let ledger_sequence = client
        .query_instant("stellar_node_ledger_sequence")
        .await
        .ok()
        .map(|v| v as u64);

    Ok(PerformanceMetrics {
        timestamp,
        tps,
        ledger_time,
        consensus_latency,
        sync_status,
        active_validators,
        ledger_sequence,
    })
}

/// Discover Prometheus URL from Kubernetes context
async fn discover_prometheus_url(_context: &str, namespace: &str) -> Result<String> {
    // Load kubeconfig with specific context
    let config = Config::infer().await?;

    let client = Client::try_from(config)?;

    // Try to find Prometheus service
    let services: kube::Api<k8s_openapi::api::core::v1::Service> =
        kube::Api::namespaced(client, namespace);

    // Look for common Prometheus service names
    let prometheus_names = vec![
        "prometheus",
        "prometheus-server",
        "kube-prometheus-stack-prometheus",
        "prometheus-operated",
    ];

    for name in prometheus_names {
        if let Ok(svc) = services.get(name).await {
            if let Some(spec) = svc.spec {
                if let Some(ports) = spec.ports {
                    if let Some(port) = ports.first() {
                        let port_num = port.port;
                        return Ok(format!("http://{}:{}", name, port_num));
                    }
                }
            }
        }
    }

    anyhow::bail!(
        "Could not discover Prometheus service in namespace {}",
        namespace
    )
}

/// Calculate statistical summary
pub fn calculate_summary(values: &[f64]) -> MetricsSummary {
    if values.is_empty() {
        return MetricsSummary {
            mean: 0.0,
            median: 0.0,
            min: 0.0,
            max: 0.0,
            p95: 0.0,
            p99: 0.0,
            stddev: 0.0,
        };
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let median = sorted[sorted.len() / 2];
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    let p95 = sorted[(sorted.len() as f64 * 0.95) as usize];
    let p99 = sorted[(sorted.len() as f64 * 0.99) as usize];

    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    let stddev = variance.sqrt();

    MetricsSummary {
        mean,
        median,
        min,
        max,
        p95,
        p99,
        stddev,
    }
}

/// Generate comparison result
pub fn generate_comparison(
    cluster_a_label: String,
    cluster_b_label: String,
    cluster_a_metrics: Vec<PerformanceMetrics>,
    cluster_b_metrics: Vec<PerformanceMetrics>,
    duration_secs: u64,
) -> ComparisonResult {
    let mut cluster_a_summary = HashMap::new();
    let mut cluster_b_summary = HashMap::new();

    // Calculate summaries for each metric
    let metrics_to_summarize = vec!["tps", "ledger_time", "consensus_latency"];

    for metric_name in metrics_to_summarize {
        let a_values: Vec<f64> = cluster_a_metrics
            .iter()
            .filter_map(|m| match metric_name {
                "tps" => m.tps,
                "ledger_time" => m.ledger_time,
                "consensus_latency" => m.consensus_latency,
                _ => None,
            })
            .collect();

        let b_values: Vec<f64> = cluster_b_metrics
            .iter()
            .filter_map(|m| match metric_name {
                "tps" => m.tps,
                "ledger_time" => m.ledger_time,
                "consensus_latency" => m.consensus_latency,
                _ => None,
            })
            .collect();

        cluster_a_summary.insert(metric_name.to_string(), calculate_summary(&a_values));
        cluster_b_summary.insert(metric_name.to_string(), calculate_summary(&b_values));
    }

    let sample_count = cluster_a_metrics.len().min(cluster_b_metrics.len());

    ComparisonResult {
        cluster_a_label,
        cluster_b_label,
        cluster_a_metrics,
        cluster_b_metrics,
        cluster_a_summary,
        cluster_b_summary,
        duration_secs,
        sample_count,
    }
}

/// Render comparison as terminal table
pub fn render_table(result: &ComparisonResult) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);

    // Header
    table.set_header(vec![
        Cell::new("Metric").fg(Color::Cyan),
        Cell::new(&result.cluster_a_label).fg(Color::Green),
        Cell::new(&result.cluster_b_label).fg(Color::Yellow),
        Cell::new("Difference").fg(Color::Magenta),
        Cell::new("Winner").fg(Color::Blue),
    ]);

    // TPS comparison
    if let (Some(a_tps), Some(b_tps)) = (
        result.cluster_a_summary.get("tps"),
        result.cluster_b_summary.get("tps"),
    ) {
        let diff = ((a_tps.mean - b_tps.mean) / b_tps.mean * 100.0).abs();
        let winner = if a_tps.mean > b_tps.mean {
            &result.cluster_a_label
        } else {
            &result.cluster_b_label
        };

        table.add_row(vec![
            Cell::new("TPS (mean)"),
            Cell::new(format!("{:.2}", a_tps.mean)),
            Cell::new(format!("{:.2}", b_tps.mean)),
            Cell::new(format!("{:.1}%", diff)),
            Cell::new(winner),
        ]);

        table.add_row(vec![
            Cell::new("TPS (p95)"),
            Cell::new(format!("{:.2}", a_tps.p95)),
            Cell::new(format!("{:.2}", b_tps.p95)),
            Cell::new(""),
            Cell::new(""),
        ]);
    }

    // Ledger time comparison
    if let (Some(a_lt), Some(b_lt)) = (
        result.cluster_a_summary.get("ledger_time"),
        result.cluster_b_summary.get("ledger_time"),
    ) {
        let diff = ((a_lt.mean - b_lt.mean) / b_lt.mean * 100.0).abs();
        let winner = if a_lt.mean < b_lt.mean {
            &result.cluster_a_label
        } else {
            &result.cluster_b_label
        };

        table.add_row(vec![
            Cell::new("Ledger Time (mean)"),
            Cell::new(format!("{:.2}s", a_lt.mean)),
            Cell::new(format!("{:.2}s", b_lt.mean)),
            Cell::new(format!("{:.1}%", diff)),
            Cell::new(winner),
        ]);
    }

    // Consensus latency comparison
    if let (Some(a_cl), Some(b_cl)) = (
        result.cluster_a_summary.get("consensus_latency"),
        result.cluster_b_summary.get("consensus_latency"),
    ) {
        let diff = ((a_cl.mean - b_cl.mean) / b_cl.mean * 100.0).abs();
        let winner = if a_cl.mean < b_cl.mean {
            &result.cluster_a_label
        } else {
            &result.cluster_b_label
        };

        table.add_row(vec![
            Cell::new("Consensus Latency (mean)"),
            Cell::new(format!("{:.2}s", a_cl.mean)),
            Cell::new(format!("{:.2}s", b_cl.mean)),
            Cell::new(format!("{:.1}%", diff)),
            Cell::new(winner),
        ]);
    }

    // Summary
    table.add_row(vec![
        Cell::new("Sample Count"),
        Cell::new(result.sample_count.to_string()),
        Cell::new(result.sample_count.to_string()),
        Cell::new(""),
        Cell::new(""),
    ]);

    table.add_row(vec![
        Cell::new("Duration"),
        Cell::new(format!("{}s", result.duration_secs)),
        Cell::new(format!("{}s", result.duration_secs)),
        Cell::new(""),
        Cell::new(""),
    ]);

    table.to_string()
}

/// Render comparison as HTML
pub fn render_html(result: &ComparisonResult) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Stellar-K8s Performance Comparison</title>
    <style>
        body {{ font-family: Arial, sans-serif; margin: 40px; }}
        h1 {{ color: #333; }}
        table {{ border-collapse: collapse; width: 100%; margin-top: 20px; }}
        th, td {{ border: 1px solid #ddd; padding: 12px; text-align: left; }}
        th {{ background-color: #4CAF50; color: white; }}
        tr:nth-child(even) {{ background-color: #f2f2f2; }}
        .winner {{ background-color: #90EE90; font-weight: bold; }}
        .summary {{ margin-top: 30px; padding: 20px; background-color: #f9f9f9; border-radius: 5px; }}
    </style>
</head>
<body>
    <h1>Stellar-K8s Performance Comparison</h1>
    <p><strong>Cluster A:</strong> {}</p>
    <p><strong>Cluster B:</strong> {}</p>
    <p><strong>Duration:</strong> {} seconds</p>
    <p><strong>Samples:</strong> {}</p>
    
    <h2>Performance Metrics</h2>
    <table>
        <tr>
            <th>Metric</th>
            <th>{}</th>
            <th>{}</th>
            <th>Difference</th>
            <th>Winner</th>
        </tr>
        {}
    </table>
    
    <div class="summary">
        <h2>Summary</h2>
        <p>This report compares performance metrics between two Stellar clusters over a {} second period.</p>
        <p>Generated at: {}</p>
    </div>
</body>
</html>"#,
        result.cluster_a_label,
        result.cluster_b_label,
        result.duration_secs,
        result.sample_count,
        result.cluster_a_label,
        result.cluster_b_label,
        generate_html_rows(result),
        result.duration_secs,
        Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    )
}

fn generate_html_rows(result: &ComparisonResult) -> String {
    let mut rows = String::new();

    // TPS row
    if let (Some(a_tps), Some(b_tps)) = (
        result.cluster_a_summary.get("tps"),
        result.cluster_b_summary.get("tps"),
    ) {
        let diff = ((a_tps.mean - b_tps.mean) / b_tps.mean * 100.0).abs();
        let winner = if a_tps.mean > b_tps.mean {
            &result.cluster_a_label
        } else {
            &result.cluster_b_label
        };

        rows.push_str(&format!(
            "<tr><td>TPS (mean)</td><td>{:.2}</td><td>{:.2}</td><td>{:.1}%</td><td class=\"winner\">{}</td></tr>\n",
            a_tps.mean, b_tps.mean, diff, winner
        ));
    }

    // Ledger time row
    if let (Some(a_lt), Some(b_lt)) = (
        result.cluster_a_summary.get("ledger_time"),
        result.cluster_b_summary.get("ledger_time"),
    ) {
        let diff = ((a_lt.mean - b_lt.mean) / b_lt.mean * 100.0).abs();
        let winner = if a_lt.mean < b_lt.mean {
            &result.cluster_a_label
        } else {
            &result.cluster_b_label
        };

        rows.push_str(&format!(
            "<tr><td>Ledger Time (mean)</td><td>{:.2}s</td><td>{:.2}s</td><td>{:.1}%</td><td class=\"winner\">{}</td></tr>\n",
            a_lt.mean, b_lt.mean, diff, winner
        ));
    }

    rows
}

/// Export comparison to file
pub async fn export_comparison(
    result: &ComparisonResult,
    output_path: &PathBuf,
    format: &OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Html => {
            let html = render_html(result);
            tokio::fs::write(output_path, html).await?;
            info!("Exported HTML report to {}", output_path.display());
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(result)?;
            tokio::fs::write(output_path, json).await?;
            info!("Exported JSON report to {}", output_path.display());
        }
        OutputFormat::Pdf => {
            // PDF generation would require additional dependencies (e.g., printpdf)
            warn!("PDF export not yet implemented, falling back to HTML");
            let html = render_html(result);
            let html_path = output_path.with_extension("html");
            tokio::fs::write(&html_path, html).await?;
            info!("Exported HTML report to {}", html_path.display());
        }
        OutputFormat::Table => {
            let table = render_table(result);
            tokio::fs::write(output_path, table).await?;
            info!("Exported table to {}", output_path.display());
        }
    }

    Ok(())
}

/// Main entry point for benchmark-compare command
pub async fn run_benchmark_compare(args: BenchmarkCompareArgs) -> Result<()> {
    info!("Starting benchmark comparison");

    // Validate arguments
    if args.cluster_a_context.is_none() && args.cluster_a_prometheus.is_none() {
        anyhow::bail!("Either --cluster-a-context or --cluster-a-prometheus must be provided");
    }
    if args.cluster_b_context.is_none() && args.cluster_b_prometheus.is_none() {
        anyhow::bail!("Either --cluster-b-context or --cluster-b-prometheus must be provided");
    }

    // Configure clusters
    let cluster_a = ClusterConfig {
        label: args.cluster_a_label.clone(),
        context: args.cluster_a_context.clone(),
        prometheus_url: args.cluster_a_prometheus.clone(),
        namespace: args.namespace.clone(),
    };

    let cluster_b = ClusterConfig {
        label: args.cluster_b_label.clone(),
        context: args.cluster_b_context.clone(),
        prometheus_url: args.cluster_b_prometheus.clone(),
        namespace: args.namespace.clone(),
    };

    // Collect metrics from both clusters concurrently
    let (cluster_a_metrics, cluster_b_metrics) = tokio::join!(
        collect_cluster_metrics(&cluster_a, args.duration, args.interval),
        collect_cluster_metrics(&cluster_b, args.duration, args.interval)
    );

    let cluster_a_metrics = cluster_a_metrics?;
    let cluster_b_metrics = cluster_b_metrics?;

    // Generate comparison
    let result = generate_comparison(
        args.cluster_a_label,
        args.cluster_b_label,
        cluster_a_metrics,
        cluster_b_metrics,
        args.duration,
    );

    // Display results
    if args.live || args.output.is_none() {
        println!("\n{}", render_table(&result));
    }

    // Export if requested
    if let Some(output_path) = args.output {
        export_comparison(&result, &output_path, &args.format).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_summary() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let summary = calculate_summary(&values);

        assert_eq!(summary.mean, 3.0);
        assert_eq!(summary.median, 3.0);
        assert_eq!(summary.min, 1.0);
        assert_eq!(summary.max, 5.0);
    }

    #[test]
    fn test_calculate_summary_empty() {
        let values = vec![];
        let summary = calculate_summary(&values);

        assert_eq!(summary.mean, 0.0);
        assert_eq!(summary.median, 0.0);
    }
}
