//! Log Analytics Engine
//!
//! Provides pattern detection and log-based insights.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A detected log pattern
#[derive(Debug, Clone)]
pub struct LogPattern {
    pub message_template: String,
    pub count: u64,
    pub first_seen: Instant,
    pub last_seen: Instant,
}

pub struct AnalyticsEngine {
    /// Map of pattern hash to detected pattern
    patterns: Arc<Mutex<HashMap<u64, LogPattern>>>,
    /// How often to clear patterns (to avoid memory leaks)
    retention: Duration,
}

impl AnalyticsEngine {
    pub fn new(retention: Duration) -> Self {
        Self {
            patterns: Arc::new(Mutex::new(HashMap::new())),
            retention,
        }
    }

    /// Process a log message and update pattern statistics
    pub fn observe(&self, message: &str) {
        let template = self.generalize(message);
        let hash = self.calculate_hash(&template);

        let mut patterns = self.patterns.lock().unwrap();
        let entry = patterns.entry(hash).or_insert_with(|| LogPattern {
            message_template: template,
            count: 0,
            first_seen: Instant::now(),
            last_seen: Instant::now(),
        });

        entry.count += 1;
        entry.last_seen = Instant::now();
    }

    /// Simplify a log message into a template (e.g., replace IDs/numbers with placeholders)
    fn generalize(&self, message: &str) -> String {
        let mut result = String::with_capacity(message.len());
        let mut in_number = false;

        for c in message.chars() {
            if c.is_ascii_digit() {
                if !in_number {
                    result.push_str("<num>");
                    in_number = true;
                }
            } else {
                result.push(c);
                in_number = false;
            }
        }
        result
    }

    fn calculate_hash(&self, s: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    pub fn get_top_patterns(&self, limit: usize) -> Vec<LogPattern> {
        let patterns = self.patterns.lock().unwrap();
        let mut p: Vec<LogPattern> = patterns.values().cloned().collect();
        p.sort_by(|a, b| b.count.cmp(&a.count));
        p.into_iter().take(limit).collect()
    }
}
