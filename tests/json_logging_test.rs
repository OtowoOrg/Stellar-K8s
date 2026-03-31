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
