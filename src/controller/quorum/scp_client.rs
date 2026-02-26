//! HTTP client for querying Stellar Core SCP state

use super::error::{QuorumAnalysisError, Result};
use super::types::{PeerInfo, QuorumSetInfo, ScpState};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;
use tracing::{debug, warn};

/// Client for querying Stellar Core HTTP API
pub struct ScpClient {
    http_client: Client,
    #[allow(dead_code)]
    timeout: Duration,
}

impl ScpClient {
    /// Create a new SCP client with the specified timeout
    pub fn new(timeout: Duration) -> Self {
        Self {
            http_client: Client::builder()
                .timeout(timeout)
                .build()
                .expect("Failed to build HTTP client"),
            timeout,
        }
    }

    /// Query SCP state from a Stellar Core node
    /// 
    /// Endpoint: GET http://{pod_ip}:11626/scp?limit=1
    pub async fn query_scp_state(&self, pod_ip: &str) -> Result<ScpState> {
        let url = format!("http://{}:11626/scp?limit=1", pod_ip);
        debug!("Querying SCP state from {}", url);

        let response = self.retry_request(&url, 3).await?;
        let json: Value = response.json().await?;

        // Parse the SCP state from the response
        // The response is an array, we take the first element
        let scp_array = json.as_array()
            .ok_or_else(|| QuorumAnalysisError::ParseError("Expected array response".to_string()))?;
        
        if scp_array.is_empty() {
            return Err(QuorumAnalysisError::ParseError("Empty SCP state response".to_string()));
        }

        let scp_obj = &scp_array[0];
        
        // Extract node ID
        let node_id = scp_obj["node"]
            .as_str()
            .ok_or_else(|| QuorumAnalysisError::ParseError("Missing node field".to_string()))?
            .to_string();

        // Extract quorum set
        let qset = &scp_obj["qset"];
        let quorum_set: QuorumSetInfo = serde_json::from_value(qset.clone())?;

        // Extract ballot state
        let ballot_state = super::types::BallotState {
            phase: scp_obj["phase"]
                .as_str()
                .unwrap_or("UNKNOWN")
                .to_string(),
            ballot_counter: scp_obj["ballotCounter"]
                .as_u64()
                .unwrap_or(0) as u32,
            value_hash: scp_obj["valueHash"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        };

        // Extract nomination state
        let nomination_state = super::types::NominationState {
            votes: scp_obj["votes"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            accepted: scp_obj["accepted"]
                .as_array()
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
        };

        Ok(ScpState {
            node_id,
            quorum_set,
            ballot_state,
            nomination_state,
        })
    }

    /// Query quorum information for a specific node
    /// 
    /// Endpoint: GET http://{pod_ip}:11626/quorum?node={node_id}&compact=false
    pub async fn query_quorum_info(
        &self,
        pod_ip: &str,
        node_id: Option<&str>,
    ) -> Result<QuorumSetInfo> {
        let url = if let Some(id) = node_id {
            format!("http://{}:11626/quorum?node={}&compact=false", pod_ip, id)
        } else {
            format!("http://{}:11626/quorum?compact=false", pod_ip)
        };

        debug!("Querying quorum info from {}", url);

        let response = self.retry_request(&url, 3).await?;
        let json: Value = response.json().await?;

        // Extract the qset field
        let qset = &json["qset"];
        let quorum_set: QuorumSetInfo = serde_json::from_value(qset.clone())?;

        Ok(quorum_set)
    }

    /// Query peer information
    /// 
    /// Endpoint: GET http://{pod_ip}:11626/peers
    pub async fn query_peers(&self, pod_ip: &str) -> Result<Vec<PeerInfo>> {
        let url = format!("http://{}:11626/peers", pod_ip);
        debug!("Querying peers from {}", url);

        let response = self.retry_request(&url, 3).await?;
        let json: Value = response.json().await?;

        // Parse peers array
        let peers_array = json["authenticated_peers"]
            .as_array()
            .or_else(|| json["peers"].as_array())
            .ok_or_else(|| QuorumAnalysisError::ParseError("No peers array found".to_string()))?;

        let peers: Vec<PeerInfo> = peers_array
            .iter()
            .filter_map(|peer| {
                Some(PeerInfo {
                    id: peer["id"].as_str()?.to_string(),
                    address: peer["address"].as_str().unwrap_or("").to_string(),
                    state: peer["state"].as_str().unwrap_or("UNKNOWN").to_string(),
                })
            })
            .collect();

        Ok(peers)
    }

    /// Retry HTTP request with exponential backoff
    /// 
    /// Attempts: 3 times with delays of 1s, 2s, 4s
    async fn retry_request(&self, url: &str, max_attempts: u32) -> Result<reqwest::Response> {
        let mut attempt = 0;
        let mut last_error = None;

        while attempt < max_attempts {
            match self.http_client.get(url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        return Ok(response);
                    } else {
                        warn!(
                            "HTTP request to {} failed with status {}, attempt {}/{}",
                            url,
                            response.status(),
                            attempt + 1,
                            max_attempts
                        );
                        last_error = Some(QuorumAnalysisError::HttpError(
                            response.error_for_status().unwrap_err()
                        ));
                    }
                }
                Err(e) => {
                    warn!(
                        "HTTP request to {} failed: {}, attempt {}/{}",
                        url,
                        e,
                        attempt + 1,
                        max_attempts
                    );
                    last_error = Some(QuorumAnalysisError::HttpError(e));
                }
            }

            attempt += 1;
            if attempt < max_attempts {
                let delay = Duration::from_secs(2u64.pow(attempt - 1));
                debug!("Retrying after {:?}", delay);
                tokio::time::sleep(delay).await;
            }
        }

        Err(last_error.unwrap_or_else(|| {
            QuorumAnalysisError::ParseError("Max retries exceeded".to_string())
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scp_client_creation() {
        let client = ScpClient::new(Duration::from_secs(10));
        assert_eq!(client.timeout, Duration::from_secs(10));
    }
}
