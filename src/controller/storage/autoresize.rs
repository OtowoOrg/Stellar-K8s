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
//! Reconciles the parent `StellarNode` after its PVC has been auto-expanded.
//!
//! [`crate::controller::volume_resizer`] grows a PVC when the kubelet reports
//! the volume is filling up, but the size it writes lives only on the PVC. The
//! `StellarNode` keeps advertising the `spec.storage` size it was created with,
//! so anything reading the parent CR — dashboards, `kubectl describe`,
//! admission policy, or a downstream component sizing a restore — sees stale
//! capacity indefinitely.
//!
//! This module closes that loop. It derives an [`ObservedStorageStatus`] from
//! the live PVC and writes it to `status.observedStorage`, giving the parent a
//! truthful view of its own storage.
//!
//! The distinction that matters is between *requested* and *actual* capacity.
//! Patching the PVC spec only asks the storage provider to grow the volume;
//! `status.capacity` moves only once the provider has done it. Reporting the
//! requested size as though it were real would hide an expansion that silently
//! failed, so both are reported and `expansion_complete` says which is which.

use crate::controller::resources::resource_name;
use crate::controller::storage::metrics::{self, Sample, VolumeUsage};
use crate::crd::{ObservedStorageStatus, StellarNode};
use crate::error::{Error, Result};
use k8s_openapi::api::core::v1::PersistentVolumeClaim;
use kube::{
    api::{Api, Patch, PatchParams},
    Client, ResourceExt,
};
use serde_json::json;
use tracing::{debug, info, instrument, warn};

/// Annotation recording how many times a PVC has been auto-expanded.
const EXPANSION_COUNT_ANN: &str = "stellar.org/auto-expansion-count";

/// Annotation recording the Unix timestamp of the most recent expansion.
const LAST_EXPANSION_ANN: &str = "stellar.org/last-auto-expansion";

/// Applied when a node's volume was grown automatically.
pub const APPLY_FIELD_MANAGER: &str = "stellar-operator";

/// Resolve a PVC quantity such as `20Gi` into bytes.
///
/// Kubernetes quantities carry SI (decimal) and binary (power-of-two) suffixes,
/// and may carry an exponent for fractional values such as `1.5Gi`. Anything
/// unrecognised returns `None` rather than guessing, so a surprising value is
/// visible as absent instead of silently wrong.
pub fn parse_quantity_bytes(quantity: &str) -> Option<i64> {
    let trimmed = quantity.trim();
    if trimmed.is_empty() {
        return None;
    }

    let suffixes: &[(&str, f64)] = &[
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0_f64.powi(4)),
        ("Pi", 1024.0_f64.powi(5)),
        ("Ei", 1024.0_f64.powi(6)),
        ("k", 1_000.0),
        ("M", 1_000_000.0),
        ("G", 1_000_000_000.0),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
        ("m", 0.001),
    ];

    for (suffix, multiplier) in suffixes {
        if let Some(number) = trimmed.strip_suffix(suffix) {
            return number
                .trim()
                .parse::<f64>()
                .ok()
                .map(|value| (value * multiplier) as i64);
        }
    }

    // Plain byte counts, and the `e`/`E` exponent form.
    if let Some((mantissa, exponent)) = split_exponent(trimmed) {
        let base: f64 = mantissa.parse().ok()?;
        let exp: i32 = exponent.parse().ok()?;
        return Some((base * 10f64.powi(exp)) as i64);
    }

    trimmed.parse::<i64>().ok()
}

/// Split a trailing `e`/`E` exponent off a quantity, avoiding the `E` binary
/// suffix, which is handled above.
fn split_exponent(value: &str) -> Option<(&str, &str)> {
    let idx = value
        .char_indices()
        .skip(1)
        .find(|(_, c)| *c == 'e' || *c == 'E')?
        .0;
    let (mantissa, exponent) = value.split_at(idx);
    let exponent = &exponent[1..];
    if mantissa.contains(['K', 'M', 'G', 'T', 'P', 'i']) || exponent.is_empty() {
        return None;
    }
    Some((mantissa, exponent))
}

/// Build the observed storage status for a PVC.
///
/// Pure so the reconciliation rules can be tested without a cluster.
pub fn observed_status_from_pvc(
    claim_name: &str,
    pvc: &PersistentVolumeClaim,
    now_unix: i64,
) -> ObservedStorageStatus {
    let requested_bytes = pvc
        .spec
        .as_ref()
        .and_then(|spec| spec.resources.as_ref())
        .and_then(|res| res.requests.as_ref())
        .and_then(|req| req.get("storage"))
        .and_then(|quantity| parse_quantity_bytes(&quantity.0))
        .or_else(|| {
            // Fall back to the status request, which is populated once the
            // volume controller has admitted the claim.
            pvc.status
                .as_ref()
                .and_then(|status| status.capacity.as_ref())
                .and_then(|capacity| parse_quantity_bytes(&capacity.0))
        });

    let actual_capacity_bytes = pvc
        .status
        .as_ref()
        .and_then(|status| status.capacity.as_ref())
        .and_then(|capacity| parse_quantity_bytes(&capacity.0));

    let annotations = pvc.metadata.annotations.as_ref();
    let expansion_count = annotations
        .and_then(|a| a.get(EXPANSION_COUNT_ANN))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);

    let last_expanded_at = annotations
        .and_then(|a| a.get(LAST_EXPANSION_ANN))
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .map(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .flatten()
        .map(|dt| dt.to_rfc3339())
        .or_else(|| {
            // A node that has never been expanded has no timestamp to report.
            let _ = now_unix;
            None
        });

    // The expansion is complete only once the provider reports at least the
    // capacity that was requested. Anything less is still in flight, or failed.
    let expansion_complete = match (requested_bytes, actual_capacity_bytes) {
        (Some(requested), Some(actual)) => actual >= requested,
        // With no reported capacity there is nothing confirming completion, so
        // do not claim success.
        _ => false,
    };

    ObservedStorageStatus {
        claim_name: claim_name.to_string(),
        requested_bytes,
        actual_capacity_bytes,
        expansion_complete,
        expansion_count,
        last_expanded_at,
    }
}

/// Fetch the current kubelet-reported usage for a PVC.
///
/// Returns `None` when the metrics are unavailable. Callers must not treat that
/// as an empty volume.
pub async fn fetch_volume_usage(
    prometheus_endpoint: &str,
    namespace: &str,
    pvc_name: &str,
) -> Option<VolumeUsage> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;

    let query = |body: String| async move {
        let resp = client
            .get(format!("{prometheus_endpoint}/api/v1/query"))
            .query(&[("query", body)])
            .send()
            .await
            .ok()?;
        metrics::parse_sample(&resp.text().await.ok()?)
    };

    let used: Option<Sample> = query(metrics::used_bytes_query(namespace, pvc_name)).await;
    let capacity: Option<Sample> =
        query(metrics::capacity_bytes_query(namespace, pvc_name)).await;

    VolumeUsage::from_samples(used, capacity)
}

/// Reconcile `status.observedStorage` on a `StellarNode` from its data PVC.
///
/// Returns the status that is now recorded, or `None` when there is no PVC to
/// observe yet.
#[instrument(skip(self, node), fields(node = %node.name_any()))]
pub async fn reconcile_observed_storage(
    client: &Client,
    node: &StellarNode,
) -> Result<Option<ObservedStorageStatus>> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let claim_name = resource_name(node, "data");

    let pvcs: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &namespace);
    let pvc = match pvcs.get(&claim_name).await {
        Ok(pvc) => pvc,
        Err(_) => {
            debug!("PVC {} not found, cannot reflect storage state", claim_name);
            return Ok(None);
        }
    };

    let now = chrono::Utc::now().timestamp();
    let observed = observed_status_from_pvc(&claim_name, &pvc, now);

    // Skip the write when nothing changed, so a steady-state node does not
    // generate a status update on every poll of the autoscaler loop.
    if let Some(existing) = node.status.as_ref().and_then(|s| s.observed_storage.as_ref()) {
        if *existing == observed {
            debug!("{}: observed storage unchanged", node.name_any());
            return Ok(Some(observed));
        }
    }

    let patch = json!({
        "status": {
            "observedStorage": observed,
        }
    });

    let nodes: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
    nodes
        .patch_status(
            node.name_any(),
            &PatchParams::apply(APPLY_FIELD_MANAGER).force(),
            &Patch::Apply(&patch),
        )
        .await
        .map_err(Error::KubeError)?;

    if !observed.expansion_complete {
        warn!(
            "{}/{}: requested {} bytes but provider reports {} — expansion still in flight or did not apply",
            namespace,
            node.name_any(),
            observed.requested_bytes.unwrap_or(-1),
            observed.actual_capacity_bytes.unwrap_or(-1)
        );
    } else {
        info!(
            "{}/{}: observed storage {} bytes ({} expansion(s))",
            namespace,
            node.name_any(),
            observed.actual_capacity_bytes.unwrap_or(-1),
            observed.expansion_count
        );
    }

    Ok(Some(observed))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::PersistentVolumeClaimSpec;
    use k8s_openapi::api::core::v1::ResourceRequirements;
    use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    const GIB: i64 = 1_073_741_824;

    fn pvc_with(
        request: Option<&str>,
        capacity: Option<&str>,
        annotations: &[(&str, &str)],
    ) -> PersistentVolumeClaim {
        let mut requests = BTreeMap::new();
        if let Some(value) = request {
            requests.insert("storage".to_string(), Quantity::from(value));
        }

        PersistentVolumeClaim {
            metadata: ObjectMeta {
                annotations: annotations
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                ..Default::default()
            },
            spec: Some(PersistentVolumeClaimSpec {
                resources: Some(ResourceRequirements {
                    requests: Some(requests),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            status: capacity.map(|value| k8s_openapi::api::core::v1::PersistentVolumeClaimStatus {
                capacity: Some(BTreeMap::from([(
                    "storage".to_string(),
                    Quantity::from(value),
                )])),
                phase: None,
                access_modes: None,
                conditions: None,
            }),
        }
    }

    // ── Quantity parsing ─────────────────────────────────────────────────────

    #[test]
    fn parses_binary_suffixes() {
        assert_eq!(parse_quantity_bytes("10Gi"), Some(10 * GIB));
        assert_eq!(parse_quantity_bytes("20Gi"), Some(20 * GIB));
        assert_eq!(parse_quantity_bytes("1Ki"), Some(1024));
        assert_eq!(parse_quantity_bytes("1Mi"), Some(1_048_576));
        assert_eq!(parse_quantity_bytes("1Ti"), Some(1024 * 1024 * GIB));
    }

    #[test]
    fn parses_decimal_suffixes() {
        assert_eq!(parse_quantity_bytes("1k"), Some(1_000));
        assert_eq!(parse_quantity_bytes("5M"), Some(5_000_000));
        assert_eq!(parse_quantity_bytes("1G"), Some(1_000_000_000));
    }

    #[test]
    fn parses_plain_byte_counts() {
        assert_eq!(parse_quantity_bytes("1024"), Some(1024));
        assert_eq!(parse_quantity_bytes("  2048  "), Some(2048));
    }

    #[test]
    fn parses_fractional_quantities() {
        assert_eq!(parse_quantity_bytes("1.5Gi"), Some(3 * GIB / 2));
    }

    #[test]
    fn rejects_unparseable_quantities() {
        assert_eq!(parse_quantity_bytes(""), None);
        assert_eq!(parse_quantity_bytes("   "), None);
        assert_eq!(parse_quantity_bytes("abc"), None);
        assert_eq!(parse_quantity_bytes("Gi"), None);
    }

    // ── Status derivation ────────────────────────────────────────────────────

    #[test]
    fn reports_requested_and_actual_when_provider_has_caught_up() {
        let pvc = pvc_with(Some("20Gi"), Some("20Gi"), &[]);
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.claim_name, "core-data");
        assert_eq!(status.requested_bytes, Some(20 * GIB));
        assert_eq!(status.actual_capacity_bytes, Some(20 * GIB));
        assert!(status.expansion_complete);
    }

    #[test]
    fn expansion_is_incomplete_while_provider_lags_the_request() {
        // The 10 Gi volume from the issue, asked to grow to 20 Gi but not yet
        // resized by the provider.
        let pvc = pvc_with(Some("20Gi"), Some("10Gi"), &[("stellar.org/auto-expansion-count", "1")]);
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.requested_bytes, Some(20 * GIB));
        assert_eq!(status.actual_capacity_bytes, Some(10 * GIB));
        assert!(
            !status.expansion_complete,
            "a request the provider has not honoured must not read as complete"
        );
    }

    #[test]
    fn expansion_is_incomplete_when_provider_reports_nothing() {
        let pvc = pvc_with(Some("20Gi"), None, &[]);
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.requested_bytes, Some(20 * GIB));
        assert_eq!(status.actual_capacity_bytes, None);
        assert!(!status.expansion_complete);
    }

    #[test]
    fn reads_expansion_count_and_timestamp_from_annotations() {
        let pvc = pvc_with(
            Some("20Gi"),
            Some("20Gi"),
            &[
                ("stellar.org/auto-expansion-count", "3"),
                ("stellar.org/last-auto-expansion", "1727000000"),
            ],
        );
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.expansion_count, 3);
        assert_eq!(
            status.last_expanded_at.as_deref(),
            Some("2024-09-22T15:33:20+00:00")
        );
    }

    #[test]
    fn never_expanded_pvc_reports_zero_count_and_no_timestamp() {
        let pvc = pvc_with(Some("10Gi"), Some("10Gi"), &[]);
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.expansion_count, 0);
        assert_eq!(status.last_expanded_at, None);
    }

    #[test]
    fn unparseable_expansion_count_is_treated_as_zero() {
        let pvc = pvc_with(
            Some("10Gi"),
            Some("10Gi"),
            &[("stellar.org/auto-expansion-count", "not-a-number")],
        );
        let status = observed_status_from_pvc("core-data", &pvc, 0);
        assert_eq!(status.expansion_count, 0);
    }

    #[test]
    fn falls_back_to_status_capacity_when_spec_request_is_absent() {
        // Some admission paths set only status.capacity.
        let pvc = pvc_with(None, Some("10Gi"), &[]);
        let status = observed_status_from_pvc("core-data", &pvc, 0);

        assert_eq!(status.requested_bytes, Some(10 * GIB));
        assert!(status.expansion_complete);
    }

    #[test]
    fn reflects_the_issue_scenario_end_to_end() {
        // 10 Gi volume, expanded to the next tier (20 Gi) by the operator, with
        // the provider reporting the new size.
        let before = observed_status_from_pvc("core-data", &pvc_with(Some("10Gi"), Some("10Gi"), &[]), 0);
        let after = observed_status_from_pvc(
            "core-data",
            &pvc_with(
                Some("20Gi"),
                Some("20Gi"),
                &[
                    ("stellar.org/auto-expansion-count", "1"),
                    ("stellar.org/last-auto-expansion", "1727000000"),
                ],
            ),
            0,
        );

        assert_eq!(before.actual_capacity_bytes, Some(10 * GIB));
        assert_eq!(after.actual_capacity_bytes, Some(20 * GIB));
        assert_eq!(after.requested_bytes, Some(20 * GIB));
        assert!(after.expansion_complete);
        assert_ne!(before, after, "the parent status must change after an expansion");
    }
}
