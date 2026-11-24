//! Health probing for the Captive Core IPC endpoint.

use std::time::Duration;

use reqwest::Client;

/// Probe a Captive Core/RPC health endpoint with a bounded timeout.
pub async fn probe_http(client: &Client, endpoint: &str, timeout: Duration) -> Result<(), String> {
    client
        .get(endpoint)
        .timeout(timeout)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
