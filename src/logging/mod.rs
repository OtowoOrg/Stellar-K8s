//! Structured Logging and Analytics Module
//!
//! This module provides a consistent schema for structured logs, intelligent
//! sampling, and hooks for log analytics.

pub mod analytics;
pub mod sampling;
pub mod alerting;
pub mod storage;

use analytics::AnalyticsEngine;
use sampling::{Sampler, SamplingConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};
use chrono::Utc;

/// Consistent schema for all logs in Stellar-K8s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredLog {
    /// RFC3339 timestamp
    pub timestamp: String,
    /// Log level (INFO, WARN, ERROR, etc.)
    pub level: String,
    /// Main log message
    pub message: String,
    /// Tracing target
    pub target: String,
    /// Rust module path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// Source file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Line number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// OpenTelemetry Trace ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// OpenTelemetry Span ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    /// Kubernetes Node Name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k8s_node: Option<String>,
    /// Kubernetes Namespace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub k8s_namespace: Option<String>,
    /// Controller reconcile ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reconcile_id: Option<String>,
    /// Arbitrary additional context
    #[serde(flatten)]
    pub extras: HashMap<String, serde_json::Value>,
}

/// A layer that enforces the `StructuredLog` schema and performs intelligent sampling.
pub struct AnalyticsLayer {
    sampler: Sampler,
    engine: Arc<AnalyticsEngine>,
}

impl AnalyticsLayer {
    pub fn new(sampling_config: SamplingConfig, engine: Arc<AnalyticsEngine>) -> Self {
        Self {
            sampler: Sampler::new(sampling_config),
            engine,
        }
    }
}

impl<S> Layer<S> for AnalyticsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();

        // 1. Intelligent Sampling
        if !self.sampler.should_sample(metadata) {
            return;
        }

        // 2. Pattern Detection & Analytics
        // Extract message for analytics (simplified for now)
        // In a real implementation, we'd use a Visitor to get the message field
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        
        if let Some(msg) = &visitor.message {
            self.engine.observe(msg);
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

/// Helper to build the structured log object from a tracing event
pub fn build_structured_log(event: &Event<'_>) -> StructuredLog {
    let metadata = event.metadata();
    let mut visitor = FullVisitor::default();
    event.record(&mut visitor);

    StructuredLog {
        timestamp: Utc::now().to_rfc3339(),
        level: metadata.level().to_string(),
        message: visitor.message.unwrap_or_default(),
        target: metadata.target().to_string(),
        module: metadata.module_path().map(|s| s.to_string()),
        file: metadata.file().map(|s| s.to_string()),
        line: metadata.line(),
        trace_id: None, // Injected by OtelTraceIdLayer
        span_id: None,
        k8s_node: std::env::var("K8S_NODE_NAME").ok(),
        k8s_namespace: std::env::var("K8S_NAMESPACE").ok(),
        reconcile_id: visitor.extras.get("reconcile_id").and_then(|v| v.as_str().map(|s| s.to_string())),
        extras: visitor.extras,
    }
}

#[derive(Default)]
struct FullVisitor {
    message: Option<String>,
    extras: HashMap<String, serde_json::Value>,
}

impl tracing::field::Visit for FullVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{:?}", value));
        } else {
            self.extras.insert(field.name().to_string(), serde_json::json!(format!("{:?}", value)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.extras.insert(field.name().to_string(), serde_json::Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.extras.insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.extras.insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.extras.insert(field.name().to_string(), serde_json::json!(value));
    }
}
