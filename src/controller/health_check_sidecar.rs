use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct HealthCheckState {
    pub core_url: String,
    pub sync_status: Arc<RwLock<SyncStatus>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatus {
    pub is_synced: bool,
    pub ledger_num: u64,
    pub network_ledger: u64,
    pub last_check: i64,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            is_synced: false,
            ledger_num: 0,
            network_ledger: 0,
            last_check: 0,
        }
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub synced: bool,
    pub ledger_num: u64,
    pub network_ledger: u64,
}

pub fn create_router(state: HealthCheckState) -> Router {
    Router::new()
        .route("/healthz", get(liveness_handler))
        .route("/readyz", get(readiness_handler))
        .with_state(state)
}

async fn liveness_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    // Liveness: just check if the process is running and responding
    match check_core_alive(&state.core_url).await {
        Ok(_) => (StatusCode::OK, Json(HealthResponse {
            status: "alive".to_string(),
            synced: false,
            ledger_num: 0,
            network_ledger: 0,
        })),
        Err(e) => {
            error!("Liveness check failed: {}", e);
            (StatusCode::SERVICE_UNAVAILABLE, Json(HealthResponse {
                status: "dead".to_string(),
                synced: false,
                ledger_num: 0,
                network_ledger: 0,
            }))
        }
    }
}

async fn readiness_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    let sync_status = state.sync_status.read().await;
    
    if sync_status.is_synced {
        (StatusCode::OK, Json(HealthResponse {
            status: "ready".to_string(),
            synced: true,
            ledger_num: sync_status.ledger_num,
            network_ledger: sync_status.network_ledger,
        }))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, Json(HealthResponse {
            status: "syncing".to_string(),
            synced: false,
            ledger_num: sync_status.ledger_num,
            network_ledger: sync_status.network_ledger,
        }))
    }
}

async fn check_core_alive(core_url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    // Try different endpoints based on the service type
    let endpoints = vec![
        format!("{}/info", core_url),      // Stellar Core
        format!("{}/", core_url),          // Horizon
        format!("{}/health", core_url),    // Soroban RPC
    ];

    for url in endpoints {
        if client.get(&url).send().await.is_ok() {
            return Ok(());
        }
    }

    Err("No API endpoint responded".to_string())
}

pub async fn sync_monitor_loop(state: HealthCheckState) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    loop {
        match fetch_sync_status(&client, &state.core_url).await {
            Ok(status) => {
                debug!("Sync status: ledger={}, network={}, synced={}", 
                    status.ledger_num, status.network_ledger, status.is_synced);
                *state.sync_status.write().await = status;
            }
            Err(e) => {
                error!("Failed to fetch sync status: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn fetch_sync_status(
    client: &reqwest::Client,
    core_url: &str,
) -> Result<SyncStatus, String> {
    // Try Stellar Core API first (for validators)
    if let Ok(status) = fetch_stellar_core_status(client, core_url).await {
        return Ok(status);
    }

    // Try Horizon API (for horizon nodes)
    if let Ok(status) = fetch_horizon_status(client, core_url).await {
        return Ok(status);
    }

    // Try Soroban RPC API (for soroban nodes)
    if let Ok(status) = fetch_soroban_status(client, core_url).await {
        return Ok(status);
    }

    Err("Failed to fetch status from any API endpoint".to_string())
}

async fn fetch_stellar_core_status(
    client: &reqwest::Client,
    core_url: &str,
) -> Result<SyncStatus, String> {
    let url = format!("{}/info", core_url);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let ledger_num = body["info"]["ledger"]["num"]
        .as_u64()
        .unwrap_or(0);
    
    let network_ledger = body["info"]["network"]["ledgerVersion"]
        .as_u64()
        .unwrap_or(0);

    // Consider synced if within 10 ledgers of network
    let is_synced = network_ledger > 0 && (network_ledger - ledger_num) <= 10;

    Ok(SyncStatus {
        is_synced,
        ledger_num,
        network_ledger,
        last_check: chrono::Utc::now().timestamp(),
    })
}

async fn fetch_horizon_status(
    client: &reqwest::Client,
    horizon_url: &str,
) -> Result<SyncStatus, String> {
    let url = format!("{}/", horizon_url);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let ledger_num = body["history_latest_ledger"]
        .as_u64()
        .unwrap_or(0);
    
    let network_ledger = body["core_latest_ledger"]
        .as_u64()
        .unwrap_or(0);

    // Consider synced if within 5 ledgers of core
    let is_synced = network_ledger > 0 && (network_ledger - ledger_num) <= 5;

    Ok(SyncStatus {
        is_synced,
        ledger_num,
        network_ledger,
        last_check: chrono::Utc::now().timestamp(),
    })
}

async fn fetch_soroban_status(
    client: &reqwest::Client,
    soroban_url: &str,
) -> Result<SyncStatus, String> {
    let url = format!("{}/health", soroban_url);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let ledger_num = body["ledgerRetentionWindow"]["oldestLedger"]
        .as_u64()
        .unwrap_or(0);
    
    let network_ledger = body["latestLedger"]
        .as_u64()
        .unwrap_or(0);

    // For Soroban RPC, check if it's healthy and has recent ledger data
    let is_synced = body["status"].as_str() == Some("healthy") && network_ledger > 0;

    Ok(SyncStatus {
        is_synced,
        ledger_num,
        network_ledger,
        last_check: chrono::Utc::now().timestamp(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_default() {
        let status = SyncStatus::default();
        assert!(!status.is_synced);
        assert_eq!(status.ledger_num, 0);
    }

    #[test]
    fn test_sync_status_synced() {
        let status = SyncStatus {
            is_synced: true,
            ledger_num: 1000,
            network_ledger: 1005,
            last_check: 0,
        };
        assert!(status.is_synced);
    }
}
