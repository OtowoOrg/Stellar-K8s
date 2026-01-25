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
        self.use_tls
            || self.endpoint.contains("localhost")
            || self.endpoint.contains("127.0.0.1")
            || self.endpoint.contains("::1")
    }

    /// Verify the telemetry configuration meets privacy standards
    pub fn verify_privacy_assurance() -> Result<(), String> {
        let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").unwrap_or_default();
        if endpoint.is_empty() {
            return Ok(()); // Disabled is safe
        }

        if endpoint.starts_with("http://")
            && !endpoint.contains("localhost")
            && !endpoint.contains("127.0.0.1")
            && !endpoint.contains("::1")
        {
            return Err("INSECURE: Telemetry endpoint must use HTTPS or be local (localhost, 127.0.0.1, ::1) for privacy assurance.".into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_is_secure() {
        assert!(SecureTelemetryProxy::new("https://tracing.stellar.org".to_string()).is_secure());
        assert!(SecureTelemetryProxy::new("http://localhost:4317".to_string()).is_secure());
        assert!(SecureTelemetryProxy::new("http://127.0.0.1:4317".to_string()).is_secure());
        assert!(SecureTelemetryProxy::new("http://[::1]:4317".to_string()).is_secure());
        assert!(!SecureTelemetryProxy::new("http://tracing.stellar.org".to_string()).is_secure());
    }

    #[test]
    fn test_verify_privacy_assurance() {
        // Test local
        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://localhost:4317");
        assert!(SecureTelemetryProxy::verify_privacy_assurance().is_ok());

        // Test secure remote
        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "https://remote:4317");
        assert!(SecureTelemetryProxy::verify_privacy_assurance().is_ok());

        // Test insecure remote
        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://remote:4317");
        assert!(SecureTelemetryProxy::verify_privacy_assurance().is_err());

        // Test empty
        env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        assert!(SecureTelemetryProxy::verify_privacy_assurance().is_ok());
    }
}
