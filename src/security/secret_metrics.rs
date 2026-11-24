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
//! Prometheus metrics for secret management.

#[cfg(feature = "metrics")]
use once_cell::sync::Lazy;
#[cfg(feature = "metrics")]
use prometheus_client::encoding::EncodeLabelSet;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::counter::Counter;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::family::Family;
#[cfg(feature = "metrics")]
use prometheus_client::metrics::gauge::Gauge;
#[cfg(feature = "metrics")]
use std::sync::atomic::{AtomicI64, AtomicU64};

#[cfg(feature = "metrics")]
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct SecretPolicyLabels {
    pub namespace: String,
    pub policy: String,
    pub provider: String,
}

#[cfg(feature = "metrics")]
pub static SECRET_ROTATIONS_TOTAL: Lazy<Family<SecretPolicyLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub static SECRET_SYNC_DRIFT: Lazy<Family<SecretPolicyLabels, Gauge<i64, AtomicI64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub static SECRET_ACCESS_ANOMALIES: Lazy<Family<SecretPolicyLabels, Counter<u64, AtomicU64>>> =
    Lazy::new(Family::default);

#[cfg(feature = "metrics")]
pub fn record_secret_rotation(namespace: &str, policy: &str, provider: &str) {
    let labels = SecretPolicyLabels {
        namespace: namespace.to_string(),
        policy: policy.to_string(),
        provider: provider.to_string(),
    };
    SECRET_ROTATIONS_TOTAL.get_or_create(&labels).inc();
}

#[cfg(feature = "metrics")]
pub fn record_sync_drift(namespace: &str, policy: &str, provider: &str, drift_count: i64) {
    let labels = SecretPolicyLabels {
        namespace: namespace.to_string(),
        policy: policy.to_string(),
        provider: provider.to_string(),
    };
    SECRET_SYNC_DRIFT.get_or_create(&labels).set(drift_count);
}

#[cfg(feature = "metrics")]
pub fn record_access_anomaly(namespace: &str, policy: &str, provider: &str) {
    let labels = SecretPolicyLabels {
        namespace: namespace.to_string(),
        policy: policy.to_string(),
        provider: provider.to_string(),
    };
    SECRET_ACCESS_ANOMALIES.get_or_create(&labels).inc();
}

#[cfg(not(feature = "metrics"))]
pub fn record_secret_rotation(_namespace: &str, _policy: &str, _provider: &str) {}
#[cfg(not(feature = "metrics"))]
pub fn record_sync_drift(_namespace: &str, _policy: &str, _provider: &str, _drift_count: i64) {}
#[cfg(not(feature = "metrics"))]
pub fn record_access_anomaly(_namespace: &str, _policy: &str, _provider: &str) {}
