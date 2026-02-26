use axum::{routing::get, Router};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

use stellar_k8s::ebpf::EbpfManager;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    info!("Starting Stellar eBPF Agent");

    let mut manager = EbpfManager::new()?;
    
    // In a real K8s environment, we would iterate over veth interfaces
    // of pods we want to protect. For this demonstration, we'll look for
    // eth0 or similar.
    let iface = std::env::var("IFACE").unwrap_or_else(|_| "eth0".to_string());
    manager.attach(&iface)?;

    let manager = Arc::new(Mutex::new(manager));
    let m_clone = manager.clone();

    let app = Router::new().route("/metrics", get(move || {
        let m = m_clone.clone();
        async move {
            let guard = m.lock().await;
            if let Ok(metrics) = guard.get_metrics() {
                format!(
                    "# HELP stellar_ebpf_allowed_packets_total Total packets allowed\n\
                     # TYPE stellar_ebpf_allowed_packets_total counter\n\
                     stellar_ebpf_allowed_packets_total {}\n\
                     # HELP stellar_ebpf_rejected_packets_total Total packets rejected\n\
                     # TYPE stellar_ebpf_rejected_packets_total counter\n\
                     stellar_ebpf_rejected_packets_total {}\n\
                     # HELP stellar_ebpf_bytes_total Total bytes processed\n\
                     # TYPE stellar_ebpf_bytes_total counter\n\
                     stellar_ebpf_bytes_total {}\n",
                    metrics.allowed_packets,
                    metrics.rejected_packets,
                    metrics.total_bytes
                )
            } else {
                "Error fetching metrics".to_string()
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:9101").await?;
    info!("Metrics server listening on http://0.0.0.0:9101/metrics");
    axum::serve(listener, app).await?;

    Ok(())
}
