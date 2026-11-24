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
use std::io;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tracing::{info, info_span};
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct SharedBufferWriter {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl io::Write for SharedBufferWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf
            .lock()
            .expect("log buffer lock poisoned")
            .extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn json_contains_field(v: &Value, key: &str) -> bool {
    if v.get("span").and_then(|s| s.get(key)).is_some() {
        return true;
    }

    v.get("spans")
        .and_then(|spans| spans.as_array())
        .map(|arr| arr.iter().any(|s| s.get(key).is_some()))
        .unwrap_or(false)
}

#[test]
fn json_log_output_contains_node_namespace_reconcile_id_fields() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));

    let make_writer = {
        let buf = buf.clone();
        move || SharedBufferWriter { buf: buf.clone() }
    };

    let fmt_layer = fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(true)
        .with_target(true)
        .with_writer(make_writer);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("info"))
        .with(fmt_layer);

    let _guard = tracing::subscriber::set_default(subscriber);

    let span = info_span!(
        "reconcile_attempt",
        node_name = "node-1",
        namespace = "ns-1",
        reconcile_id = 123_u64
    );
    let _enter = span.enter();
    info!("hello");

    let buf_guard = buf.lock().expect("lock poisoned");
    let output = String::from_utf8_lossy(&buf_guard);
    let first_line = output
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("expected at least one JSON log line");

    let v: Value = serde_json::from_str(first_line).expect("log line should be valid JSON");

    for key in ["node_name", "namespace", "reconcile_id"] {
        assert!(
            json_contains_field(&v, key),
            "expected JSON log to contain field '{key}' in span context, got: {v}"
        );
    }

    assert!(
        v.get("message").is_some() || v.get("fields").and_then(|f| f.get("message")).is_some(),
        "expected JSON log to contain an event message, got: {v}"
    );
}

// ── Test: error field is emitted as Display, not Debug ─────────────────────────

/// Issue #933: When `error` is logged with `%err` (Display), the JSON value
/// must be the Display form (no extra quotes or `<ErrorType>` Debug prefix).
#[test]
fn json_log_error_field_is_display_not_debug() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));

    let make_writer = {
        let buf = buf.clone();
        move || SharedBufferWriter { buf: buf.clone() }
    };

    let fmt_layer = fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_writer(make_writer);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("info"))
        .with(fmt_layer);

    let _guard = tracing::subscriber::set_default(subscriber);

    // Emit an error event using %err (Display trait).
    let err_msg = "connection refused";
    tracing::error!(error = %err_msg, "operation failed");

    let buf_guard = buf.lock().expect("lock poisoned");
    let output = String::from_utf8_lossy(&buf_guard);
    let first_line = output
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("expected at least one JSON log line");

    let v: Value = serde_json::from_str(first_line).expect("log line should be valid JSON");

    let error_val = v
        .get("error")
        .expect("expected 'error' field in JSON log output");

    // The value must be a plain string (Display), not a debug-wrapped string.
    assert!(
        error_val.is_string(),
        "expected 'error' to be a JSON string, got: {error_val}"
    );
    assert_eq!(
        error_val.as_str().unwrap(),
        err_msg,
        "expected 'error' Display value; got: {error_val}"
    );
}

// ── Test: duration_ms is serialized as a number, not a string ─────────────────

/// Issue #933: `duration_ms` must be a JSON number so dashboards can aggregate
/// it directly (e.g. `avg(duration_ms)`).
#[test]
fn json_log_duration_ms_is_numeric() {
    let buf = Arc::new(Mutex::new(Vec::<u8>::new()));

    let make_writer = {
        let buf = buf.clone();
        move || SharedBufferWriter { buf: buf.clone() }
    };

    let fmt_layer = fmt::layer()
        .json()
        .flatten_event(true)
        .with_current_span(false)
        .with_writer(make_writer);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::new("info"))
        .with(fmt_layer);

    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::info!(duration_ms = 42_u64, "reconcile complete");

    let buf_guard = buf.lock().expect("lock poisoned");
    let output = String::from_utf8_lossy(&buf_guard);
    let first_line = output
        .lines()
        .find(|l| !l.trim().is_empty())
        .expect("expected at least one JSON log line");

    let v: Value = serde_json::from_str(first_line).expect("log line should be valid JSON");

    let dur_val = v
        .get("duration_ms")
        .expect("expected 'duration_ms' field in JSON log output");

    assert!(
        dur_val.is_number(),
        "expected 'duration_ms' to be a JSON number (not a string), got: {dur_val}"
    );
    assert_eq!(
        dur_val.as_u64().unwrap(),
        42,
        "expected duration_ms == 42, got: {dur_val}"
    );
}
