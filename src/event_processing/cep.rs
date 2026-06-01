//! Complex Event Pattern (CEP) detection engine.
//!
//! Detects multi-event patterns such as sequences, thresholds, and temporal
//! windows over a stream of [`ProcessingEvent`]s.

use crate::event_processing::schema::{EventSeverity, ProcessingEvent};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info};

// ── Pattern definitions ───────────────────────────────────────────────────────

/// A single condition that must match an event
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventCondition {
    /// Required event_type prefix (empty = any)
    pub event_type_prefix: String,
    /// Required source (None = any)
    pub source_filter: Option<String>,
    /// Minimum severity (None = any)
    pub min_severity: Option<EventSeverity>,
    /// Required label key=value pairs
    pub required_labels: HashMap<String, String>,
}

impl EventCondition {
    pub fn matches(&self, event: &ProcessingEvent) -> bool {
        if !self.event_type_prefix.is_empty()
            && !event.event_type.starts_with(&self.event_type_prefix)
        {
            return false;
        }
        if let Some(src) = &self.source_filter {
            if event.source.to_string() != *src {
                return false;
            }
        }
        if let Some(min_sev) = &self.min_severity {
            if event.severity < *min_sev {
                return false;
            }
        }
        for (k, v) in &self.required_labels {
            if event.labels.get(k).map(|s| s.as_str()) != Some(v.as_str()) {
                return false;
            }
        }
        true
    }
}

/// How a pattern is evaluated
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PatternKind {
    /// All conditions must fire in order within the window
    Sequence,
    /// Any single condition fires
    Any,
    /// A condition fires N or more times within the window
    Threshold { condition: EventCondition, count: usize },
    /// A condition fires and then another does NOT fire within the window
    Absence { trigger: EventCondition, absent: EventCondition },
}

/// A named CEP pattern
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CepPattern {
    pub id: String,
    pub name: String,
    pub kind: PatternKind,
    /// Conditions for Sequence / Any patterns
    pub conditions: Vec<EventCondition>,
    /// Time window in seconds
    pub window_secs: u64,
    pub description: String,
}

/// A detected pattern match
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatternMatch {
    pub pattern_id: String,
    pub pattern_name: String,
    pub matched_events: Vec<String>, // event IDs
    pub detected_at: DateTime<Utc>,
    pub description: String,
}

// ── Engine ────────────────────────────────────────────────────────────────────

struct PatternState {
    pattern: CepPattern,
    /// Events seen within the current window (oldest first)
    window: VecDeque<ProcessingEvent>,
    /// For Sequence: index of the next condition to match
    seq_index: usize,
    /// For Threshold: count of matching events in window
    threshold_count: usize,
}

impl PatternState {
    fn new(pattern: CepPattern) -> Self {
        Self {
            pattern,
            window: VecDeque::new(),
            seq_index: 0,
            threshold_count: 0,
        }
    }

    fn evict_old(&mut self, now: DateTime<Utc>) {
        let cutoff = now - Duration::seconds(self.pattern.window_secs as i64);
        while let Some(front) = self.window.front() {
            if front.timestamp < cutoff {
                self.window.pop_front();
            } else {
                break;
            }
        }
    }

    /// Feed one event; return a match if the pattern fires.
    fn feed(&mut self, event: &ProcessingEvent) -> Option<PatternMatch> {
        let now = Utc::now();
        self.evict_old(now);
        self.window.push_back(event.clone());

        match &self.pattern.kind.clone() {
            PatternKind::Any => {
                for cond in &self.pattern.conditions {
                    if cond.matches(event) {
                        return Some(self.make_match(vec![event.id.clone()]));
                    }
                }
                None
            }
            PatternKind::Sequence => {
                let conditions = self.pattern.conditions.clone();
                if self.seq_index < conditions.len()
                    && conditions[self.seq_index].matches(event)
                {
                    self.seq_index += 1;
                    if self.seq_index == conditions.len() {
                        self.seq_index = 0;
                        let ids: Vec<_> = self.window.iter().map(|e| e.id.clone()).collect();
                        return Some(self.make_match(ids));
                    }
                }
                None
            }
            PatternKind::Threshold { condition, count } => {
                if condition.matches(event) {
                    self.threshold_count += 1;
                }
                if self.threshold_count >= *count {
                    self.threshold_count = 0;
                    let ids: Vec<_> = self.window.iter().map(|e| e.id.clone()).collect();
                    return Some(self.make_match(ids));
                }
                None
            }
            PatternKind::Absence { trigger, absent } => {
                if trigger.matches(event) {
                    // Check if the absent condition has NOT fired in the window
                    let absent_fired = self.window.iter().any(|e| absent.matches(e));
                    if !absent_fired {
                        return Some(self.make_match(vec![event.id.clone()]));
                    }
                }
                None
            }
        }
    }

    fn make_match(&self, ids: Vec<String>) -> PatternMatch {
        PatternMatch {
            pattern_id: self.pattern.id.clone(),
            pattern_name: self.pattern.name.clone(),
            matched_events: ids,
            detected_at: Utc::now(),
            description: self.pattern.description.clone(),
        }
    }
}

/// The CEP engine: register patterns, feed events, receive matches.
pub struct CepEngine {
    states: RwLock<HashMap<String, PatternState>>,
    match_tx: broadcast::Sender<PatternMatch>,
}

impl CepEngine {
    pub fn new() -> Arc<Self> {
        let (match_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            states: RwLock::new(HashMap::new()),
            match_tx,
        })
    }

    /// Register a new pattern. Replaces any existing pattern with the same ID.
    pub async fn register_pattern(&self, pattern: CepPattern) {
        info!("CEP: registering pattern '{}' ({})", pattern.name, pattern.id);
        let mut states = self.states.write().await;
        states.insert(pattern.id.clone(), PatternState::new(pattern));
    }

    /// Feed an event through all registered patterns.
    pub async fn process(&self, event: &ProcessingEvent) {
        let mut states = self.states.write().await;
        for state in states.values_mut() {
            if let Some(m) = state.feed(event) {
                debug!("CEP match: pattern='{}' events={:?}", m.pattern_name, m.matched_events);
                let _ = self.match_tx.send(m);
            }
        }
    }

    /// Subscribe to pattern matches.
    pub fn subscribe_matches(&self) -> broadcast::Receiver<PatternMatch> {
        self.match_tx.subscribe()
    }
}

impl Default for CepEngine {
    fn default() -> Self {
        let (match_tx, _) = broadcast::channel(256);
        Self {
            states: RwLock::new(HashMap::new()),
            match_tx,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;

    fn make_event(event_type: &str) -> ProcessingEvent {
        ProcessingEvent::new(event_type, EventSource::Controller, "agg", "ns", serde_json::json!({}))
    }

    #[tokio::test]
    async fn threshold_pattern_fires() {
        let engine = CepEngine::new();
        let mut rx = engine.subscribe_matches();

        engine
            .register_pattern(CepPattern {
                id: "p1".into(),
                name: "disk-warn-3x".into(),
                kind: PatternKind::Threshold {
                    condition: EventCondition {
                        event_type_prefix: "stellar.disk".into(),
                        source_filter: None,
                        min_severity: None,
                        required_labels: HashMap::new(),
                    },
                    count: 3,
                },
                conditions: vec![],
                window_secs: 60,
                description: "3 disk events".into(),
            })
            .await;

        for _ in 0..3 {
            engine.process(&make_event("stellar.disk.warning")).await;
        }

        let m = rx.try_recv().unwrap();
        assert_eq!(m.pattern_id, "p1");
    }

    #[tokio::test]
    async fn sequence_pattern_fires() {
        let engine = CepEngine::new();
        let mut rx = engine.subscribe_matches();

        let cond = |prefix: &str| EventCondition {
            event_type_prefix: prefix.into(),
            source_filter: None,
            min_severity: None,
            required_labels: HashMap::new(),
        };

        engine
            .register_pattern(CepPattern {
                id: "p2".into(),
                name: "create-then-delete".into(),
                kind: PatternKind::Sequence,
                conditions: vec![cond("stellar.node.created"), cond("stellar.node.deleted")],
                window_secs: 60,
                description: "node created then deleted".into(),
            })
            .await;

        engine.process(&make_event("stellar.node.created")).await;
        engine.process(&make_event("stellar.node.deleted")).await;

        let m = rx.try_recv().unwrap();
        assert_eq!(m.pattern_id, "p2");
    }
}
