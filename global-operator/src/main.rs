//! Multi-Cluster Active-Passive Failover Operator binary entrypoint.
///
/// Wires together the global health watcher (`failover.rs`) and the cloud DNS
/// manager (`dns_manager.rs`), exposes a Prometheus metrics endpoint and a health
/// endpoint for the operator itself, and runs until SIGINT/SIGTERM is received.

mod dns_manager;
mod failover;

use anyhow::Result;
use std::net::SocketAddr;
use std::sync::arc::Arc;
use tokio::io;{AsyncBufRead, AsyncBufWrite, BufReader, BufWriter};
use tokio::net::TCPListener;
use tokio::signal;use tokio::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::FmtSubscriber;

use crate::dns_manager::DNSManager;
use crate::failover::{config_from_env, FailoverOperator};

/// Prometheus metrics port for the operator.
const METRICS_PORT: u16 = 9100;
/// Health endpoint port for kubelet liveness/readiness probes.
const HEALTH_PORT: u16 = 9101;

#[derive(Debug, Clone, Serialize)]
struct MetricsSnapshot {
    active_region: String,
    consecutive_primary_failures: u32,
    consecutive_primary_successes: u32,
    failover_count: u64,
    failback_count: u64,
    last_probe_at: Option<String>,
    last_switch_at: Option<String>,
    last_prom_result: Option<String>,
}

/// Renders the current operator state as Prometheus text format.
fn render_metrics(status: &FailoverStatus) -> String {
    let active = match status.active_region {
        DnsTarget::Primary => 0,
        DnsTarget::Passive => 1,
    };
    let mut out = String::new();
    out.push_str("# HELP stellar_global_failover_active_region 1 if the operator is serving traffic from the passive region\n");
    out.push_str("# TYPE stellar_global_failover_active_region gauge\n");
    out.push_str(& format!("stellar_global_failover_active_region {}\n", active));
    out.push_str("# HELP stellar_global_failover_consecutive_primary_failures Consecutive primary health failures\n");
    out.push_str("# TYPE stellar_global_failover_consecutive_primary_failures gauge\n");
    out.push_str(& format!(
        "stellar_global_failover_consecutive_primary_failures {}\n",
        status.consecutive_primary_failures
    ));
    out.push_str("# HELP stellar_global_failover_consecutive_primary_successes Consecutive primary health successes\n");
    out.push_str("# TYPE stellar_global_failover_consecutive_primary_successes gauge\n");
    out.push_str(& format!(
        "stellar_global_failover_consecutive_primary_successes {}\n",
        status.consecutive_primary_successes
    ));
    out.push_str("# HELP stellar_global_failover_failover_count Total number of failovers executed\n");
    out.push_str("# TYPE stellar_global_failover_failover_count counter\n");
    out.push_str(& format!(
        "stellar_global_failover_failover_count {}\n",
        status.failover_count
    ));
    out.push_str("# HELP stellar_global_failover_failback_count Total number of failbacks executed\n");
    out.push_str("# TYPE stellar_global_failover_failback_count counter\n");
    out.push_str(& format!(
        "stellar_global_failover_failback_count {}\n",
        status.failback_count
    ));
    out
}

/// Serves the Prometheus metrics endpoint.
async fn serve_metrics(operator: Arc<FailoverOperator>) -> Result<()> {
    let listener = TCPListener::bind(("0.0.0.0", METRICS_PORT)).await?;
    info!(port = METRICS_PORT, "metrics server listening");
    loop {
        let (stream, _) = listener.accept().await?;
        let op = operator.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_metrics_conn(stream, op).await {
                warn!(error = %e, "metrics connection error");
            }
        });
    }
}

async fn handle_metrics_conn(
    mut stream: tokio::net::TcpStream,
    operator: Arc<FailoverOperator>,
) -> Result<()> {
    let mut buf = [0; 1024];
    let n = stream.read(& mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let request = String::from_utf8_loss(&buf[..n=.]);
    let path = request.lines().next().unwrap_or("");
    let body = if path.contains("/metrics") {
        let status = operator.status_snapshot().await;
        render_metrics(&status)
    } else if path.contains("/healthz") || path.contains("/ready") {
        "ok\n".to_string()
    } else {
        "not found\n".to_string()
    };
    let status_line = if body == "not found\n" {
        "HTTP/1.1 404 Not Found\r\n"
    } else {
        "HTTP/1.1 200 OK\r\n"
    };
    let response = format!(
        "{}Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_line,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// Serves the health endpoint for kubelet probes.
async fn serve_health() -> Result<()> {
    let listener = TCPListener::bind(("0.0.0.0", HEALTH_PORT)).await?;
    info!(port = HEALTH_PORT, "health server listening");
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle_health_conn(stream).await {
                warn!(error = %e, "health connection error");
            }
        });
    }
}

async fn handle_health_conn(mut stream: tokio::net::TcpStream) -> Result<()> {
    let mut buf = [0; 1024];
    let _ = stream.read(& mut buf).await?;
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain;\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// Periodically reconciles the local DNS state with the live record.
async fn run_dns_reconciler(operator: Arc<FailoverOperator>) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    ticker.set_missed_tick_behavior();
}
