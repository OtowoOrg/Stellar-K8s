//! Log Retention and Archival Policies
//!
//! Manages log lifecycle and archival settings.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// How many days to keep logs in hot storage (e.g., Loki/Elasticsearch)
    pub hot_retention_days: u32,
    /// How many days to keep logs in cold storage (e.g., S3)
    pub cold_retention_days: u32,
    /// Whether to compress logs before archival
    pub compress_archival: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            hot_retention_days: 7,
            cold_retention_days: 90,
            compress_archival: true,
        }
    }
}

pub struct Archiver {
    pub policy: RetentionPolicy,
    pub s3_bucket: String,
}

impl Archiver {
    pub fn new(policy: RetentionPolicy, s3_bucket: String) -> Self {
        Self { policy, s3_bucket }
    }

    pub fn get_archival_path(
        &self,
        node_name: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> String {
        format!(
            "stellar-logs/{}/{}/{}.log.gz",
            node_name,
            timestamp.format("%Y-%m-%d"),
            timestamp.format("%H-%M-%S")
        )
    }

    pub fn get_cost_recommendations(
        &self,
        patterns: &[crate::logging::analytics::LogPattern],
    ) -> Vec<String> {
        let mut recs = Vec::new();

        for pattern in patterns {
            if pattern.count > 100_000 {
                recs.push(format!(
                    "High volume pattern detected: '{}'. Consider increasing sampling rate for target containing this pattern.",
                    pattern.message_template
                ));
            }
        }

        if self.policy.hot_retention_days > 30 {
            recs.push("Hot storage retention is > 30 days. Consider reducing hot retention and relying on S3 archival to save costs.".to_string());
        }

        recs
    }
}
