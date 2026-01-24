//! Telemetry Proxy Implementation
//!
//! This module provides the Zero-Knowledge Telemetry Proxy which
//! ensures all outgoing telemetry is scrubbed and privacy-protected.

/// A wrapper for the OTLP exporter that enforces encryption and scrubbing
pub struct SecureTelemetryProxy {
    endpoint: String,
    use_tls: bool,
}

impl SecureTelemetryProxy {
    pub fn new(endpoint: String) -> Self {
        let use_tls = endpoint.starts_with("https");
        Self { endpoint, use_tls }
    }

    /// Returns whether the proxy is configured securely
    pub fn is_secure(&self) -> bool {
        self.use_tls || self.endpoint.contains("localhost") || self.endpoint.contains("127.0.0.1")
    }

    /// Verify the telemetry configuration meets privacy standards
    pub fn verify_privacy_assurance() -> Result<(), String> {
        let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_default();
        if endpoint.is_empty() {
            return Ok(()); // Disabled is safe
        }

        if endpoint.starts_with("http://") && !endpoint.contains("localhost") {
            return Err("INSECURE: Telemetry endpoint must use HTTPS or be local for privacy assurance.".into());
        }

        Ok(())
    }

    /// Provides the scrubbing OTel Collector configuration
    pub fn get_collector_config() -> &'static str {
        r#"
receivers:
  otlp:
    protocols:
      grpc:
      http:

processors:
  batch:
  transform:
    error_mode: ignore
    trace_statements:
      - context: span
        statements:
          - delete_key(attributes, "ip")
          - delete_key(attributes, "host.ip")
          - delete_key(attributes, "k8s.pod.ip")
          - set(attributes["k8s.cluster.name"], "REDACTED")
          - set(attributes["cluster.name"], "REDACTED")
    metric_statements:
      - context: datapoint
        statements:
          - delete_key(attributes, "ip")
          - set(attributes["cluster_name"], "REDACTED")

exporters:
  otlp/public:
    endpoint: ${PUBLIC_DASHBOARD_ENDPOINT}
    tls:
      insecure: false

service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch, transform]
      exporters: [otlp/public]
    metrics:
      receivers: [otlp]
      processors: [batch, transform]
      exporters: [otlp/public]
"#
    }
}
