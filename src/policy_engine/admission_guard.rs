// Copyright 2026 Stellar-K8s Contributors
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
//! Runtime admission guard for dual-approval emergency overrides and
//! LRU-cached cryptographic bundle verification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

pub const EMERGENCY_OVERRIDE_ANNOTATION: &str = "stellar.org/emergency-override";
pub const EMERGENCY_APPROVER_1_ANNOTATION: &str = "stellar.org/emergency-approver-1";
pub const EMERGENCY_APPROVER_2_ANNOTATION: &str = "stellar.org/emergency-approver-2";

/// Dual-approval emergency override decision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmergencyOverrideValidation {
    pub approved: bool,
    pub reason: String,
}

/// Validates emergency override annotations requiring two distinct approvers.
pub fn validate_emergency_override(
    annotations: &HashMap<String, String>,
) -> EmergencyOverrideValidation {
    if annotations
        .get(EMERGENCY_OVERRIDE_ANNOTATION)
        .map(|v| v == "true")
        .unwrap_or(false)
    {
        let a1 = annotations
            .get(EMERGENCY_APPROVER_1_ANNOTATION)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let a2 = annotations
            .get(EMERGENCY_APPROVER_2_ANNOTATION)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        match (a1, a2) {
            (Some(p1), Some(p2)) if p1 != p2 => EmergencyOverrideValidation {
                approved: true,
                reason: format!("Dual-approval emergency override granted by {p1} and {p2}"),
            },
            _ => EmergencyOverrideValidation {
                approved: false,
                reason: "Emergency override requires two distinct non-empty approvers".into(),
            },
        }
    } else {
        EmergencyOverrideValidation {
            approved: false,
            reason: "No emergency override annotation present".into(),
        }
    }
}

/// Bounded LRU cache keyed on bundle SHA-256 digest so bundles are only
/// re-verified when their digest changes.
pub struct VerifiedBundleLruCache<V> {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, (V, DateTime<Utc>)>,
}

impl<V: Clone> VerifiedBundleLruCache<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
        }
    }

    pub fn get_valid(&mut self, digest: &str, now: DateTime<Utc>) -> Option<V> {
        let (val, expires_at) = self.entries.get(digest)?.clone();
        if now >= expires_at {
            self.entries.remove(digest);
            return None;
        }
        self.order.retain(|d| d != digest);
        self.order.push_back(digest.to_string());
        Some(val)
    }

    pub fn insert(&mut self, digest: String, value: V, expires_at: DateTime<Utc>) {
        if self.entries.len() >= self.capacity && !self.entries.contains_key(&digest) {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.retain(|d| d != &digest);
        self.order.push_back(digest.clone());
        self.entries.insert(digest, (value, expires_at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn emergency_override_requires_two_distinct_approvers() {
        let mut ann = HashMap::new();
        ann.insert(EMERGENCY_OVERRIDE_ANNOTATION.into(), "true".into());
        ann.insert(EMERGENCY_APPROVER_1_ANNOTATION.into(), "alice".into());
        ann.insert(EMERGENCY_APPROVER_2_ANNOTATION.into(), "alice".into());
        assert!(!validate_emergency_override(&ann).approved);

        ann.insert(EMERGENCY_APPROVER_2_ANNOTATION.into(), "bob".into());
        assert!(validate_emergency_override(&ann).approved);
    }

    #[test]
    fn lru_cache_evicts_expired_bundles_fail_closed() {
        let mut cache = VerifiedBundleLruCache::new(2);
        let now = Utc::now();
        cache.insert("sha256:abc".into(), "bundle-1", now - Duration::seconds(1));
        assert!(cache.get_valid("sha256:abc", now).is_none());

        cache.insert("sha256:def".into(), "bundle-2", now + Duration::minutes(5));
        assert_eq!(cache.get_valid("sha256:def", now), Some("bundle-2"));
    }
}
