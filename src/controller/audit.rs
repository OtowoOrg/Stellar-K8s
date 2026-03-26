//! Kubernetes annotation-based audit trail for reconciliation actions.
//!
//! Every successful reconcile stamps three annotations on the StellarNode:
//!
//! | Annotation                          | Example value                    |
//! |-------------------------------------|----------------------------------|
//! | `stellar.org/last-reconcile-time`   | `2026-03-25T12:00:00Z`           |
//! | `stellar.org/last-action`           | `created-deployment`             |
//! | `stellar.org/operator-version`      | `0.1.0`                          |

use chrono::Utc;
use kube::{
    api::{Api, Patch, PatchParams},
    client::Client,
    ResourceExt,
};
use std::collections::BTreeMap;
use tracing::warn;

use crate::crd::StellarNode;
use crate::error::Result;

// Annotation keys
pub const ANNOTATION_LAST_RECONCILE_TIME: &str = "stellar.org/last-reconcile-time";
pub const ANNOTATION_LAST_ACTION: &str = "stellar.org/last-action";
pub const ANNOTATION_OPERATOR_VERSION: &str = "stellar.org/operator-version";

/// Well-known action strings recorded in `stellar.org/last-action`.
pub mod actions {
    pub const CREATED_DEPLOYMENT: &str = "created-deployment";
    pub const UPDATED_DEPLOYMENT: &str = "updated-deployment";
    pub const CREATED_STATEFULSET: &str = "created-statefulset";
    pub const UPDATED_STATEFULSET: &str = "updated-statefulset";
    pub const UPDATED_SERVICE: &str = "updated-service";
    pub const UPDATED_CONFIG: &str = "updated-config";
    pub const RECONCILED: &str = "reconciled";
    pub const SUSPENDED: &str = "suspended";
    pub const MAINTENANCE: &str = "maintenance";
    pub const REMEDIATED: &str = "remediated";
    pub const DELETED: &str = "deleted";
}

/// Build the annotation map for a reconcile event.
///
/// Returns a `BTreeMap` ready to be merged into `metadata.annotations`.
pub fn build_audit_annotations(action: &str) -> BTreeMap<String, String> {
    let mut annotations = BTreeMap::new();
    annotations.insert(
        ANNOTATION_LAST_RECONCILE_TIME.to_string(),
        Utc::now().to_rfc3339(),
    );
    annotations.insert(ANNOTATION_LAST_ACTION.to_string(), action.to_string());
    annotations.insert(
        ANNOTATION_OPERATOR_VERSION.to_string(),
        env!("CARGO_PKG_VERSION").to_string(),
    );
    annotations
}

/// Patch the audit annotations onto a StellarNode resource.
///
/// Errors are logged as warnings and never propagate — annotation failures
/// must not block reconciliation.
pub async fn patch_audit_annotations(client: &Client, node: &StellarNode, action: &str) {
    if let Err(e) = try_patch_audit_annotations(client, node, action).await {
        warn!(
            "Failed to patch audit annotations on {}/{}: {:?}",
            node.namespace().unwrap_or_default(),
            node.name_any(),
            e
        );
    }
}

async fn try_patch_audit_annotations(
    client: &Client,
    node: &StellarNode,
    action: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let annotations = build_audit_annotations(action);
    let patch = serde_json::json!({
        "metadata": {
            "annotations": annotations
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator").force(),
        &Patch::Apply(&patch),
    )
    .await
    .map_err(crate::error::Error::KubeError)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_audit_annotations_contains_all_keys() {
        let annotations = build_audit_annotations(actions::RECONCILED);

        assert!(
            annotations.contains_key(ANNOTATION_LAST_RECONCILE_TIME),
            "missing last-reconcile-time"
        );
        assert!(
            annotations.contains_key(ANNOTATION_LAST_ACTION),
            "missing last-action"
        );
        assert!(
            annotations.contains_key(ANNOTATION_OPERATOR_VERSION),
            "missing operator-version"
        );
    }

    #[test]
    fn build_audit_annotations_records_correct_action() {
        let annotations = build_audit_annotations(actions::CREATED_DEPLOYMENT);
        assert_eq!(
            annotations.get(ANNOTATION_LAST_ACTION).map(String::as_str),
            Some(actions::CREATED_DEPLOYMENT)
        );
    }

    #[test]
    fn build_audit_annotations_records_operator_version() {
        let annotations = build_audit_annotations(actions::RECONCILED);
        let version = annotations
            .get(ANNOTATION_OPERATOR_VERSION)
            .expect("operator-version annotation missing");
        // Version must be non-empty and match the crate version
        assert!(!version.is_empty());
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn build_audit_annotations_reconcile_time_is_rfc3339() {
        let annotations = build_audit_annotations(actions::RECONCILED);
        let time_str = annotations
            .get(ANNOTATION_LAST_RECONCILE_TIME)
            .expect("last-reconcile-time annotation missing");
        // Must parse as a valid RFC 3339 timestamp
        chrono::DateTime::parse_from_rfc3339(time_str)
            .expect("last-reconcile-time is not valid RFC 3339");
    }

    #[test]
    fn build_audit_annotations_custom_action() {
        let custom = "custom-action-xyz";
        let annotations = build_audit_annotations(custom);
        assert_eq!(
            annotations.get(ANNOTATION_LAST_ACTION).map(String::as_str),
            Some(custom)
        );
    }

    #[test]
    fn all_action_constants_are_non_empty() {
        let all = [
            actions::CREATED_DEPLOYMENT,
            actions::UPDATED_DEPLOYMENT,
            actions::CREATED_STATEFULSET,
            actions::UPDATED_STATEFULSET,
            actions::UPDATED_SERVICE,
            actions::UPDATED_CONFIG,
            actions::RECONCILED,
            actions::SUSPENDED,
            actions::MAINTENANCE,
            actions::REMEDIATED,
            actions::DELETED,
        ];
        for action in all {
            assert!(!action.is_empty(), "action constant must not be empty");
        }
    }
}
