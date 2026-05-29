//! Configuration Versioning and History Tracking
//!
//! Tracks changes to configurations and maintains a versioned history.

use crate::crd::StellarNodeSpec;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigVersion {
    pub version: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub spec: StellarNodeSpec,
    pub hash: String,
}

pub struct VersionManager {
    history: VecDeque<ConfigVersion>,
    max_history: usize,
}

impl VersionManager {
    pub fn new(max_history: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(max_history),
            max_history,
        }
    }

    pub fn push(&mut self, spec: StellarNodeSpec) -> u64 {
        let version = (self.history.len() as u64) + 1;
        let hash = self.calculate_hash(&spec);

        let new_version = ConfigVersion {
            version,
            timestamp: chrono::Utc::now(),
            spec,
            hash,
        };

        if self.history.len() >= self.max_history {
            self.history.pop_front();
        }
        self.history.push_back(new_version);
        version
    }

    pub fn get_latest(&self) -> Option<&ConfigVersion> {
        self.history.back()
    }

    pub fn get_version(&self, version: u64) -> Option<&ConfigVersion> {
        self.history.iter().find(|v| v.version == version)
    }

    fn calculate_hash(&self, spec: &StellarNodeSpec) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        // Simplified hash - in reality, we'd hash the serialized JSON
        let json = serde_json::to_string(spec).unwrap_or_default();
        json.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}
