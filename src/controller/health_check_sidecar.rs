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
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, error};

use super::peer_connectivity::{
    connectivity_message, peer_monitor_loop, PeerConnectivityReport, PeerEndpoint,
    DEFAULT_INTERVAL_SECS, DEFAULT_PROBE_TIMEOUT_SECS,
};

#[derive(Clone)]
pub struct HealthCheckState {
    pub core_url: String,
    pub core_version: String,
    pub archive_urls: Vec<String>,
    pub sync_status: Arc<RwLock<SyncStatus>>,
    /// Latest peer-connectivity round.
    ///
    /// `None` until the first probe round finishes. Absence is *not* a failure:
    /// nodes without configured peers, and sidecars started before the first
    /// round, must not be reported degraded.
    pub peer_connectivity: Arc<RwLock<Option<PeerConnectivityReport>>>,
}


impl HealthCheckState {
    /// Spawn the peer probe loop for `peers`.
    ///
    /// Returns without spawning when there is nothing to probe, so non-validator
    /// pods do not pay for a no-op loop.
    pub fn spawn_peer_monitor(&self, peers: Vec<PeerEndpoint>) {
        if peers.is_empty() {
            debug!("no peers configured; skipping peer connectivity monitor");
            return;
        }
        let interval = Duration::from_secs(DEFAULT_INTERVAL_SECS);
        let timeout = Duration::from_secs(DEFAULT_PROBE_TIMEOUT_SECS);
        let slot = Arc::clone(&self.peer_connectivity);
        tokio::spawn(peer_monitor_loop(peers, interval, timeout, slot));
    }
}

impl HealthCheckState {
    /// Build state with empty sync status and no peer data yet.
    pub fn new(core_url: impl Into<String>) -> Self {
        Self {
            core_url: core_url.into(),
            core_version: "v21.3.1".to_string(),
            archive_urls: Vec::new(),
            sync_status: Arc::new(RwLock::new(SyncStatus::default())),
            peer_connectivity: Arc::new(RwLock::new(None)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncStatus {
    pub is_synced: bool,
    pub ledger_num: u64,
    pub network_ledger: u64,
    pub last_check: i64,
    pub archive_compatible: bool,
    pub archive_error: Option<String>,
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            is_synced: false,
            ledger_num: 0,
            network_ledger: 0,
            last_check: 0,
            archive_compatible: true,
            archive_error: None,
        }
    }
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub synced: bool,
    pub ledger_num: u64,
    pub network_ledger: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_compatible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_error: Option<String>,
}

/// Body of the `/peers` endpoint.
#[derive(Serialize)]
pub struct PeerConnectivityResponse {
    /// `unknown` before the first round, then `ok` or `degraded`.
    pub status: String,
    /// Number of peers that answered.
    pub reachable: usize,
    /// Number of peers that did not answer.
    pub unreachable: usize,
    /// True only when peers are configured and all of them failed.
    pub degraded: bool,
    /// Actionable diagnostics for the unreachable peers.
    pub message: String,
    /// Per-peer address, port, outcome and last attempt time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<PeerConnectivityReport>,
}

pub fn create_router(state: HealthCheckState) -> Router {
    Router::new()
        .route("/healthz", get(liveness_handler))
        .route("/readyz", get(readiness_handler))
        .route("/archive-compatibility", get(archive_compatibility_handler))
        .route("/peers", get(peers_handler))
        .with_state(state)
}

async fn liveness_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    // Liveness: just check if the process is running and responding
    match check_core_alive(&state.core_url).await {
        Ok(_) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: "alive".to_string(),
                synced: false,
                ledger_num: 0,
                network_ledger: 0,
                archive_compatible: None,
                archive_error: None,
            }),
        ),
        Err(e) => {
            error!("Liveness check failed: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "dead".to_string(),
                    synced: false,
                    ledger_num: 0,
                    network_ledger: 0,
                    archive_compatible: None,
                    archive_error: None,
                }),
            )
        }
    }
}

async fn readiness_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    let sync_status = state.sync_status.read().await;

    // Incompatible archive version detected before catch-up starts
    if !sync_status.archive_compatible {
        let err_msg = sync_status
            .archive_error
            .clone()
            .unwrap_or_else(|| "Incompatible history archive version detected".to_string());
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: format!("archive_incompatible: {}", err_msg),
                synced: false,
                ledger_num: sync_status.ledger_num,
                network_ledger: sync_status.network_ledger,
                archive_compatible: Some(false),
                archive_error: Some(err_msg),
            }),
        );
    }

    let peer_report = state.peer_connectivity.read().await;
    let peers_unreachable = match peer_report.as_ref() {
        Some(report) => report.is_fully_degraded(),
        None => false,
    };

    if !sync_status.is_synced {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "syncing".to_string(),
                synced: false,
                ledger_num: sync_status.ledger_num,
                network_ledger: sync_status.network_ledger,
                archive_compatible: Some(true),
                archive_error: None,
            }),
        );
    }

    if peers_unreachable {
        // Synced but partitioned: a validator with no reachable peer cannot
        // complete SCP, so it is not actually ready to serve.
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "degraded".to_string(),
                synced: true,
                ledger_num: sync_status.ledger_num,
                network_ledger: sync_status.network_ledger,
                archive_compatible: Some(sync_status.archive_compatible),
                archive_error: sync_status.archive_error.clone(),
            }),
        );
    }

    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "ready".to_string(),
            synced: true,
            ledger_num: sync_status.ledger_num,
            network_ledger: sync_status.network_ledger,
            archive_compatible: Some(sync_status.archive_compatible),
            archive_error: sync_status.archive_error.clone(),
        }),
    )
}

async fn peers_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    let report = state.peer_connectivity.read().await;

    let body = match report.as_ref() {
        None => PeerConnectivityResponse {
            status: "unknown".to_string(),
            reachable: 0,
            unreachable: 0,
            degraded: false,
            message: "peer connectivity probe has not completed yet".to_string(),
            report: None,
        },
        Some(report) => {
            let degraded = report.is_fully_degraded();
            let status = match (report.peers.is_empty(), degraded) {
                (true, _) => "unknown",
                (false, true) => "degraded",
                (false, false) => "ok",
            };
            PeerConnectivityResponse {
                status: status.to_string(),
                reachable: report.reachable_count(),
                unreachable: report.unreachable_count(),
                degraded,
                message: connectivity_message(report),
                report: Some(report.clone()),
            }
        }
    };

    let code = if body.degraded {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (code, Json(body))
}

async fn archive_compatibility_handler(State(state): State<HealthCheckState>) -> impl IntoResponse {
    let sync_status = state.sync_status.read().await;
    (
        if sync_status.archive_compatible {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({
            "archiveCompatible": sync_status.archive_compatible,
            "archiveError": sync_status.archive_error,
            "coreVersion": state.core_version,
            "archiveUrls": state.archive_urls,
        })),
    )
}

async fn check_core_alive(core_url: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    // Try different endpoints based on the service type
    let endpoints = vec![
        format!("{}/info", core_url),   // Stellar Core
        format!("{}/", core_url),       // Horizon
        format!("{}/health", core_url), // Soroban RPC
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
        // 1. Check archive version compatibility if archives are configured
        if !state.archive_urls.is_empty() {
            let compat_results =
                crate::controller::archive_health::check_archives_version_compatibility(
                    &state.archive_urls,
                    &state.core_version,
                    Some(std::time::Duration::from_secs(5)),
                )
                .await;

            let mut sync_status = state.sync_status.write().await;
            if let Some(incompat) = compat_results.iter().find(|r| !r.is_compatible) {
                sync_status.archive_compatible = false;
                sync_status.archive_error = incompat.error.clone();
                error!(
                    "History archive version incompatibility: {}",
                    incompat.summary()
                );
            } else {
                sync_status.archive_compatible = true;
                sync_status.archive_error = None;
            }
        }

        // 2. Check sync status
        match fetch_sync_status(&client, &state.core_url).await {
            Ok(status) => {
                debug!(
                    "Sync status: ledger={}, network={}, synced={}",
                    status.ledger_num, status.network_ledger, status.is_synced
                );
                let mut current = state.sync_status.write().await;
                current.is_synced = status.is_synced;
                current.ledger_num = status.ledger_num;
                current.network_ledger = status.network_ledger;
                current.last_check = status.last_check;
            }
            Err(e) => {
                error!("Failed to fetch sync status: {}", e);
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

async fn fetch_sync_status(client: &reqwest::Client, core_url: &str) -> Result<SyncStatus, String> {
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

    let ledger_num = body["info"]["ledger"]["num"].as_u64().unwrap_or(0);

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
        archive_compatible: true,
        archive_error: None,
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

    let ledger_num = body["history_latest_ledger"].as_u64().unwrap_or(0);

    let network_ledger = body["core_latest_ledger"].as_u64().unwrap_or(0);

    // Consider synced if within 5 ledgers of core
    let is_synced = network_ledger > 0 && (network_ledger - ledger_num) <= 5;

    Ok(SyncStatus {
        is_synced,
        ledger_num,
        network_ledger,
        last_check: chrono::Utc::now().timestamp(),
        archive_compatible: true,
        archive_error: None,
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

    let network_ledger = body["latestLedger"].as_u64().unwrap_or(0);

    // For Soroban RPC, check if it's healthy and has recent ledger data
    let is_synced = body["status"].as_str() == Some("healthy") && network_ledger > 0;

    Ok(SyncStatus {
        is_synced,
        ledger_num,
        network_ledger,
        last_check: chrono::Utc::now().timestamp(),
        archive_compatible: true,
        archive_error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controller::peer_connectivity::PeerProbeResult;

    fn probe(address: &str, port: u16, reachable: bool) -> PeerProbeResult {
        PeerProbeResult {
            address: address.to_string(),
            port,
            reachable,
            error: None,
            last_attempt: 1_700_000_000,
            latency_ms: Some(1),
        }
    }

    fn ready_state() -> HealthCheckState {
        HealthCheckState::new("http://localhost:11626")
    }

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

    #[test]
    fn new_state_has_no_peer_data() {
        let state = ready_state();
        assert_eq!(state.core_url, "http://localhost:11626");
        assert!(!state.sync_status.try_read().expect("lock").is_synced);
        assert!(state.peer_connectivity.try_read().expect("lock").is_none());
    }

    #[tokio::test]
    async fn readiness_is_ok_when_synced_and_peer_data_absent() {
        let state = ready_state();
        *state.sync_status.write().await = SyncStatus {
            is_synced: true,
            ledger_num: 10,
            network_ledger: 11,
            last_check: 0,
        };

        let response = readiness_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readiness_is_degraded_when_every_peer_is_unreachable() {
        let state = ready_state();
        *state.sync_status.write().await = SyncStatus {
            is_synced: true,
            ledger_num: 10,
            network_ledger: 11,
            last_check: 0,
        };
        *state.peer_connectivity.write().await = Some(PeerConnectivityReport {
            peers: vec![probe("10.0.0.8", 11625, false)],
            last_check: 1_700_000_000,
            interval_secs: 30,
        });

        let response = readiness_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn readiness_stays_ok_when_only_some_peers_are_unreachable() {
        let state = ready_state();
        *state.sync_status.write().await = SyncStatus {
            is_synced: true,
            ledger_num: 10,
            network_ledger: 11,
            last_check: 0,
        };
        *state.peer_connectivity.write().await = Some(PeerConnectivityReport {
            peers: vec![
                probe("10.0.0.8", 11625, true),
                probe("10.0.0.9", 11625, false),
            ],
            last_check: 1_700_000_000,
            interval_secs: 30,
        });

        let response = readiness_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn peers_endpoint_reports_unknown_before_the_first_round() {
        let state = ready_state();
        let response = peers_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn peers_endpoint_reports_degraded_when_all_unreachable() {
        let state = ready_state();
        *state.peer_connectivity.write().await = Some(PeerConnectivityReport {
            peers: vec![probe("10.0.0.8", 11625, false)],
            last_check: 1_700_000_000,
            interval_secs: 30,
        });

        let response = peers_handler(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn spawning_a_monitor_without_peers_is_a_noop() {
        let state = ready_state();
        state.spawn_peer_monitor(Vec::new());
        assert!(state.peer_connectivity.try_read().expect("lock").is_none());
    }
}
