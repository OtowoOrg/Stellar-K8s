//! Event-driven automation workflow executor.
//!
//! Defines trigger → action rules. When a [`ProcessingEvent`] matches a
//! trigger condition the associated [`AutomationAction`] is executed.

use crate::event_processing::schema::ProcessingEvent;
use crate::event_processing::cep::EventCondition;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

// ── Action types ──────────────────────────────────────────────────────────────

/// What to do when a rule fires
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AutomationAction {
    /// Log a message at INFO level
    Log { message: String },
    /// Emit a new synthetic event back into the pipeline
    EmitEvent {
        event_type: String,
        payload: serde_json::Value,
    },
    /// Call an HTTP endpoint (fire-and-forget)
    HttpCallback {
        url: String,
        method: String,
        body: Option<serde_json::Value>,
    },
    /// Execute a sequence of actions
    Sequence(Vec<AutomationAction>),
}

// ── Rule ──────────────────────────────────────────────────────────────────────

/// A named automation rule: trigger condition → action
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: String,
    pub name: String,
    pub trigger: EventCondition,
    pub action: AutomationAction,
    pub enabled: bool,
    /// Maximum times this rule can fire per minute (0 = unlimited)
    pub rate_limit_per_min: u32,
}

// ── Executor ──────────────────────────────────────────────────────────────────

struct RuleState {
    rule: AutomationRule,
    /// Timestamps of recent firings (for rate limiting)
    fire_times: Vec<std::time::Instant>,
}

impl RuleState {
    fn new(rule: AutomationRule) -> Self {
        Self { rule, fire_times: Vec::new() }
    }

    fn is_rate_limited(&mut self) -> bool {
        if self.rule.rate_limit_per_min == 0 {
            return false;
        }
        let now = std::time::Instant::now();
        let one_min = std::time::Duration::from_secs(60);
        self.fire_times.retain(|t| now.duration_since(*t) < one_min);
        if self.fire_times.len() >= self.rule.rate_limit_per_min as usize {
            return true;
        }
        self.fire_times.push(now);
        false
    }
}

/// Callback type for emitting synthetic events back into the pipeline
pub type EmitFn = Arc<dyn Fn(ProcessingEvent) + Send + Sync>;

/// The automation workflow executor
pub struct AutomationExecutor {
    rules: RwLock<HashMap<String, RuleState>>,
    emit_fn: Option<EmitFn>,
}

impl AutomationExecutor {
    pub fn new(emit_fn: Option<EmitFn>) -> Arc<Self> {
        Arc::new(Self {
            rules: RwLock::new(HashMap::new()),
            emit_fn,
        })
    }

    /// Register or replace a rule.
    pub async fn register_rule(&self, rule: AutomationRule) {
        info!("automation: registering rule '{}' ({})", rule.name, rule.id);
        let mut rules = self.rules.write().await;
        rules.insert(rule.id.clone(), RuleState::new(rule));
    }

    /// Disable a rule by ID.
    pub async fn disable_rule(&self, id: &str) {
        let mut rules = self.rules.write().await;
        if let Some(rs) = rules.get_mut(id) {
            rs.rule.enabled = false;
        }
    }

    /// Process an event against all registered rules.
    pub async fn process(&self, event: &ProcessingEvent) {
        // Collect matching rules first to avoid holding the lock during async execution
        let matching: Vec<(String, AutomationAction)> = {
            let mut rules = self.rules.write().await;
            let mut out = vec![];
            for rs in rules.values_mut() {
                if !rs.rule.enabled || !rs.rule.trigger.matches(event) {
                    continue;
                }
                if rs.is_rate_limited() {
                    warn!("automation: rule '{}' rate-limited", rs.rule.name);
                    continue;
                }
                out.push((rs.rule.name.clone(), rs.rule.action.clone()));
            }
            out
        };

        for (name, action) in matching {
            self.execute_action(&name, &action, event).await;
        }
    }

    async fn execute_action(&self, rule_name: &str, action: &AutomationAction, trigger: &ProcessingEvent) {
        match action {
            AutomationAction::Log { message } => {
                info!("automation[{rule_name}]: {message} (triggered by {})", trigger.event_type);
            }
            AutomationAction::EmitEvent { event_type, payload } => {
                info!("automation[{rule_name}]: emitting {event_type}");
                if let Some(emit) = &self.emit_fn {
                    let ev = ProcessingEvent::new(
                        event_type.clone(),
                        crate::event_processing::schema::EventSource::External("automation".into()),
                        trigger.aggregate_id.clone(),
                        trigger.namespace.clone(),
                        payload.clone(),
                    )
                    .with_causation_id(trigger.id.clone())
                    .with_correlation_id(trigger.correlation_id.clone());
                    emit(ev);
                }
            }
            AutomationAction::HttpCallback { url, method, body } => {
                info!("automation[{rule_name}]: HTTP {method} {url}");
                let client = reqwest::Client::new();
                let req = match method.to_uppercase().as_str() {
                    "POST" => client.post(url),
                    "PUT" => client.put(url),
                    _ => client.get(url),
                };
                let req = if let Some(b) = body { req.json(b) } else { req };
                if let Err(e) = req.send().await {
                    error!("automation[{rule_name}]: HTTP callback failed: {e}");
                }
            }
            AutomationAction::Sequence(actions) => {
                for a in actions {
                    self.execute_action(rule_name, a, trigger).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_processing::schema::EventSource;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn log_action_fires() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = counter.clone();
        let emit: EmitFn = Arc::new(move |_| { c2.fetch_add(1, Ordering::SeqCst); });

        let exec = AutomationExecutor::new(Some(emit));
        exec.register_rule(AutomationRule {
            id: "r1".into(),
            name: "test-rule".into(),
            trigger: EventCondition {
                event_type_prefix: "stellar.node".into(),
                source_filter: None,
                min_severity: None,
                required_labels: HashMap::new(),
            },
            action: AutomationAction::EmitEvent {
                event_type: "stellar.automation.triggered".into(),
                payload: serde_json::json!({}),
            },
            enabled: true,
            rate_limit_per_min: 0,
        })
        .await;

        let ev = ProcessingEvent::new(
            "stellar.node.created",
            EventSource::Controller,
            "agg",
            "ns",
            serde_json::json!({}),
        );
        exec.process(&ev).await;
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
