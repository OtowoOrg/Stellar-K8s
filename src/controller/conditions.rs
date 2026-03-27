//! Condition management helpers following Kubernetes API conventions

use chrono::Utc;

use crate::crd::Condition;

/// Standard condition types following Kubernetes conventions
pub const CONDITION_TYPE_READY: &str = "Ready";
pub const CONDITION_TYPE_PROGRESSING: &str = "Progressing";
pub const CONDITION_TYPE_DEGRADED: &str = "Degraded";
pub const CONDITION_TYPE_AVAILABLE: &str = "Available";

/// Standard condition statuses
pub const CONDITION_STATUS_TRUE: &str = "True";
pub const CONDITION_STATUS_FALSE: &str = "False";
pub const CONDITION_STATUS_UNKNOWN: &str = "Unknown";

/// Update or add a condition to the conditions list
///
/// If a condition with the same type exists and has different status/reason/message,
/// it will be updated with a new transition time. Otherwise, it will be added.
pub fn set_condition(
    conditions: &mut Vec<Condition>,
    type_: &str,
    status: &str,
    reason: &str,
    message: &str,
) {
    let now = Utc::now().to_rfc3339();

    if let Some(existing) = conditions.iter_mut().find(|c| c.type_ == type_) {
        // Update transition time only if status changed
        let should_update_time = existing.status != status;

        existing.status = status.to_string();
        existing.reason = reason.to_string();
        existing.message = message.to_string();

        if should_update_time {
            existing.last_transition_time = now;
        }
    } else {
        // Add new condition
        conditions.push(Condition {
            type_: type_.to_string(),
            status: status.to_string(),
            last_transition_time: now,
            reason: reason.to_string(),
            message: message.to_string(),
            observed_generation: None,
        });
    }
}

/// Find a condition by type
pub fn find_condition<'a>(conditions: &'a [Condition], type_: &str) -> Option<&'a Condition> {
    conditions.iter().find(|c| c.type_ == type_)
}

/// Check if a condition is true
pub fn is_condition_true(conditions: &[Condition], type_: &str) -> bool {
    find_condition(conditions, type_)
        .map(|c| c.status == CONDITION_STATUS_TRUE)
        .unwrap_or(false)
}

/// Remove a condition by type
pub fn remove_condition(conditions: &mut Vec<Condition>, type_: &str) {
    conditions.retain(|c| c.type_ != type_);
}

/// Create a Ready=True condition
pub fn ready_condition(reason: &str, message: &str) -> Condition {
    Condition {
        type_: CONDITION_TYPE_READY.to_string(),
        status: CONDITION_STATUS_TRUE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: None,
    }
}

/// Create a Ready=False condition
pub fn not_ready_condition(reason: &str, message: &str) -> Condition {
    Condition {
        type_: CONDITION_TYPE_READY.to_string(),
        status: CONDITION_STATUS_FALSE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: None,
    }
}

/// Create a Progressing=True condition
pub fn progressing_condition(reason: &str, message: &str) -> Condition {
    Condition {
        type_: CONDITION_TYPE_PROGRESSING.to_string(),
        status: CONDITION_STATUS_TRUE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: None,
    }
}

/// Create a Progressing=False condition
pub fn not_progressing_condition(reason: &str, message: &str) -> Condition {
    Condition {
        type_: CONDITION_TYPE_PROGRESSING.to_string(),
        status: CONDITION_STATUS_FALSE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: None,
    }
}

/// Create a Degraded=True condition
pub fn degraded_condition(reason: &str, message: &str) -> Condition {
    Condition {
        type_: CONDITION_TYPE_DEGRADED.to_string(),
        status: CONDITION_STATUS_TRUE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: reason.to_string(),
        message: message.to_string(),
        observed_generation: None,
    }
}

/// Create a Degraded=False condition
pub fn not_degraded_condition() -> Condition {
    Condition {
        type_: CONDITION_TYPE_DEGRADED.to_string(),
        status: CONDITION_STATUS_FALSE.to_string(),
        last_transition_time: Utc::now().to_rfc3339(),
        reason: "NoIssues".to_string(),
        message: "No degradation detected".to_string(),
        observed_generation: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── set_condition: adding new conditions ──────────────────────────────────

    #[test]
    fn test_set_condition_adds_new() {
        let mut conditions = Vec::new();
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "AllHealthy",
            "All checks passed",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].type_, CONDITION_TYPE_READY);
        assert_eq!(conditions[0].status, CONDITION_STATUS_TRUE);
        assert_eq!(conditions[0].reason, "AllHealthy");
        assert_eq!(conditions[0].message, "All checks passed");
    }

    #[test]
    fn test_set_condition_adds_multiple_different_types() {
        let mut conditions = Vec::new();

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Ready",
            "Node is ready",
        );
        set_condition(
            &mut conditions,
            CONDITION_TYPE_PROGRESSING,
            CONDITION_STATUS_TRUE,
            "Syncing",
            "Syncing data",
        );
        set_condition(
            &mut conditions,
            CONDITION_TYPE_DEGRADED,
            CONDITION_STATUS_FALSE,
            "NoIssues",
            "No degradation",
        );

        assert_eq!(conditions.len(), 3);
        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_PROGRESSING).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_DEGRADED).is_some());
    }

    #[test]
    fn test_set_condition_adds_empty_conditions_list() {
        let mut conditions: Vec<Condition> = Vec::new();
        set_condition(
            &mut conditions,
            CONDITION_TYPE_AVAILABLE,
            CONDITION_STATUS_TRUE,
            "Available",
            "Node is available",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].type_, CONDITION_TYPE_AVAILABLE);
    }

    // ── set_condition: updating existing conditions ───────────────────────────

    #[test]
    fn test_set_condition_updates_existing() {
        let mut conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_FALSE.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "NotHealthy".to_string(),
            message: "Node not ready".to_string(),
            observed_generation: None,
        }];

        let old_time = conditions[0].last_transition_time.clone();
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Healthy",
            "Node is ready",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].status, CONDITION_STATUS_TRUE);
        assert_eq!(conditions[0].reason, "Healthy");
        assert_eq!(conditions[0].message, "Node is ready");
        // Time should change when status changes
        assert_ne!(conditions[0].last_transition_time, old_time);
    }

    #[test]
    fn test_set_condition_updates_without_status_change() {
        let mut conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_TRUE.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "OldReason".to_string(),
            message: "Old message".to_string(),
            observed_generation: None,
        }];

        let old_time = conditions[0].last_transition_time.clone();
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE, // Same status
            "NewReason",
            "New message",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].reason, "NewReason");
        assert_eq!(conditions[0].message, "New message");
        // Time should NOT change when status stays the same
        assert_eq!(conditions[0].last_transition_time, old_time);
    }

    #[test]
    fn test_set_condition_transitions_true_to_false() {
        let mut conditions = vec![ready_condition("Healthy", "All good")];

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_FALSE,
            "Unhealthy",
            "Checks failed",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].status, CONDITION_STATUS_FALSE);
        assert_eq!(conditions[0].reason, "Unhealthy");
    }

    #[test]
    fn test_set_condition_transitions_false_to_true() {
        let mut conditions = vec![not_ready_condition("Unhealthy", "Checks failed")];

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Healthy",
            "All checks passed",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].status, CONDITION_STATUS_TRUE);
        assert_eq!(conditions[0].reason, "Healthy");
    }

    #[test]
    fn test_set_condition_transitions_to_unknown() {
        let mut conditions = vec![ready_condition("Healthy", "All good")];

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_UNKNOWN,
            "Unknown",
            "Health status unknown",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].status, CONDITION_STATUS_UNKNOWN);
    }

    #[test]
    fn test_set_condition_only_updates_matching_type() {
        let mut conditions = vec![
            ready_condition("Healthy", "All good"),
            progressing_condition("Syncing", "Syncing data"),
        ];

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_FALSE,
            "Unhealthy",
            "Failed",
        );

        assert_eq!(conditions.len(), 2);
        // READY should be updated
        assert_eq!(conditions[0].status, CONDITION_STATUS_FALSE);
        // PROGRESSING should be unchanged
        assert_eq!(conditions[1].status, CONDITION_STATUS_TRUE);
        assert_eq!(conditions[1].reason, "Syncing");
    }

    // ── set_condition: edge cases ─────────────────────────────────────────────

    #[test]
    fn test_set_condition_with_empty_reason_and_message() {
        let mut conditions = Vec::new();
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "",
            "",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].reason, "");
        assert_eq!(conditions[0].message, "");
    }

    #[test]
    fn test_set_condition_preserves_other_fields() {
        let mut conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_FALSE.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "OldReason".to_string(),
            message: "OldMessage".to_string(),
            observed_generation: Some(5),
        }];

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "NewReason",
            "NewMessage",
        );

        // observed_generation should be preserved
        assert_eq!(conditions[0].observed_generation, Some(5));
    }

    // ── find_condition ────────────────────────────────────────────────────────

    #[test]
    fn test_find_condition() {
        let conditions = vec![
            ready_condition("Healthy", "All good"),
            progressing_condition("Syncing", "Syncing data"),
        ];

        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_PROGRESSING).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_DEGRADED).is_none());
    }

    #[test]
    fn test_find_condition_returns_first_match() {
        let conditions = vec![
            ready_condition("Healthy", "First"),
            ready_condition("Healthy", "Second"), // Duplicate type
        ];

        let found = find_condition(&conditions, CONDITION_TYPE_READY);
        assert!(found.is_some());
        // Should return the first match
        assert_eq!(found.unwrap().message, "First");
    }

    #[test]
    fn test_find_condition_empty_list() {
        let conditions: Vec<Condition> = Vec::new();
        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_none());
    }

    #[test]
    fn test_find_condition_all_standard_types() {
        let conditions = vec![
            ready_condition("Ready", "Ready message"),
            progressing_condition("Progressing", "Progressing message"),
            degraded_condition("Degraded", "Degraded message"),
            Condition {
                type_: CONDITION_TYPE_AVAILABLE.to_string(),
                status: CONDITION_STATUS_TRUE.to_string(),
                last_transition_time: "2024-01-01T00:00:00Z".to_string(),
                reason: "Available".to_string(),
                message: "Available message".to_string(),
                observed_generation: None,
            },
        ];

        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_PROGRESSING).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_DEGRADED).is_some());
        assert!(find_condition(&conditions, CONDITION_TYPE_AVAILABLE).is_some());
    }

    // ── is_condition_true ─────────────────────────────────────────────────────

    #[test]
    fn test_is_condition_true() {
        let conditions = vec![ready_condition("Healthy", "All good")];

        assert!(is_condition_true(&conditions, CONDITION_TYPE_READY));
        assert!(!is_condition_true(&conditions, CONDITION_TYPE_DEGRADED));
    }

    #[test]
    fn test_is_condition_true_with_false_status() {
        let conditions = vec![not_ready_condition("Unhealthy", "Failed")];

        assert!(!is_condition_true(&conditions, CONDITION_TYPE_READY));
    }

    #[test]
    fn test_is_condition_true_with_unknown_status() {
        let conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_UNKNOWN.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "Unknown".to_string(),
            message: "Status unknown".to_string(),
            observed_generation: None,
        }];

        assert!(!is_condition_true(&conditions, CONDITION_TYPE_READY));
    }

    #[test]
    fn test_is_condition_true_empty_list() {
        let conditions: Vec<Condition> = Vec::new();
        assert!(!is_condition_true(&conditions, CONDITION_TYPE_READY));
    }

    #[test]
    fn test_is_condition_true_multiple_conditions() {
        let conditions = vec![
            ready_condition("Healthy", "Ready"),
            degraded_condition("Degraded", "Degraded"),
        ];

        assert!(is_condition_true(&conditions, CONDITION_TYPE_READY));
        assert!(is_condition_true(&conditions, CONDITION_TYPE_DEGRADED));
        assert!(!is_condition_true(&conditions, CONDITION_TYPE_PROGRESSING));
    }

    // ── remove_condition ──────────────────────────────────────────────────────

    #[test]
    fn test_remove_condition() {
        let mut conditions = vec![
            ready_condition("Healthy", "Ready"),
            progressing_condition("Syncing", "Syncing"),
        ];

        remove_condition(&mut conditions, CONDITION_TYPE_READY);

        assert_eq!(conditions.len(), 1);
        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_none());
        assert!(find_condition(&conditions, CONDITION_TYPE_PROGRESSING).is_some());
    }

    #[test]
    fn test_remove_condition_nonexistent() {
        let mut conditions = vec![ready_condition("Healthy", "Ready")];

        remove_condition(&mut conditions, CONDITION_TYPE_DEGRADED);

        // Should not panic and should not remove anything
        assert_eq!(conditions.len(), 1);
        assert!(find_condition(&conditions, CONDITION_TYPE_READY).is_some());
    }

    #[test]
    fn test_remove_condition_empty_list() {
        let mut conditions: Vec<Condition> = Vec::new();

        remove_condition(&mut conditions, CONDITION_TYPE_READY);

        assert!(conditions.is_empty());
    }

    #[test]
    fn test_remove_condition_all() {
        let mut conditions = vec![
            ready_condition("Healthy", "Ready"),
            progressing_condition("Syncing", "Syncing"),
            degraded_condition("Degraded", "Degraded"),
        ];

        remove_condition(&mut conditions, CONDITION_TYPE_READY);
        remove_condition(&mut conditions, CONDITION_TYPE_PROGRESSING);
        remove_condition(&mut conditions, CONDITION_TYPE_DEGRADED);

        assert!(conditions.is_empty());
    }

    // ── transition time behavior ──────────────────────────────────────────────

    #[test]
    fn test_transition_time_set_on_new_condition() {
        let mut conditions = Vec::new();

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Ready",
            "Node is ready",
        );

        // Transition time should be set
        assert!(!conditions[0].last_transition_time.is_empty());
    }

    #[test]
    fn test_transition_time_changes_on_status_change() {
        let mut conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_FALSE.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "NotReady".to_string(),
            message: "Not ready".to_string(),
            observed_generation: None,
        }];

        let old_time = conditions[0].last_transition_time.clone();

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Ready",
            "Node is ready",
        );

        // Transition time should be updated
        assert_ne!(conditions[0].last_transition_time, old_time);
    }

    #[test]
    fn test_transition_time_preserved_on_same_status() {
        let mut conditions = vec![Condition {
            type_: CONDITION_TYPE_READY.to_string(),
            status: CONDITION_STATUS_TRUE.to_string(),
            last_transition_time: "2024-01-01T00:00:00Z".to_string(),
            reason: "OldReason".to_string(),
            message: "Old message".to_string(),
            observed_generation: None,
        }];

        let old_time = conditions[0].last_transition_time.clone();

        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE, // Same status
            "NewReason",
            "New message",
        );

        // Transition time should be preserved
        assert_eq!(conditions[0].last_transition_time, old_time);
    }

    // ── convenience constructors ──────────────────────────────────────────────

    #[test]
    fn test_ready_condition_constructor() {
        let condition = ready_condition("Healthy", "All checks passed");

        assert_eq!(condition.type_, CONDITION_TYPE_READY);
        assert_eq!(condition.status, CONDITION_STATUS_TRUE);
        assert_eq!(condition.reason, "Healthy");
        assert_eq!(condition.message, "All checks passed");
        assert!(!condition.last_transition_time.is_empty());
    }

    #[test]
    fn test_not_ready_condition_constructor() {
        let condition = not_ready_condition("Unhealthy", "Some checks failed");

        assert_eq!(condition.type_, CONDITION_TYPE_READY);
        assert_eq!(condition.status, CONDITION_STATUS_FALSE);
        assert_eq!(condition.reason, "Unhealthy");
        assert_eq!(condition.message, "Some checks failed");
    }

    #[test]
    fn test_progressing_condition_constructor() {
        let condition = progressing_condition("Syncing", "Syncing ledger data");

        assert_eq!(condition.type_, CONDITION_TYPE_PROGRESSING);
        assert_eq!(condition.status, CONDITION_STATUS_TRUE);
        assert_eq!(condition.reason, "Syncing");
        assert_eq!(condition.message, "Syncing ledger data");
    }

    #[test]
    fn test_not_progressing_condition_constructor() {
        let condition = not_progressing_condition("Idle", "No active sync");

        assert_eq!(condition.type_, CONDITION_TYPE_PROGRESSING);
        assert_eq!(condition.status, CONDITION_STATUS_FALSE);
        assert_eq!(condition.reason, "Idle");
        assert_eq!(condition.message, "No active sync");
    }

    #[test]
    fn test_degraded_condition_constructor() {
        let condition = degraded_condition("HighLatency", "Network latency detected");

        assert_eq!(condition.type_, CONDITION_TYPE_DEGRADED);
        assert_eq!(condition.status, CONDITION_STATUS_TRUE);
        assert_eq!(condition.reason, "HighLatency");
        assert_eq!(condition.message, "Network latency detected");
    }

    #[test]
    fn test_not_degraded_condition_constructor() {
        let condition = not_degraded_condition();

        assert_eq!(condition.type_, CONDITION_TYPE_DEGRADED);
        assert_eq!(condition.status, CONDITION_STATUS_FALSE);
        assert_eq!(condition.reason, "NoIssues");
        assert_eq!(condition.message, "No degradation detected");
    }

    // ── complex scenarios ─────────────────────────────────────────────────────

    #[test]
    fn test_multiple_status_transitions() {
        let mut conditions = Vec::new();

        // Start with not ready
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_FALSE,
            "Unhealthy",
            "Initial state",
        );

        // Transition to ready
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Healthy",
            "All checks passed",
        );

        // Transition back to not ready
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_FALSE,
            "Unhealthy",
            "Check failed",
        );

        assert_eq!(conditions.len(), 1);
        assert_eq!(conditions[0].status, CONDITION_STATUS_FALSE);
        assert_eq!(conditions[0].reason, "Unhealthy");
    }

    #[test]
    fn test_conditional_logic_with_multiple_conditions() {
        let conditions = vec![
            ready_condition("Healthy", "Ready"),
            degraded_condition("Degraded", "Degraded"),
        ];

        // Node is ready but degraded
        assert!(is_condition_true(&conditions, CONDITION_TYPE_READY));
        assert!(is_condition_true(&conditions, CONDITION_TYPE_DEGRADED));

        // This represents a node that is functional but performing poorly
    }

    #[test]
    fn test_condition_order_preserved() {
        let mut conditions = Vec::new();

        set_condition(
            &mut conditions,
            CONDITION_TYPE_DEGRADED,
            CONDITION_STATUS_TRUE,
            "Degraded",
            "Degraded",
        );
        set_condition(
            &mut conditions,
            CONDITION_TYPE_READY,
            CONDITION_STATUS_TRUE,
            "Ready",
            "Ready",
        );
        set_condition(
            &mut conditions,
            CONDITION_TYPE_PROGRESSING,
            CONDITION_STATUS_TRUE,
            "Progressing",
            "Progressing",
        );

        // Order should be preserved as inserted
        assert_eq!(conditions[0].type_, CONDITION_TYPE_DEGRADED);
        assert_eq!(conditions[1].type_, CONDITION_TYPE_READY);
        assert_eq!(conditions[2].type_, CONDITION_TYPE_PROGRESSING);
    }
}
