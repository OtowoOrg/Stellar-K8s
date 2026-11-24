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
//! Flow ingestion for the egress synthesizer.
//!
//! Flows arrive either as in-memory [`ObservedFlow`] records or as Hubble
//! style JSON lines. The ingestor is the hot path of the synthesizer, so it
//! takes the lock once per batch and never allocates per record beyond the
//! record itself.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fmt;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// A single observed egress flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedFlow {
    /// Source address of the flow.
    pub src_ip: String,
    /// Destination address the workload talked to.
    pub dst_ip: String,
    /// Destination port the workload talked to.
    pub dst_port: u16,
    /// Transport protocol, normalised to upper case (`TCP`, `UDP`, `ICMP`).
    pub protocol: String,
    /// Namespace the flow originated from.
    pub namespace: String,
    /// Pod the flow originated from.
    pub pod_name: String,
    /// Number of bytes observed on the flow.
    pub bytes: u64,
    /// When the flow was observed.
    pub timestamp: DateTime<Utc>,
}

impl ObservedFlow {
    /// Build a flow record with the current timestamp.
    pub fn new(src_ip: &str, dst_ip: &str, dst_port: u16, protocol: &str) -> Self {
        Self {
            src_ip: src_ip.to_string(),
            dst_ip: dst_ip.to_string(),
            dst_port,
            protocol: protocol.to_ascii_uppercase(),
            namespace: String::new(),
            pod_name: String::new(),
            bytes: 0,
            timestamp: Utc::now(),
        }
    }

    /// Parse a flow record from a decoded JSON value.
    ///
    /// Both the nested Hubble layout (`source.ip`, `destination.port`) and a
    /// flat layout (`src_ip`, `dst_ip`, `dst_port`) are accepted so that
    /// operators can replay hand written captures.
    pub fn from_json(value: &Value) -> Result<Self, FlowParseError> {
        let source = value.get("source");
        let destination = value.get("destination");

        let src_ip = pick_str(value, source, &["src_ip", "source_ip"], "src_ip")?.to_string();
        let dst_ip =
            pick_str(value, destination, &["dst_ip", "destination_ip"], "dst_ip")?.to_string();

        let dst_port = pick_u16(
            value,
            destination,
            &["dst_port", "destination_port"],
            "dst_port",
        )?;
        let protocol = pick_str(
            value,
            None,
            &["protocol", "IPProtocol", "ip_protocol"],
            "protocol",
        )
        .unwrap_or("TCP")
        .to_ascii_uppercase();

        let namespace = pick_str(value, source, &["namespace"], "namespace")
            .unwrap_or_default()
            .to_string();
        let pod_name = pick_str(value, source, &["pod_name", "pod"], "pod_name")
            .unwrap_or_default()
            .to_string();

        let bytes = value
            .get("bytes")
            .or_else(|| value.get("Summary").and_then(|s| s.get("bytes")))
            .and_then(Value::as_u64)
            .unwrap_or(0);

        let timestamp = value
            .get("timestamp")
            .or_else(|| value.get("Time"))
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|parsed| parsed.with_timezone(&Utc))
            .unwrap_or_else(Utc::now);

        Ok(Self {
            src_ip,
            dst_ip,
            dst_port,
            protocol,
            namespace,
            pod_name,
            bytes,
            timestamp,
        })
    }

    /// Parse a flow record from a JSON line.
    pub fn from_line(line: &str) -> Result<Self, FlowParseError> {
        let value: Value = serde_json::from_str(line)
            .map_err(|err| FlowParseError::MalformedJson(err.to_string()))?;
        Self::from_json(&value)
    }
}

/// Error returned while parsing a flow record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowParseError {
    /// The line was not valid JSON.
    MalformedJson(String),
    /// A mandatory field was missing or of the wrong type.
    MissingField(&'static str),
}

impl fmt::Display for FlowParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedJson(details) => write!(f, "flow record is not valid JSON: {details}"),
            Self::MissingField(field) => write!(f, "flow record is missing field `{field}`"),
        }
    }
}

impl std::error::Error for FlowParseError {}

/// Statistics about one ingestion batch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IngestStats {
    /// Number of flows that were accepted.
    pub accepted: u64,
    /// Number of flow records that were rejected.
    pub rejected: u64,
    /// Total bytes seen in the accepted flows.
    pub total_bytes: u64,
    /// Wall clock time the batch took to ingest.
    pub elapsed: Duration,
}

impl IngestStats {
    /// Ingestion throughput in flows per second.
    pub fn flows_per_second(&self) -> f64 {
        let seconds = self.elapsed.as_secs_f64();
        if seconds <= 0.0 {
            return 0.0;
        }
        self.accepted as f64 / seconds
    }
}

#[derive(Debug, Default)]
struct IngestState {
    flows: Vec<ObservedFlow>,
    accepted: u64,
    rejected: u64,
    total_bytes: u64,
}

/// Ingests flow records and keeps them for clustering.
#[derive(Debug, Default)]
pub struct FlowIngestor {
    state: RwLock<IngestState>,
}

impl FlowIngestor {
    /// Create an empty ingestor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a batch of flow records.
    pub async fn ingest(&self, flows: Vec<ObservedFlow>) -> IngestStats {
        let started = Instant::now();
        let accepted = flows.len() as u64;
        let total_bytes = flows.iter().map(|flow| flow.bytes).sum();

        {
            let mut state = self.state.write().await;
            state.flows.extend(flows);
            state.accepted += accepted;
            state.total_bytes += total_bytes;
        }

        IngestStats {
            accepted,
            rejected: 0,
            total_bytes,
            elapsed: started.elapsed(),
        }
    }

    /// Ingest a batch of raw log lines, skipping the ones that cannot be
    /// parsed.
    pub async fn ingest_lines(&self, lines: &[String]) -> IngestStats {
        let started = Instant::now();
        let mut parsed = Vec::with_capacity(lines.len());
        let mut rejected = 0u64;

        for line in lines {
            match ObservedFlow::from_line(line) {
                Ok(flow) => parsed.push(flow),
                Err(_) => rejected += 1,
            }
        }

        let accepted = parsed.len() as u64;
        let total_bytes = parsed.iter().map(|flow| flow.bytes).sum();

        {
            let mut state = self.state.write().await;
            state.flows.extend(parsed);
            state.accepted += accepted;
            state.rejected += rejected;
            state.total_bytes += total_bytes;
        }

        IngestStats {
            accepted,
            rejected,
            total_bytes,
            elapsed: started.elapsed(),
        }
    }

    /// All flows observed so far.
    pub async fn flows(&self) -> Vec<ObservedFlow> {
        self.state.read().await.flows.clone()
    }

    /// Number of flows observed so far.
    pub async fn flow_count(&self) -> usize {
        self.state.read().await.flows.len()
    }

    /// Cumulative statistics of the ingestor.
    pub async fn stats(&self) -> IngestStats {
        let state = self.state.read().await;
        IngestStats {
            accepted: state.accepted,
            rejected: state.rejected,
            total_bytes: state.total_bytes,
            elapsed: Duration::from_secs(0),
        }
    }
}

fn pick_str<'a>(
    root: &'a Value,
    nested: Option<&'a Value>,
    keys: &[&str],
    field: &'static str,
) -> Result<&'a str, FlowParseError> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_str) {
            return Ok(value);
        }
    }
    if let Some(nested) = nested {
        for key in keys {
            if let Some(value) = nested.get(*key).and_then(Value::as_str) {
                return Ok(value);
            }
        }
    }
    Err(FlowParseError::MissingField(field))
}

fn pick_u16(
    root: &Value,
    nested: Option<&Value>,
    keys: &[&str],
    field: &'static str,
) -> Result<u16, FlowParseError> {
    for key in keys {
        if let Some(value) = root.get(*key).and_then(Value::as_u64) {
            return u16::try_from(value).map_err(|_| FlowParseError::MissingField(field));
        }
    }
    if let Some(nested) = nested {
        for key in keys {
            if let Some(value) = nested.get(*key).and_then(Value::as_u64) {
                return u16::try_from(value).map_err(|_| FlowParseError::MissingField(field));
            }
        }
    }
    Err(FlowParseError::MissingField(field))
}
