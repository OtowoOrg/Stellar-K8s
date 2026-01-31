//! Auto-remediation for stale/desynced Stellar nodes
//!
//! Detects nodes that are stuck (ledger not progressing) and performs graduated remediation:
//! 1. Restart the pod
//! 2. Emit event for manual intervention (Clear DB -> Fresh Sync)

use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Event, ObjectReference, Pod};
use kube::{
    api::{Api, DeleteParams, ListParams, Patch, PatchParams, PostParams},
    Client, Resource, ResourceExt,
};
use tracing::{debug, info};

use crate::crd::StellarNode;
use crate::error::{Error, Result};

/// Annotation keys for remediation state tracking
pub const LAST_LEDGER_ANNOTATION: &str = "stellar.org/last-observed-ledger";
pub const LAST_LEDGER_TIME_ANNOTATION: &str = "stellar.org/last-ledger-update-time";
pub const REMEDIATION_LEVEL_ANNOTATION: &str = "stellar.org/remediation-level";
pub const REMEDIATION_TIME_ANNOTATION: &str = "stellar.org/last-remediation-time";

/// Default stale threshold in minutes
const DEFAULT_STALE_THRESHOLD_MINUTES: i64 = 15;

/// Cooldown between remediation attempts in minutes
const REMEDIATION_COOLDOWN_MINUTES: i64 = 10;

/// Remediation levels (graduated response)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RemediationLevel {
    /// No remediation needed
    None = 0,
    /// Restart the pod
    Restart = 1,
    /// Database clear needed (requires manual intervention)
    ClearAndResync = 2,
}

impl RemediationLevel {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => RemediationLevel::None,
            1 => RemediationLevel::Restart,
            _ => RemediationLevel::ClearAndResync,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RemediationLevel::None => "None",
            RemediationLevel::Restart => "Restart",
            RemediationLevel::ClearAndResync => "ClearAndResync",
        }
    }
}

/// Result of stale detection check
#[derive(Debug)]
pub struct StaleCheckResult {
    pub is_stale: bool,
    pub current_ledger: Option<u64>,
    pub last_observed_ledger: Option<u64>,
    pub minutes_since_progress: Option<i64>,
    pub recommended_action: RemediationLevel,
}

impl StaleCheckResult {
    pub fn healthy(current_ledger: Option<u64>) -> Self {
        Self {
            is_stale: false,
            current_ledger,
            last_observed_ledger: current_ledger,
            minutes_since_progress: Some(0),
            recommended_action: RemediationLevel::None,
        }
    }

    pub fn stale(
        current_ledger: Option<u64>,
        last_observed: Option<u64>,
        minutes: i64,
        level: RemediationLevel,
    ) -> Self {
        Self {
            is_stale: true,
            current_ledger,
            last_observed_ledger: last_observed,
            minutes_since_progress: Some(minutes),
            recommended_action: level,
        }
    }
}

/// Check if a node is stale (ledger not progressing)
pub fn check_stale_node(node: &StellarNode, current_ledger: Option<u64>) -> StaleCheckResult {
    let annotations = node.metadata.annotations.as_ref();

    // Get last observed ledger and time
    let last_ledger: Option<u64> = annotations
        .and_then(|a| a.get(LAST_LEDGER_ANNOTATION))
        .and_then(|v| v.parse().ok());

    let last_time: Option<DateTime<Utc>> = annotations
        .and_then(|a| a.get(LAST_LEDGER_TIME_ANNOTATION))
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let current_level: u8 = annotations
        .and_then(|a| a.get(REMEDIATION_LEVEL_ANNOTATION))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    // If no current ledger, we can't determine staleness
    let current = match current_ledger {
        Some(l) => l,
        None => {
            debug!("No current ledger available, cannot check staleness");
            return StaleCheckResult::healthy(None);
        }
    };

    // Check if ledger has progressed
    match (last_ledger, last_time) {
        (Some(last), Some(time)) if current <= last => {
            // Ledger hasn't progressed - check how long
            let now = Utc::now();
            let duration = now.signed_duration_since(time);
            let minutes = duration.num_minutes();

            debug!(
                "Ledger stuck at {} for {} minutes (threshold: {})",
                current, minutes, DEFAULT_STALE_THRESHOLD_MINUTES
            );

            if minutes >= DEFAULT_STALE_THRESHOLD_MINUTES {
                // Determine remediation level based on previous attempts
                let next_level = if current_level == 0 {
                    RemediationLevel::Restart
                } else {
                    RemediationLevel::ClearAndResync
                };

                StaleCheckResult::stale(Some(current), Some(last), minutes, next_level)
            } else {
                StaleCheckResult::healthy(Some(current))
            }
        }
        _ => {
            // Ledger has progressed or first observation
            StaleCheckResult::healthy(Some(current))
        }
    }
}

/// Check if enough time has passed since last remediation
pub fn can_remediate(node: &StellarNode) -> bool {
    let last_remediation: Option<DateTime<Utc>> = node
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(REMEDIATION_TIME_ANNOTATION))
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc));

    match last_remediation {
        Some(time) => {
            let now = Utc::now();
            let duration = now.signed_duration_since(time);
            duration.num_minutes() >= REMEDIATION_COOLDOWN_MINUTES
        }
        None => true,
    }
}

/// Perform pod restart remediation
pub async fn restart_pod(client: &Client, node: &StellarNode) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();

    info!(
        "Performing pod restart remediation for {}/{}",
        namespace, name
    );

    let pod_api: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let label_selector =
        format!("app.kubernetes.io/instance={name},app.kubernetes.io/name=stellar-node");

    let pods = pod_api
        .list(&ListParams::default().labels(&label_selector))
        .await
        .map_err(Error::KubeError)?;

    for pod in pods.items {
        let pod_name = pod.name_any();
        info!("Deleting pod {} for remediation", pod_name);

        pod_api
            .delete(&pod_name, &DeleteParams::default())
            .await
            .map_err(Error::KubeError)?;
    }

    Ok(())
}

/// Emit a Kubernetes Event for remediation action
pub async fn emit_remediation_event(
    client: &Client,
    node: &StellarNode,
    action: RemediationLevel,
    reason: &str,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let events: Api<Event> = Api::namespaced(client.clone(), &namespace);

    let time = Utc::now();

    // Build object reference
    let obj_ref = ObjectReference {
        api_version: Some(StellarNode::api_version(&()).to_string()),
        kind: Some(StellarNode::kind(&()).to_string()),
        name: Some(node.name_any()),
        namespace: Some(namespace.clone()),
        uid: node.metadata.uid.clone(),
        ..Default::default()
    };

    let event = Event {
        metadata: kube::api::ObjectMeta {
            generate_name: Some(format!("{}-remediation-", node.name_any())),
            namespace: Some(namespace.clone()),
            ..Default::default()
        },
        type_: Some("Warning".to_string()),
        reason: Some(format!("AutoRemediation{}", action.as_str())),
        message: Some(format!(
            "Auto-remediation triggered: {} - {}",
            action.as_str(),
            reason
        )),
        involved_object: obj_ref,
        first_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(time)),
        last_timestamp: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(time)),
        count: Some(1),
        ..Default::default()
    };

    events
        .create(&PostParams::default(), &event)
        .await
        .map_err(Error::KubeError)?;

    info!(
        "Emitted remediation event for {}/{}: {:?}",
        namespace,
        node.name_any(),
        action
    );

    Ok(())
}

/// Update remediation annotations on the node
pub async fn update_remediation_state(
    client: &Client,
    node: &StellarNode,
    current_ledger: Option<u64>,
    level: RemediationLevel,
    performed_remediation: bool,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

    let now = Utc::now().to_rfc3339();
    let mut annotations = node.metadata.annotations.clone().unwrap_or_default();

    // Update ledger tracking if ledger progressed
    let last_ledger: Option<u64> = annotations
        .get(LAST_LEDGER_ANNOTATION)
        .and_then(|v| v.parse().ok());

    if current_ledger > last_ledger {
        if let Some(ledger) = current_ledger {
            annotations.insert(LAST_LEDGER_ANNOTATION.to_string(), ledger.to_string());
            annotations.insert(LAST_LEDGER_TIME_ANNOTATION.to_string(), now.clone());
            // Reset remediation level on progress
            annotations.insert(REMEDIATION_LEVEL_ANNOTATION.to_string(), "0".to_string());
            debug!(
                "Ledger progressed to {}, resetting remediation state",
                ledger
            );
        }
    }

    // Update remediation state if we performed remediation
    if performed_remediation {
        annotations.insert(
            REMEDIATION_LEVEL_ANNOTATION.to_string(),
            (level as u8).to_string(),
        );
        annotations.insert(REMEDIATION_TIME_ANNOTATION.to_string(), now);
        info!(
            "Updated remediation state: level={:?} for {}/{}",
            level,
            namespace,
            node.name_any()
        );
    }

    let patch = serde_json::json!({
        "metadata": {
            "annotations": annotations
        }
    });

    api.patch(
        &node.name_any(),
        &PatchParams::apply("stellar-operator"),
        &Patch::Merge(&patch),
    )
    .await
    .map_err(Error::KubeError)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remediation_level_conversion() {
        assert_eq!(RemediationLevel::from_u8(0), RemediationLevel::None);
        assert_eq!(RemediationLevel::from_u8(1), RemediationLevel::Restart);
        assert_eq!(
            RemediationLevel::from_u8(2),
            RemediationLevel::ClearAndResync
        );
        assert_eq!(
            RemediationLevel::from_u8(99),
            RemediationLevel::ClearAndResync
        );
    }

    #[test]
    fn test_remediation_level_as_str() {
        assert_eq!(RemediationLevel::None.as_str(), "None");
        assert_eq!(RemediationLevel::Restart.as_str(), "Restart");
        assert_eq!(RemediationLevel::ClearAndResync.as_str(), "ClearAndResync");
    }
}
