//! Configuration Rollback System
//!
//! Automatically rolls back configurations when deployment failures are detected.

use crate::config_mgmt::versioning::VersionManager;
use crate::crd::StellarNodeSpec;

pub struct RollbackManager;

impl RollbackManager {
    /// Determines if a rollback is needed based on node status conditions
    pub fn should_rollback(conditions: &[crate::crd::Condition]) -> bool {
        conditions.iter().any(|c| {
            c.type_ == "Ready" && c.status == "False" && 
            (c.reason == "CrashLoopBackOff" || c.reason == "ImagePullBackOff")
        })
    }

    /// Finds the previous stable version to roll back to
    pub fn get_rollback_target(history: &VersionManager) -> Option<StellarNodeSpec> {
        // Find the second to last version in history
        history.get_latest().and_then(|_| {
            // This is a simplified logic - in practice we'd track 'stable' versions
            history.get_version(history.get_latest().unwrap().version - 1)
                .map(|v| v.spec.clone())
        })
    }
}
