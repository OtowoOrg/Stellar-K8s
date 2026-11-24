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
//! Capacity Planning Metrics
//!
//! Integration with Prometheus and internal metrics for capacity analysis.

use once_cell::sync::Lazy;
use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use std::sync::atomic::AtomicI64;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct CapacityLabels {
    pub resource: String,
    pub node_type: String,
}

/// Projected resource exhaustion timestamp (Unix timestamp)
pub static CAPACITY_EXHAUSTION_PREDICTION: Lazy<Family<CapacityLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

/// Confidence score of capacity predictions (0-100)
pub static CAPACITY_PREDICTION_CONFIDENCE: Lazy<Family<CapacityLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

pub fn record_exhaustion_prediction(
    resource: &str,
    node_type: &str,
    timestamp: i64,
    confidence: i64,
) {
    let labels = CapacityLabels {
        resource: resource.to_string(),
        node_type: node_type.to_string(),
    };
    CAPACITY_EXHAUSTION_PREDICTION
        .get_or_create(&labels)
        .set(timestamp);
    CAPACITY_PREDICTION_CONFIDENCE
        .get_or_create(&labels)
        .set(confidence);
}
