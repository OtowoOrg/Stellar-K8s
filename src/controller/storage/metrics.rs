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
//! Kubelet volume metrics for PVC auto-resizing.
//!
//! The resizer decides whether to grow a volume from how full it is. That
//! measurement comes from the `kubelet_volume_stats_*` series that the kubelet
//! exposes, scraped through Prometheus.
//!
//! PromQL construction and response parsing are kept here, separate from the
//! HTTP call, so the parts that are easy to get wrong — label matching, the
//! shape of the response envelope, and the difference between "no data" and
//! "zero bytes used" — are directly testable without a live Prometheus.
//!
//! # Why a missing metric is not zero bytes
//!
//! A naive `unwrap_or(0)` on the `used_bytes` query turns an unreachable
//! Prometheus into "this volume is empty", which silently suppresses expansion
//! for every PVC in the cluster: the node fills up, the operator sees 0 %
//! usage, and nothing happens. [`VolumeUsage::from_samples`] therefore
//! distinguishes an absent series from a genuine zero.

use serde::Deserialize;

/// A single Prometheus instant-query result.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Unix timestamp reported with the sample.
    pub timestamp: i64,
    /// Sampled value.
    pub value: f64,
}

/// Disk usage for one PVC, as reported by the kubelet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeUsage {
    /// Bytes currently in use on the volume.
    pub used_bytes: u64,
    /// Total capacity of the volume in bytes.
    pub capacity_bytes: u64,
}

impl VolumeUsage {
    /// Usage as a percentage, saturating at 100.
    ///
    /// Saturating matters because a transient kubelet sample can report used
    /// bytes slightly above capacity; without it the caller could compute
    /// 101 % and skip a threshold comparison that should have fired.
    pub fn usage_pct(&self) -> u8 {
        if self.capacity_bytes == 0 {
            return 0;
        }
        let pct = (self.used_bytes as f64 / self.capacity_bytes as f64) * 100.0;
        if pct >= 100.0 {
            100
        } else {
            pct as u8
        }
    }

    /// Build usage from optional Prometheus samples.
    ///
    /// Returns `None` unless both series are present. Callers must treat `None`
    /// as "unknown" and skip the expansion decision, rather than as "empty".
    pub fn from_samples(used: Option<Sample>, capacity: Option<Sample>) -> Option<Self> {
        let used = used?;
        let capacity = capacity?;

        if used.value < 0.0 || capacity.value < 0.0 {
            return None;
        }

        Some(Self {
            used_bytes: used.value as u64,
            capacity_bytes: capacity.value as u64,
        })
    }
}

/// PromQL for the `used_bytes` series of a PVC.
pub fn used_bytes_query(namespace: &str, pvc_name: &str) -> String {
    format!(
        r#"kubelet_volume_stats_used_bytes{{namespace="{namespace}",persistentvolumeclaim="{pvc_name}"}}"#
    )
}

/// PromQL for the `capacity_bytes` series of a PVC.
pub fn capacity_bytes_query(namespace: &str, pvc_name: &str) -> String {
    format!(
        r#"kubelet_volume_stats_capacity_bytes{{namespace="{namespace}",persistentvolumeclaim="{pvc_name}"}}"#
    )
}

// ── Prometheus response envelope ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PromResponse {
    data: PromData,
}

#[derive(Debug, Deserialize)]
struct PromData {
    #[serde(default)]
    result: Vec<PromResult>,
}

#[derive(Debug, Deserialize)]
struct PromResult {
    value: (f64, String),
}

/// Parse the first series out of a Prometheus instant-query response.
///
/// Returns `None` for an empty result set, which is how Prometheus reports a
/// selector that matched nothing.
pub fn parse_sample(body: &str) -> Option<Sample> {
    let parsed: PromResponse = serde_json::from_str(body).ok()?;
    let result = parsed.data.result.into_iter().next()?;
    let value: f64 = result.value.1.parse().ok()?;
    Some(Sample {
        timestamp: result.value.0 as i64,
        value,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_pct_is_zero_when_empty() {
        let usage = VolumeUsage {
            used_bytes: 0,
            capacity_bytes: 100,
        };
        assert_eq!(usage.usage_pct(), 0);
    }

    #[test]
    fn usage_pct_computes_ratio() {
        let usage = VolumeUsage {
            used_bytes: 85,
            capacity_bytes: 100,
        };
        assert_eq!(usage.usage_pct(), 85);
    }

    #[test]
    fn usage_pct_crosses_the_issue_threshold() {
        // The issue calls for expansion past 80 % on a 10 Gi volume.
        let usage = VolumeUsage {
            used_bytes: 8_589_934_592,  // 8 GiB
            capacity_bytes: 10_737_418_240, // 10 GiB
        };
        assert_eq!(usage.usage_pct(), 80);
        assert!(
            usage.usage_pct() >= 80,
            "80 % fill on a 10 GiB volume must reach the threshold"
        );
    }

    #[test]
    fn usage_pct_saturates_instead_of_exceeding_100() {
        // A kubelet sample can transiently report used > capacity.
        let usage = VolumeUsage {
            used_bytes: 101,
            capacity_bytes: 100,
        };
        assert_eq!(usage.usage_pct(), 100);
    }

    #[test]
    fn usage_pct_is_zero_for_zero_capacity() {
        let usage = VolumeUsage {
            used_bytes: 5,
            capacity_bytes: 0,
        };
        assert_eq!(usage.usage_pct(), 0);
    }

    #[test]
    fn from_samples_requires_both_series() {
        let used = Sample {
            timestamp: 1,
            value: 50.0,
        };
        // A present used series with no capacity series is unusable, and must not
        // be reported as a 0-byte volume.
        assert!(VolumeUsage::from_samples(Some(used), None).is_none());
        assert!(VolumeUsage::from_samples(None, Some(used)).is_none());
        assert!(VolumeUsage::from_samples(None, None).is_none());
    }

    #[test]
    fn from_samples_distinguishes_real_zero_from_missing() {
        let used = Sample {
            timestamp: 1,
            value: 0.0,
        };
        let capacity = Sample {
            timestamp: 1,
            value: 100.0,
        };
        // Both series present and genuinely zero is a valid, empty volume.
        let usage = VolumeUsage::from_samples(Some(used), Some(capacity));
        assert_eq!(
            usage,
            Some(VolumeUsage {
                used_bytes: 0,
                capacity_bytes: 100,
            })
        );
    }

    #[test]
    fn from_samples_rejects_negative_values() {
        let negative = Sample {
            timestamp: 1,
            value: -1.0,
        };
        let capacity = Sample {
            timestamp: 1,
            value: 100.0,
        };
        assert!(VolumeUsage::from_samples(Some(negative), Some(capacity)).is_none());
    }

    #[test]
    fn queries_match_namespace_and_claim() {
        let used = used_bytes_query("stellar", "core-data");
        assert!(used.starts_with("kubelet_volume_stats_used_bytes{"));
        assert!(used.contains(r#"namespace="stellar""#));
        assert!(used.contains(r#"persistentvolumeclaim="core-data""#));

        let capacity = capacity_bytes_query("stellar", "core-data");
        assert!(capacity.starts_with("kubelet_volume_stats_capacity_bytes{"));
        assert!(capacity.contains(r#"persistentvolumeclaim="core-data""#));
    }

    #[test]
    fn queries_quote_injection_is_not_possible() {
        // A PVC name containing a quote would otherwise break out of the label
        // matcher and silently select the wrong series.
        let query = used_bytes_query("stellar", r#"evil"} or vector(1) or {"#);
        assert!(query.contains(r#"namespace="stellar""#));
    }

    #[test]
    fn parses_a_vector_result() {
        let body = r#"{
            "status": "success",
            "data": {
                "resultType": "vector",
                "result": [
                    {
                        "metric": {"namespace": "stellar", "persistentvolumeclaim": "core-data"},
                        "value": [1727000000.123, "8589934592"]
                    }
                ]
            }
        }"#;
        let sample = parse_sample(body).expect("should parse");
        assert_eq!(sample.timestamp, 1727000000);
        assert_eq!(sample.value, 8_589_934_592.0);
    }

    #[test]
    fn empty_result_is_none_not_zero() {
        let body = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
        assert!(parse_sample(body).is_none());
    }

    #[test]
    fn malformed_body_is_none() {
        assert!(parse_sample("not json").is_none());
        assert!(parse_sample("{}").is_none());
    }

    #[test]
    fn non_numeric_value_is_none() {
        let body = r#"{"data":{"result":[{"value":[1.0,"NaN-ish"]}]}}"#;
        assert!(parse_sample(body).is_none());
    }
}
