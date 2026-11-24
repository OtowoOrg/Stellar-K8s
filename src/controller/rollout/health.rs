//! Health checking for rollout gates
//!
//! Provides generic health-checking functionality for rollout gates,
//! particularly focused on ingestion lag monitoring for Horizon nodes.

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use reqwest;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::Result;

/// Configuration for rollout health checks
#[derive(Clone, Debug)]
pub struct RolloutHealthConfig {
    /// Maximum acceptable ingestion lag in ledgers
    pub max_ingestion_lag: u64,

    /// Timeout for health checks to complete
    pub health_check_timeout: Duration,

    /// Overall timeout before pausing the rollout
    pub rollout_pause_timeout: Duration,

    /// Interval between health checks
    pub check_interval: Duration,
}

impl Default for RolloutHealthConfig {
    fn default() -> Self {
        Self {
            max_ingestion_lag: 2,
            health_check_timeout: Duration::from_secs(5),
            rollout_pause_timeout: Duration::from_secs(180), // 3 minutes
            check_interval: Duration::from_secs(10),
        }
    }
}

/// Horizon health status for rollout gate purposes
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HorizonRolloutHealth {
    /// Whether the pod is healthy
    pub healthy: bool,

    /// Current ingestion lag in ledgers
    pub ingestion_lag: u64,

    /// Current ledger sequence
    pub current_ledger: u64,

    /// Core ledger sequence
    pub core_ledger: u64,

    /// Whether health threshold is met
    pub meets_threshold: bool,

    /// Human-readable status message
    pub message: String,
}

/// Performs health checks for rollout gates
pub struct RolloutHealthChecker;

impl RolloutHealthChecker {
    /// Check Horizon ingestion lag from a pod's health endpoint
    pub async fn check_horizon_ingestion_lag(
        pod_ip: &str,
        config: &RolloutHealthConfig,
    ) -> Result<HorizonRolloutHealth> {
        let url = format!("http://{pod_ip}:8000/health");

        debug!("Checking Horizon ingestion lag for pod at {}", pod_ip);

        let client = reqwest::Client::builder()
            .timeout(config.health_check_timeout)
            .build()
            .map_err(|e| crate::error::Error::ConfigError(format!("Failed to create HTTP client: {e}")))?;

        match client.get(&url).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    debug!("Health endpoint returned status: {}", response.status());
                    return Ok(HorizonRolloutHealth {
                        healthy: false,
                        ingestion_lag: u64::MAX,
                        current_ledger: 0,
                        core_ledger: 0,
                        meets_threshold: false,
                        message: format!("Health endpoint returned status {}", response.status()),
                    });
                }

                match response.json::<HorizonHealthResponse>().await {
                    Ok(health) => {
                        let ingestion_lag = health
                            .core_latest_ledger
                            .saturating_sub(health.history_latest_ledger);

                        let meets_threshold = ingestion_lag <= config.max_ingestion_lag;
                        let message = if meets_threshold {
                            format!(
                                "Horizon health check passed: lag {} <= {}",
                                ingestion_lag, config.max_ingestion_lag
                            )
                        } else {
                            format!(
                                "Horizon ingestion lag {} exceeds threshold {}",
                                ingestion_lag, config.max_ingestion_lag
                            )
                        };

                        debug!("{}", message);

                        Ok(HorizonRolloutHealth {
                            healthy: health.core_synced,
                            ingestion_lag,
                            current_ledger: health.history_latest_ledger,
                            core_ledger: health.core_latest_ledger,
                            meets_threshold,
                            message,
                        })
                    }
                    Err(e) => {
                        warn!("Failed to parse Horizon health response: {}", e);
                        Ok(HorizonRolloutHealth {
                            healthy: false,
                            ingestion_lag: u64::MAX,
                            current_ledger: 0,
                            core_ledger: 0,
                            meets_threshold: false,
                            message: format!("Failed to parse health response: {e}"),
                        })
                    }
                }
            }
            Err(e) => {
                warn!("Failed to query Horizon health endpoint: {}", e);
                Ok(HorizonRolloutHealth {
                    healthy: false,
                    ingestion_lag: u64::MAX,
                    current_ledger: 0,
                    core_ledger: 0,
                    meets_threshold: false,
                    message: format!("Cannot reach health endpoint: {e}"),
                })
            }
        }
    }
}

/// Horizon health response structure (matches Horizon API)
#[derive(Debug, Deserialize, Serialize)]
struct HorizonHealthResponse {
    #[serde(default)]
    pub status: String,

    #[serde(default)]
    pub core_latest_ledger: u64,

    #[serde(default)]
    pub history_latest_ledger: u64,

    #[serde(default)]
    pub core_synced: bool,

    #[serde(default)]
    pub history_elder_ledger: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RolloutHealthConfig::default();
        assert_eq!(config.max_ingestion_lag, 2);
        assert_eq!(config.health_check_timeout, Duration::from_secs(5));
        assert_eq!(config.rollout_pause_timeout, Duration::from_secs(180));
    }
}
