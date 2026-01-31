//! VSL (Validator Selection List) fetching logic
//!
//! Handles downloading and parsing of validator selection lists from trusted sources.

use crate::error::{Error, Result};
use reqwest;
use tracing::{debug, info};

/// Fetch and parse a VSL from a given URL
pub async fn fetch_vsl(url: &str) -> Result<String> {
    debug!("Fetching VSL from {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(url).send().await.map_err(Error::HttpError)?;

    if !response.status().is_success() {
        return Err(Error::ConfigError(format!(
            "Failed to fetch VSL from {}: status {}",
            url,
            response.status()
        )));
    }

    let content = response.text().await.map_err(Error::HttpError)?;

    info!("Successfully fetched VSL from {}", url);

    // For now we assume the source returns a TOML-formatted quorum set
    // In a more robust implementation, we might validate the format here.
    Ok(content)
}

/// Trigger a configuration reload in Stellar Core if it's already running
pub async fn trigger_config_reload(pod_ip: &str) -> Result<()> {
    let url = format!("http://{pod_ip}:11626/http-command?admin=true&command=config-reload");

    debug!("Triggering config-reload via {}", url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| Error::ConfigError(format!("Failed to build HTTP client: {e}")))?;

    let response = client.get(&url).send().await.map_err(Error::HttpError)?;

    if !response.status().is_success() {
        return Err(Error::ConfigError(format!(
            "Failed to trigger config-reload: status {}",
            response.status()
        )));
    }

    info!("Successfully triggered config-reload for pod at {}", pod_ip);
    Ok(())
}
