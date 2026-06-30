//! Structured event taxonomy for Stellar-K8s operator actions
//!
//! Provides a uniform vocabulary of `reason` and `action` strings emitted as
//! Kubernetes Events so that operators can write deterministic alert rules and
//! runbook queries without hard-coding free-form strings.
//!
//! # Usage
//!
//! ```rust
//! use stellar_k8s::controller::event_taxonomy::{EventAction, EventReason, EventCategory};
//!
//! let reason = EventReason::ReconcileSucceeded;
//! let action = EventAction::Reconcile;
//! println!("{} / {}", reason.as_str(), action.as_str());
//! ```

use kube::runtime::events::EventType;
use serde::{Deserialize, Serialize};

/// High-level category that a Kubernetes Event belongs to.
///
/// Used for metric labelling and log correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventCategory {
    /// Lifecycle events: creation, deletion, updates to StellarNode resources.
    Lifecycle,
    /// Health-monitoring events: sync lag, archive checks, node health probes.
    Health,
    /// Auto-remediation events: pod restarts, DB clears, catchup triggers.
    Remediation,
    /// Security events: CVE patches, PSS violations, mTLS rotations.
    Security,
    /// Scaling events: HPA triggers, VPA recommendations, disk expansions.
    Scaling,
    /// Disaster-recovery events: backups, restores, failovers.
    DisasterRecovery,
    /// Upgrade events: operator-driven rolling upgrades, GitOps applies.
    Upgrade,
    /// Configuration events: operator config reloads, feature-flag changes.
    Configuration,
    /// Audit events: admission, policy decisions, compliance checks.
    Audit,
}

impl EventCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lifecycle => "Lifecycle",
            Self::Health => "Health",
            Self::Remediation => "Remediation",
            Self::Security => "Security",
            Self::Scaling => "Scaling",
            Self::DisasterRecovery => "DisasterRecovery",
            Self::Upgrade => "Upgrade",
            Self::Configuration => "Configuration",
            Self::Audit => "Audit",
        }
    }
}

/// The `reason` field of a Kubernetes Event.
///
/// Reasons are short, CamelCase strings (≤ 128 chars) that identify *why*
/// the event was emitted.  Consumers can match these as exact strings in
/// alert rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventReason {
    // ── Lifecycle ─────────────────────────────────────────────────────────
    /// A new StellarNode resource has been observed for the first time.
    NodeCreated,
    /// A StellarNode resource has been deleted (after finalizer cleanup).
    NodeDeleted,
    /// A StellarNode spec change was detected and is being applied.
    NodeUpdated,
    /// Reconciliation of a StellarNode completed without error.
    ReconcileSucceeded,
    /// Reconciliation of a StellarNode failed; the item was re-queued.
    ReconcileFailed,
    /// A finalizer was added to a StellarNode to gate deletion.
    FinalizerAdded,
    /// A finalizer was removed after cleanup completed.
    FinalizerRemoved,

    // ── Health ────────────────────────────────────────────────────────────
    /// The node is fully synced and passing all health checks.
    NodeHealthy,
    /// The node is degraded: syncing but behind threshold.
    NodeDegraded,
    /// The node is not syncing: ledger is stale beyond the configured threshold.
    NodeStale,
    /// History archive health check passed.
    ArchiveHealthy,
    /// History archive health check detected one or more unhealthy URLs.
    ArchiveUnhealthy,
    /// The node's peer connectivity dropped below the minimum quorum threshold.
    QuorumUnderThreshold,

    // ── Remediation ───────────────────────────────────────────────────────
    /// A pod restart was triggered to recover a stale node.
    PodRestarted,
    /// A database-clear-and-resync was initiated for a persistently stale node.
    DatabaseClearInitiated,
    /// Automatic remediation was skipped because the cooldown period is active.
    RemediationCooldown,
    /// Remediation completed; the node returned to a healthy state.
    RemediationSucceeded,
    /// Remediation failed after the maximum number of attempts.
    RemediationFailed,

    // ── Security ──────────────────────────────────────────────────────────
    /// A CVE patch was applied to the node's container image.
    CvePatchApplied,
    /// A Pod Security Standard violation was detected and corrected.
    PssViolationCorrected,
    /// An mTLS certificate was rotated.
    MtlsCertRotated,
    /// A Kubernetes Secret referenced by the node was rotated; rolling restart triggered.
    SecretRotated,

    // ── Scaling ───────────────────────────────────────────────────────────
    /// A PVC was expanded because disk utilisation exceeded the threshold.
    DiskExpanded,
    /// An HPA target was adjusted based on Horizon request rates.
    HpaTargetAdjusted,
    /// A VPA recommendation was applied to the node's resource requests.
    VpaRecommendationApplied,

    // ── Disaster Recovery ─────────────────────────────────────────────────
    /// A snapshot backup was successfully created.
    BackupCreated,
    /// A node was successfully restored from a snapshot.
    RestoreCompleted,
    /// A cross-region or cross-cloud failover was initiated.
    FailoverInitiated,
    /// A DR drill completed successfully.
    DrDrillPassed,
    /// A DR drill failed; manual review required.
    DrDrillFailed,

    // ── Upgrade ───────────────────────────────────────────────────────────
    /// A rolling upgrade was initiated for a node.
    UpgradeInitiated,
    /// A rolling upgrade completed successfully.
    UpgradeSucceeded,
    /// A rolling upgrade was rolled back due to health check failures.
    UpgradeRolledBack,

    // ── Configuration ─────────────────────────────────────────────────────
    /// The operator ConfigMap was reloaded and new settings applied.
    ConfigReloaded,
    /// A feature flag changed state (enabled or disabled).
    FeatureFlagChanged,

    // ── Audit ─────────────────────────────────────────────────────────────
    /// An admission webhook request was validated and allowed.
    AdmissionAllowed,
    /// An admission webhook request was rejected due to policy violations.
    AdmissionRejected,
    /// A compliance check passed.
    CompliancePassed,
    /// A compliance check detected a violation.
    ComplianceViolation,
}

impl EventReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NodeCreated => "NodeCreated",
            Self::NodeDeleted => "NodeDeleted",
            Self::NodeUpdated => "NodeUpdated",
            Self::ReconcileSucceeded => "ReconcileSucceeded",
            Self::ReconcileFailed => "ReconcileFailed",
            Self::FinalizerAdded => "FinalizerAdded",
            Self::FinalizerRemoved => "FinalizerRemoved",
            Self::NodeHealthy => "NodeHealthy",
            Self::NodeDegraded => "NodeDegraded",
            Self::NodeStale => "NodeStale",
            Self::ArchiveHealthy => "ArchiveHealthy",
            Self::ArchiveUnhealthy => "ArchiveUnhealthy",
            Self::QuorumUnderThreshold => "QuorumUnderThreshold",
            Self::PodRestarted => "PodRestarted",
            Self::DatabaseClearInitiated => "DatabaseClearInitiated",
            Self::RemediationCooldown => "RemediationCooldown",
            Self::RemediationSucceeded => "RemediationSucceeded",
            Self::RemediationFailed => "RemediationFailed",
            Self::CvePatchApplied => "CvePatchApplied",
            Self::PssViolationCorrected => "PssViolationCorrected",
            Self::MtlsCertRotated => "MtlsCertRotated",
            Self::SecretRotated => "SecretRotated",
            Self::DiskExpanded => "DiskExpanded",
            Self::HpaTargetAdjusted => "HpaTargetAdjusted",
            Self::VpaRecommendationApplied => "VpaRecommendationApplied",
            Self::BackupCreated => "BackupCreated",
            Self::RestoreCompleted => "RestoreCompleted",
            Self::FailoverInitiated => "FailoverInitiated",
            Self::DrDrillPassed => "DrDrillPassed",
            Self::DrDrillFailed => "DrDrillFailed",
            Self::UpgradeInitiated => "UpgradeInitiated",
            Self::UpgradeSucceeded => "UpgradeSucceeded",
            Self::UpgradeRolledBack => "UpgradeRolledBack",
            Self::ConfigReloaded => "ConfigReloaded",
            Self::FeatureFlagChanged => "FeatureFlagChanged",
            Self::AdmissionAllowed => "AdmissionAllowed",
            Self::AdmissionRejected => "AdmissionRejected",
            Self::CompliancePassed => "CompliancePassed",
            Self::ComplianceViolation => "ComplianceViolation",
        }
    }

    /// Returns the [`EventType`] (Normal or Warning) appropriate for this reason.
    pub fn event_type(self) -> EventType {
        match self {
            Self::ReconcileFailed
            | Self::NodeDegraded
            | Self::NodeStale
            | Self::ArchiveUnhealthy
            | Self::QuorumUnderThreshold
            | Self::RemediationFailed
            | Self::RemediationCooldown
            | Self::DatabaseClearInitiated
            | Self::PodRestarted
            | Self::DrDrillFailed
            | Self::UpgradeRolledBack
            | Self::AdmissionRejected
            | Self::ComplianceViolation
            | Self::FailoverInitiated => EventType::Warning,
            _ => EventType::Normal,
        }
    }

    /// Returns the [`EventCategory`] this reason belongs to.
    pub fn category(self) -> EventCategory {
        match self {
            Self::NodeCreated
            | Self::NodeDeleted
            | Self::NodeUpdated
            | Self::ReconcileSucceeded
            | Self::ReconcileFailed
            | Self::FinalizerAdded
            | Self::FinalizerRemoved => EventCategory::Lifecycle,

            Self::NodeHealthy
            | Self::NodeDegraded
            | Self::NodeStale
            | Self::ArchiveHealthy
            | Self::ArchiveUnhealthy
            | Self::QuorumUnderThreshold => EventCategory::Health,

            Self::PodRestarted
            | Self::DatabaseClearInitiated
            | Self::RemediationCooldown
            | Self::RemediationSucceeded
            | Self::RemediationFailed => EventCategory::Remediation,

            Self::CvePatchApplied
            | Self::PssViolationCorrected
            | Self::MtlsCertRotated
            | Self::SecretRotated => EventCategory::Security,

            Self::DiskExpanded
            | Self::HpaTargetAdjusted
            | Self::VpaRecommendationApplied => EventCategory::Scaling,

            Self::BackupCreated
            | Self::RestoreCompleted
            | Self::FailoverInitiated
            | Self::DrDrillPassed
            | Self::DrDrillFailed => EventCategory::DisasterRecovery,

            Self::UpgradeInitiated
            | Self::UpgradeSucceeded
            | Self::UpgradeRolledBack => EventCategory::Upgrade,

            Self::ConfigReloaded | Self::FeatureFlagChanged => EventCategory::Configuration,

            Self::AdmissionAllowed
            | Self::AdmissionRejected
            | Self::CompliancePassed
            | Self::ComplianceViolation => EventCategory::Audit,
        }
    }
}

/// The `action` field of a Kubernetes Event.
///
/// Actions describe *what the operator did* in response to a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum EventAction {
    Reconcile,
    Create,
    Update,
    Delete,
    Restart,
    Scale,
    Backup,
    Restore,
    Upgrade,
    Rotate,
    Patch,
    Validate,
    Audit,
    Failover,
    Remediate,
    Configure,
}

impl EventAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reconcile => "Reconcile",
            Self::Create => "Create",
            Self::Update => "Update",
            Self::Delete => "Delete",
            Self::Restart => "Restart",
            Self::Scale => "Scale",
            Self::Backup => "Backup",
            Self::Restore => "Restore",
            Self::Upgrade => "Upgrade",
            Self::Rotate => "Rotate",
            Self::Patch => "Patch",
            Self::Validate => "Validate",
            Self::Audit => "Audit",
            Self::Failover => "Failover",
            Self::Remediate => "Remediate",
            Self::Configure => "Configure",
        }
    }
}

/// A fully-typed event descriptor pairing a reason with an action.
///
/// Pass this to [`EventEmitter::emit`] or deconstruct it for use with the
/// `emit_event!` macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventDescriptor {
    pub reason: EventReason,
    pub action: EventAction,
}

impl EventDescriptor {
    pub const fn new(reason: EventReason, action: EventAction) -> Self {
        Self { reason, action }
    }

    pub fn event_type(self) -> EventType {
        self.reason.event_type()
    }
    pub fn category(self) -> EventCategory {
        self.reason.category()
    }
}

/// Pre-defined descriptors for common operator flows.
pub mod descriptors {
    use super::{EventAction as A, EventDescriptor as D, EventReason as R};

    pub const RECONCILE_OK: D = D::new(R::ReconcileSucceeded, A::Reconcile);
    pub const RECONCILE_FAIL: D = D::new(R::ReconcileFailed, A::Reconcile);
    pub const NODE_CREATED: D = D::new(R::NodeCreated, A::Create);
    pub const NODE_DELETED: D = D::new(R::NodeDeleted, A::Delete);
    pub const NODE_UPDATED: D = D::new(R::NodeUpdated, A::Update);
    pub const POD_RESTARTED: D = D::new(R::PodRestarted, A::Restart);
    pub const DB_CLEAR: D = D::new(R::DatabaseClearInitiated, A::Remediate);
    pub const CVE_PATCH: D = D::new(R::CvePatchApplied, A::Patch);
    pub const DISK_EXPANDED: D = D::new(R::DiskExpanded, A::Scale);
    pub const BACKUP_CREATED: D = D::new(R::BackupCreated, A::Backup);
    pub const FAILOVER: D = D::new(R::FailoverInitiated, A::Failover);
    pub const UPGRADE_START: D = D::new(R::UpgradeInitiated, A::Upgrade);
    pub const UPGRADE_OK: D = D::new(R::UpgradeSucceeded, A::Upgrade);
    pub const UPGRADE_ROLLBACK: D = D::new(R::UpgradeRolledBack, A::Upgrade);
    pub const ADMISSION_ALLOWED: D = D::new(R::AdmissionAllowed, A::Validate);
    pub const ADMISSION_REJECTED: D = D::new(R::AdmissionRejected, A::Validate);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_str_round_trips() {
        let cases = [
            (EventReason::ReconcileSucceeded, "ReconcileSucceeded"),
            (EventReason::ReconcileFailed, "ReconcileFailed"),
            (EventReason::NodeCreated, "NodeCreated"),
            (EventReason::NodeDeleted, "NodeDeleted"),
            (EventReason::PodRestarted, "PodRestarted"),
            (EventReason::DatabaseClearInitiated, "DatabaseClearInitiated"),
            (EventReason::CvePatchApplied, "CvePatchApplied"),
            (EventReason::DiskExpanded, "DiskExpanded"),
            (EventReason::BackupCreated, "BackupCreated"),
            (EventReason::FailoverInitiated, "FailoverInitiated"),
            (EventReason::AdmissionRejected, "AdmissionRejected"),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason.as_str(), expected, "mismatch for {reason:?}");
        }
    }

    #[test]
    fn action_str_round_trips() {
        let cases = [
            (EventAction::Reconcile, "Reconcile"),
            (EventAction::Create, "Create"),
            (EventAction::Restart, "Restart"),
            (EventAction::Scale, "Scale"),
            (EventAction::Failover, "Failover"),
        ];
        for (action, expected) in cases {
            assert_eq!(action.as_str(), expected);
        }
    }

    #[test]
    fn warnings_are_warning_type() {
        let warning_reasons = [
            EventReason::ReconcileFailed,
            EventReason::NodeStale,
            EventReason::NodeDegraded,
            EventReason::ArchiveUnhealthy,
            EventReason::RemediationFailed,
            EventReason::AdmissionRejected,
            EventReason::ComplianceViolation,
            EventReason::DrDrillFailed,
            EventReason::UpgradeRolledBack,
        ];
        for r in warning_reasons {
            assert_eq!(
                r.event_type(),
                EventType::Warning,
                "{r:?} should be Warning"
            );
        }
    }

    #[test]
    fn normals_are_normal_type() {
        let normal_reasons = [
            EventReason::ReconcileSucceeded,
            EventReason::NodeCreated,
            EventReason::NodeHealthy,
            EventReason::ArchiveHealthy,
            EventReason::CvePatchApplied,
            EventReason::BackupCreated,
            EventReason::UpgradeSucceeded,
        ];
        for r in normal_reasons {
            assert_eq!(
                r.event_type(),
                EventType::Normal,
                "{r:?} should be Normal"
            );
        }
    }

    #[test]
    fn category_lifecycle_covers_reconcile() {
        assert_eq!(
            EventReason::ReconcileSucceeded.category(),
            EventCategory::Lifecycle
        );
        assert_eq!(
            EventReason::ReconcileFailed.category(),
            EventCategory::Lifecycle
        );
    }

    #[test]
    fn category_health_covers_stale() {
        assert_eq!(EventReason::NodeStale.category(), EventCategory::Health);
        assert_eq!(
            EventReason::ArchiveUnhealthy.category(),
            EventCategory::Health
        );
    }

    #[test]
    fn category_remediation_covers_restart() {
        assert_eq!(
            EventReason::PodRestarted.category(),
            EventCategory::Remediation
        );
    }

    #[test]
    fn descriptor_event_type_delegates_to_reason() {
        let d = descriptors::RECONCILE_FAIL;
        assert_eq!(d.event_type(), EventType::Warning);

        let d = descriptors::RECONCILE_OK;
        assert_eq!(d.event_type(), EventType::Normal);
    }

    #[test]
    fn descriptor_category_delegates_to_reason() {
        assert_eq!(descriptors::POD_RESTARTED.category(), EventCategory::Remediation);
        assert_eq!(descriptors::DISK_EXPANDED.category(), EventCategory::Scaling);
        assert_eq!(descriptors::BACKUP_CREATED.category(), EventCategory::DisasterRecovery);
        assert_eq!(descriptors::ADMISSION_REJECTED.category(), EventCategory::Audit);
    }
}
