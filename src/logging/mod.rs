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
//! Structured Logging and Analytics Module
//!
//! This module provides a consistent schema for structured logs, intelligent
//! sampling, and hooks for log analytics.

pub mod alerting;
pub mod analytics;
/// Standardised log field name constants (Issue #1115).
///
/// Import as `use stellar_k8s::logging::fields as F;` and reference
/// `F::NODE`, `F::NAMESPACE`, etc. in every `tracing::*!` call so
/// field names stay consistent across CI pipelines and runtime diagnostics.
pub mod fields;
pub mod sampling;
pub mod storage;
pub mod subscriber;

pub use subscriber::{
    init_binary_subscriber, init_subscriber, LogOutputFormat, SubscriberConfig, SubscriberGuard,
    SubscriberInit,
};

use analytics::AnalyticsEngine;
use chrono::Utc;
use sampling::{Sampler, SamplingConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{Event, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

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
    /// Request correlation ID across service boundaries
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
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
        trace_id: crate::telemetry::current_trace_context().map(|(tid, _)| tid),
        span_id: crate::telemetry::current_trace_context().map(|(_, sid)| sid),
        k8s_node: std::env::var("K8S_NODE_NAME").ok(),
        k8s_namespace: std::env::var("K8S_NAMESPACE").ok(),
        reconcile_id: visitor
            .extras
            .get("reconcile_id")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        correlation_id: visitor
            .extras
            .get("correlation_id")
            .or_else(|| visitor.extras.get("x_correlation_id"))
            .and_then(|v| v.as_str().map(|s| s.to_string())),
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
            self.extras.insert(
                field.name().to_string(),
                serde_json::json!(format!("{:?}", value)),
            );
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.extras.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.extras
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.extras
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.extras
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.extras
            .insert(field.name().to_string(), serde_json::json!(value));
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        self.extras.insert(
            field.name().to_string(),
            serde_json::json!(value.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structured_log_serialization() {
        let mut extras = HashMap::new();
        extras.insert("component".to_string(), serde_json::json!("controller"));
        extras.insert("duration_ms".to_string(), serde_json::json!(42));

        let log = StructuredLog {
            timestamp: "2026-07-26T10:00:00Z".to_string(),
            level: "INFO".to_string(),
            message: "Reconciliation successful".to_string(),
            target: "stellar_k8s::controller".to_string(),
            module: Some("stellar_k8s::controller".to_string()),
            file: Some("src/controller/mod.rs".to_string()),
            line: Some(100),
            trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string()),
            span_id: Some("00f067aa0ba902b7".to_string()),
            k8s_node: Some("node-1".to_string()),
            k8s_namespace: Some("default".to_string()),
            reconcile_id: Some("rec-123".to_string()),
            correlation_id: Some("corr-456".to_string()),
            extras,
        };

        let json_str = serde_json::to_string(&log).expect("Failed to serialize StructuredLog");
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("Failed to parse JSON");

        assert_eq!(parsed["level"], "INFO");
        assert_eq!(parsed["message"], "Reconciliation successful");
        assert_eq!(parsed["target"], "stellar_k8s::controller");
        assert_eq!(parsed["component"], "controller");
        assert_eq!(parsed["duration_ms"], 42);
        assert_eq!(parsed["reconcile_id"], "rec-123");
    }

    #[test]
    fn test_structured_log_deserialization_roundtrip() {
        let mut extras = HashMap::new();
        extras.insert("custom_key".to_string(), serde_json::json!("custom_value"));

        let log = StructuredLog {
            timestamp: Utc::now().to_rfc3339(),
            level: "WARN".to_string(),
            message: "High memory usage detected".to_string(),
            target: "stellar_k8s::monitoring".to_string(),
            module: None,
            file: None,
            line: None,
            trace_id: None,
            span_id: None,
            k8s_node: None,
            k8s_namespace: None,
            reconcile_id: None,
            correlation_id: None,
            extras,
        };

        let json = serde_json::to_string(&log).unwrap();
        let log_back: StructuredLog = serde_json::from_str(&json).unwrap();

        assert_eq!(log_back.level, "WARN");
        assert_eq!(log_back.message, "High memory usage detected");
        assert_eq!(log_back.extras.get("custom_key").unwrap(), "custom_value");
    }
}
