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
    }

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
        assert_ne!(conditions[0].last_transition_time, old_time); // Time should change when status changes
    }

    #[test]
    fn test_is_condition_true() {
        let conditions = vec![ready_condition("Healthy", "All good")];

        assert!(is_condition_true(&conditions, CONDITION_TYPE_READY));
        assert!(!is_condition_true(&conditions, CONDITION_TYPE_DEGRADED));
    }

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
}
