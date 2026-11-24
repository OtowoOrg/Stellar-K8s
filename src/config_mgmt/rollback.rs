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
            c.type_ == "Ready"
                && c.status == "False"
                && (c.reason == "CrashLoopBackOff" || c.reason == "ImagePullBackOff")
        })
    }

    /// Finds the previous stable version to roll back to
    pub fn get_rollback_target(history: &VersionManager) -> Option<StellarNodeSpec> {
        // Find the second to last version in history
        history.get_latest().and_then(|_| {
            // This is a simplified logic - in practice we'd track 'stable' versions
            history
                .get_version(history.get_latest().unwrap().version - 1)
                .map(|v| v.spec.clone())
        })
    }
}
