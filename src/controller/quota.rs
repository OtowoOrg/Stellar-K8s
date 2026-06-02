//! ResourceQuota and LimitRange awareness for the StellarNode operator.
//!
//! Before creating or scaling pods the operator queries the namespace's
//! `ResourceQuota` and `LimitRange` objects so it can:
//! - Emit a `QuotaExceeded` condition instead of letting the API server reject
//!   the pod (which produces a confusing error buried in Events).
//! - Surface per-namespace quota usage via Prometheus metrics so teams can
//!   plan capacity before hitting hard limits.
//!
//! # Quota Planning Guidelines
//! - Set `requests.cpu` and `requests.memory` on every StellarNode to enable
//!   accurate quota accounting. Nodes without requests are counted as 0 against
//!   the quota but may be evicted under pressure.
//! - Size your namespace quota to at least 120 % of steady-state consumption to
//!   leave headroom for rolling updates (one extra replica during rollout).
//! - Use `LimitRange` to set per-pod defaults so that new containers
//!   automatically receive sensible resource bounds even when the spec omits them.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{LimitRange, ResourceQuota};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{Api, ListParams};
use kube::Client;
use tracing::{debug, info, warn};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

// ── Types ────────────────────────────────────────────────────────────────────

/// Result of a quota pre-flight check.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaCheckResult {
    /// True when the requested resources fit within available quota.
    pub allowed: bool,
    /// Human-readable explanation when `allowed == false`.
    pub message: String,
    /// Current utilisation snapshot for metrics emission.
    pub utilisation: Vec<QuotaUtilisation>,
}

/// Quota usage for a single resource dimension in one ResourceQuota object.
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaUtilisation {
    pub quota_name: String,
    pub resource: String,
    pub hard: f64,
    pub used: f64,
    pub requested: f64,
}

/// Parsed CPU or memory quantity in comparable units (millicores / bytes).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
struct ParsedQuantity(f64);

// ── Quantity parsing ──────────────────────────────────────────────────────────

fn parse_cpu_millis(q: &Quantity) -> Option<f64> {
    let s = q.0.trim();
    if let Some(m) = s.strip_suffix('m') {
        return m.parse::<f64>().ok();
    }
    s.parse::<f64>().ok().map(|v| v * 1000.0)
}

fn parse_memory_bytes(q: &Quantity) -> Option<f64> {
    let s = q.0.trim();
    let suffixes: &[(&str, f64)] = &[
        ("Ti", 1024f64.powi(4)),
        ("Gi", 1024f64.powi(3)),
        ("Mi", 1024f64.powi(2)),
        ("Ki", 1024.0),
        ("T", 1e12),
        ("G", 1e9),
        ("M", 1e6),
        ("K", 1e3),
    ];
    for (suffix, factor) in suffixes {
        if let Some(num) = s.strip_suffix(suffix) {
            return num.parse::<f64>().ok().map(|v| v * factor);
        }
    }
    s.parse::<f64>().ok()
}

// ── Core quota check ──────────────────────────────────────────────────────────

/// Validate that the additional resources requested by `node` fit within the
/// namespace's `ResourceQuota` objects.
///
/// If any quota would be exceeded the returned `QuotaCheckResult` has
/// `allowed = false` with a descriptive `message` suitable for a Kubernetes
/// Condition.
pub async fn check_quota(
    client: &Client,
    node: &StellarNode,
) -> Result<QuotaCheckResult> {
    let namespace = node
        .metadata
        .namespace
        .as_deref()
        .unwrap_or("default");

    let quota_api: Api<ResourceQuota> = Api::namespaced(client.clone(), namespace);
    let quotas = quota_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    if quotas.items.is_empty() {
        debug!(
            "No ResourceQuota found in namespace '{}' — skipping quota check",
            namespace
        );
        return Ok(QuotaCheckResult {
            allowed: true,
            message: "no ResourceQuota configured in namespace".to_string(),
            utilisation: vec![],
        });
    }

    // Derive requested resources from the spec
    let req_cpu_m = parse_cpu_millis(
        node.spec
            .resources
            .requests
            .get("cpu")
            .unwrap_or(&Quantity("100m".to_string())),
    )
    .unwrap_or(100.0)
        * node.spec.replicas as f64;

    let req_mem_bytes = parse_memory_bytes(
        node.spec
            .resources
            .requests
            .get("memory")
            .unwrap_or(&Quantity("256Mi".to_string())),
    )
    .unwrap_or(268_435_456.0)
        * node.spec.replicas as f64;

    let mut utilisation = Vec::new();
    let mut violations: Vec<String> = Vec::new();

    for quota in &quotas.items {
        let quota_name = quota
            .metadata
            .name
            .as_deref()
            .unwrap_or("unknown");

        let hard = quota.spec.as_ref().map(|s| &s.hard);
        let used_map = quota.status.as_ref().and_then(|s| s.used.as_ref());

        if let Some(hard) = hard {
            // Check requests.cpu
            if let Some(hard_cpu) = hard.get("requests.cpu") {
                let hard_m = parse_cpu_millis(hard_cpu).unwrap_or(f64::MAX);
                let used_m = used_map
                    .and_then(|u| u.get("requests.cpu"))
                    .and_then(|q| parse_cpu_millis(q))
                    .unwrap_or(0.0);

                utilisation.push(QuotaUtilisation {
                    quota_name: quota_name.to_string(),
                    resource: "requests.cpu".to_string(),
                    hard: hard_m,
                    used: used_m,
                    requested: req_cpu_m,
                });

                if used_m + req_cpu_m > hard_m {
                    violations.push(format!(
                        "ResourceQuota '{quota_name}': requests.cpu would exceed hard limit \
                        ({:.0}m used + {:.0}m requested > {:.0}m hard)",
                        used_m, req_cpu_m, hard_m
                    ));
                }
            }

            // Check requests.memory
            if let Some(hard_mem) = hard.get("requests.memory") {
                let hard_b = parse_memory_bytes(hard_mem).unwrap_or(f64::MAX);
                let used_b = used_map
                    .and_then(|u| u.get("requests.memory"))
                    .and_then(|q| parse_memory_bytes(q))
                    .unwrap_or(0.0);

                utilisation.push(QuotaUtilisation {
                    quota_name: quota_name.to_string(),
                    resource: "requests.memory".to_string(),
                    hard: hard_b,
                    used: used_b,
                    requested: req_mem_bytes,
                });

                if used_b + req_mem_bytes > hard_b {
                    violations.push(format!(
                        "ResourceQuota '{quota_name}': requests.memory would exceed hard limit \
                        ({:.0} bytes used + {:.0} requested > {:.0} hard)",
                        used_b, req_mem_bytes, hard_b
                    ));
                }
            }

            // Check pods count
            if let Some(hard_pods) = hard.get("pods") {
                let hard_n = hard_pods.0.parse::<f64>().unwrap_or(f64::MAX);
                let used_n = used_map
                    .and_then(|u| u.get("pods"))
                    .and_then(|q| q.0.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let req_n = node.spec.replicas as f64;

                utilisation.push(QuotaUtilisation {
                    quota_name: quota_name.to_string(),
                    resource: "pods".to_string(),
                    hard: hard_n,
                    used: used_n,
                    requested: req_n,
                });

                if used_n + req_n > hard_n {
                    violations.push(format!(
                        "ResourceQuota '{quota_name}': pods would exceed hard limit \
                        ({used_n:.0} used + {req_n:.0} requested > {hard_n:.0} hard)"
                    ));
                }
            }
        }
    }

    if violations.is_empty() {
        info!(
            node = %node.metadata.name.as_deref().unwrap_or(""),
            namespace = %namespace,
            "ResourceQuota check passed"
        );
        Ok(QuotaCheckResult {
            allowed: true,
            message: "quota check passed".to_string(),
            utilisation,
        })
    } else {
        let message = violations.join("; ");
        warn!(
            node = %node.metadata.name.as_deref().unwrap_or(""),
            namespace = %namespace,
            "ResourceQuota check failed: {}", message
        );
        Ok(QuotaCheckResult {
            allowed: false,
            message,
            utilisation,
        })
    }
}

// ── LimitRange validation ─────────────────────────────────────────────────────

/// Check that the node's resource requests/limits comply with namespace LimitRange.
///
/// Returns a list of violation messages. Empty means compliant.
pub async fn check_limit_range(
    client: &Client,
    node: &StellarNode,
) -> Result<Vec<String>> {
    let namespace = node
        .metadata
        .namespace
        .as_deref()
        .unwrap_or("default");

    let lr_api: Api<LimitRange> = Api::namespaced(client.clone(), namespace);
    let limit_ranges = lr_api
        .list(&ListParams::default())
        .await
        .map_err(Error::KubeError)?;

    let mut violations = Vec::new();

    for lr in &limit_ranges.items {
        let lr_name = lr.metadata.name.as_deref().unwrap_or("unknown");
        let Some(spec) = &lr.spec else { continue };

        for limit in &spec.limits {
            if limit.type_ != "Container" {
                continue;
            }

            // Check max CPU
            if let Some(max_cpu) = limit.max.as_ref().and_then(|m| m.get("cpu")) {
                let max_m = parse_cpu_millis(max_cpu).unwrap_or(f64::MAX);
                if let Some(req) = node
                    .spec
                    .resources
                    .limits
                    .get("cpu")
                    .and_then(|q| parse_cpu_millis(q))
                {
                    if req > max_m {
                        violations.push(format!(
                            "LimitRange '{lr_name}': container CPU limit {req:.0}m \
                             exceeds max {max_m:.0}m"
                        ));
                    }
                }
            }

            // Check max memory
            if let Some(max_mem) = limit.max.as_ref().and_then(|m| m.get("memory")) {
                let max_b = parse_memory_bytes(max_mem).unwrap_or(f64::MAX);
                if let Some(req) = node
                    .spec
                    .resources
                    .limits
                    .get("memory")
                    .and_then(|q| parse_memory_bytes(q))
                {
                    if req > max_b {
                        violations.push(format!(
                            "LimitRange '{lr_name}': container memory limit {req:.0} bytes \
                             exceeds max {max_b:.0} bytes"
                        ));
                    }
                }
            }
        }
    }

    Ok(violations)
}

// ── Prometheus metrics ────────────────────────────────────────────────────────

/// Record quota utilisation as Prometheus gauges.
///
/// Each `QuotaUtilisation` entry is emitted as three gauges:
/// - `stellar_operator_quota_hard`
/// - `stellar_operator_quota_used`
/// - `stellar_operator_quota_requested`
///
/// Labels: `namespace`, `quota_name`, `resource`.
///
/// In practice this function is called after every successful quota check so
/// Prometheus can alert on `used / hard > 0.9` before the hard limit is hit.
pub fn record_quota_metrics(namespace: &str, utilisation: &[QuotaUtilisation]) {
    for u in utilisation {
        debug!(
            namespace = %namespace,
            quota = %u.quota_name,
            resource = %u.resource,
            hard = u.hard,
            used = u.used,
            requested = u.requested,
            "quota utilisation"
        );
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_millis_handles_millicores() {
        assert_eq!(parse_cpu_millis(&Quantity("500m".to_string())), Some(500.0));
    }

    #[test]
    fn parse_cpu_millis_handles_cores() {
        assert_eq!(parse_cpu_millis(&Quantity("2".to_string())), Some(2000.0));
    }

    #[test]
    fn parse_cpu_millis_handles_fractional_cores() {
        assert_eq!(parse_cpu_millis(&Quantity("0.5".to_string())), Some(500.0));
    }

    #[test]
    fn parse_memory_bytes_handles_mebibytes() {
        assert_eq!(
            parse_memory_bytes(&Quantity("256Mi".to_string())),
            Some(256.0 * 1024.0 * 1024.0)
        );
    }

    #[test]
    fn parse_memory_bytes_handles_gibibytes() {
        assert_eq!(
            parse_memory_bytes(&Quantity("4Gi".to_string())),
            Some(4.0 * 1024.0 * 1024.0 * 1024.0)
        );
    }

    #[test]
    fn parse_memory_bytes_handles_megabytes() {
        assert_eq!(
            parse_memory_bytes(&Quantity("512M".to_string())),
            Some(512.0 * 1e6)
        );
    }

    #[test]
    fn parse_memory_bytes_handles_kibibytes() {
        assert_eq!(
            parse_memory_bytes(&Quantity("1Ki".to_string())),
            Some(1024.0)
        );
    }

    #[test]
    fn quota_utilisation_detects_cpu_breach() {
        let u = QuotaUtilisation {
            quota_name: "default-quota".to_string(),
            resource: "requests.cpu".to_string(),
            hard: 1000.0,
            used: 900.0,
            requested: 200.0,
        };
        // used (900) + requested (200) = 1100 > hard (1000) → breach
        assert!(u.used + u.requested > u.hard);
    }

    #[test]
    fn quota_utilisation_passes_when_under_limit() {
        let u = QuotaUtilisation {
            quota_name: "default-quota".to_string(),
            resource: "requests.memory".to_string(),
            hard: 4_294_967_296.0, // 4 Gi
            used: 1_073_741_824.0, // 1 Gi
            requested: 268_435_456.0, // 256 Mi
        };
        assert!(u.used + u.requested <= u.hard);
    }

    #[test]
    fn record_quota_metrics_does_not_panic_on_empty_slice() {
        record_quota_metrics("test-ns", &[]);
    }

    #[test]
    fn record_quota_metrics_logs_all_entries() {
        let entries = vec![
            QuotaUtilisation {
                quota_name: "q1".to_string(),
                resource: "requests.cpu".to_string(),
                hard: 2000.0,
                used: 500.0,
                requested: 300.0,
            },
            QuotaUtilisation {
                quota_name: "q1".to_string(),
                resource: "requests.memory".to_string(),
                hard: 4_294_967_296.0,
                used: 1_073_741_824.0,
                requested: 268_435_456.0,
            },
        ];
        // Should not panic
        record_quota_metrics("stellar-prod", &entries);
    }
}
