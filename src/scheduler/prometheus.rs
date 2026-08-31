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
//! Prometheus client for querying latency metrics
use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tracing::{debug, warn};

#[derive(Debug, Deserialize)]
struct PrometheusResponse {
    status: String,
    data: PrometheusData,
}

#[derive(Debug, Deserialize)]
struct PrometheusData {
    #[serde(rename = "resultType")]
    _result_type: String,
    result: Vec<PrometheusResult>,
}

#[derive(Debug, Deserialize)]
struct PrometheusResult {
    _metric: std::collections::HashMap<String, String>,
    value: (f64, String),
}

pub struct PrometheusClient {
    client: Client,
    url: String,
}

impl PrometheusClient {
    pub fn new(url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            url,
        }
    }

    /// Query average latency for a specific validator over a time window
    pub async fn get_validator_latency(
        &self,
        namespace: &str,
        name: &str,
        window: &str,
    ) -> Result<Option<f64>> {
        let query = format!(
            "avg_over_time(stellar_quorum_consensus_latency_ms{{namespace=\"{}\", name=\"{}\"}}[{}])",
            namespace, name, window
        );

        let url = format!("{}/api/v1/query", self.url);
        let response = self
            .client
            .get(&url)
            .query(&[("query", &query)])
            .send()
            .await?;

        if !response.status().is_success() {
            warn!("Prometheus query failed: {}", response.status());
            return Ok(None);
        }

        let resp: PrometheusResponse = response.json().await?;
        if resp.status != "success" || resp.data.result.is_empty() {
            return Ok(None);
        }

        // result[0].value is (timestamp, value_string)
        let value_str = &resp.data.result[0].value.1;
        let value: f64 = value_str.parse()?;

        debug!("Fetched latency for {}/{}: {}ms", namespace, name, value);
        Ok(Some(value))
    }
}
