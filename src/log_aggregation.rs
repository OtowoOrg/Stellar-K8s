//! Log Aggregation with Structured Logging Pipeline
//! 
//! This module provides:
//! - Fluentd/Fluentbit DaemonSet configuration
//! - Structured JSON logging format for all services
//! - Log-based alerts for error patterns and anomalies
//! - Centralized log collection and querying

use chrono::{DateTime, Utc};
use kube::{
    api::{Api, ListParams, PostParams},
    Client, ResourceExt,
};
use kube::api::ObjectMeta;
use kube::core::DynamicObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn, error};

/// Fluentbit DaemonSet configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluentbitConfig {
    /// Fluentbit image
    pub image: String,
    /// Namespace for DaemonSet
    pub namespace: String,
    /// Service account name
    pub service_account: String,
    /// Log output destinations
    pub outputs: Vec<LogOutput>,
    /// Input configuration
    pub inputs: Vec<LogInput>,
    /// Filter configuration
    pub filters: Vec<LogFilter>,
    /// Buffer configuration
    pub buffer: BufferConfig,
    /// Resource limits
    pub resources: ResourceRequirements,
    /// Tolerations for scheduling
    pub tolerations: Vec<Toleration>,
    /// Node selector
    pub node_selector: HashMap<String, String>,
}

/// Log output destination
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogOutput {
    /// Output name
    pub name: String,
    /// Output type (elasticsearch, loki, kafka, stdout, etc.)
    pub output_type: OutputType,
    /// Destination endpoint
    pub endpoint: String,
    /// Authentication
    pub auth: Option<AuthConfig>,
    /// TLS configuration
    pub tls: Option<TlsConfig>,
    /// Additional options
    pub options: HashMap<String, String>,
}

/// Output types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputType {
    Elasticsearch,
    Loki,
    Kafka,
    Stdout,
    File,
    Splunk,
    Datadog,
    NewRelic,
    CloudWatch,
    AzureLogAnalytics,
}

/// Authentication configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub username: Option<String>,
    pub password: Option<String>,
    pub token: Option<String>,
    pub api_key: Option<String>,
}

/// TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub ca_file: Option<String>,
    pub cert_file: Option<String>,
    pub key_file: Option<String>,
    pub verify: bool,
}

/// Log input configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogInput {
    /// Input name
    pub name: String,
    /// Input type
    pub input_type: InputType,
    /// Path for file inputs
    pub path: Option<String>,
    /// Tag for logs
    pub tag: String,
    /// Additional options
    pub options: HashMap<String, String>,
}

/// Input types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputType {
    Tail,
    Systemd,
    Kubernetes,
    Tcp,
    Udp,
    Http,
}

/// Log filter configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFilter {
    /// Filter name
    pub name: String,
    /// Filter type
    pub filter_type: FilterType,
    /// Match pattern
    pub match_pattern: String,
    /// Filter rules
    pub rules: Vec<FilterRule>,
}

/// Filter types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterType {
    Grep,
    RecordModifier,
    Nest,
    Modify,
    StandardOutput,
}

/// Filter rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterRule {
    pub key: String,
    pub regex: Option<String>,
    pub action: FilterAction,
    pub value: Option<String>,
}

/// Filter actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterAction {
    Keep,
    Drop,
    Replace,
    Add,
}

/// Buffer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferConfig {
    pub memory: Option<String>,
    pub storage: Option<String>,
    pub chunk_size: Option<String>,
    pub flush_interval: Option<String>,
    pub flush_timeout: Option<String>,
}

/// Resource requirements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceRequirements {
    pub limits: HashMap<String, String>,
    pub requests: HashMap<String, String>,
}

/// Toleration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Toleration {
    pub key: Option<String>,
    pub operator: String,
    pub value: Option<String>,
    pub effect: String,
    pub toleration_seconds: Option<i64>,
}

/// Structured log entry for Fluentbit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredLogEntry {
    /// Timestamp in RFC3339 format
    pub timestamp: String,
    /// Log level
    pub level: String,
    /// Log message
    pub message: String,
    /// Service name
    pub service: String,
    /// Kubernetes metadata
    pub kubernetes: Option<KubernetesMetadata>,
    /// Trace context
    pub trace: Option<TraceContext>,
    /// Additional fields
    #[serde(flatten)]
    pub extras: HashMap<String, Value>,
}

/// Kubernetes metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesMetadata {
    pub pod_name: String,
    pub namespace: String,
    pub pod_uid: String,
    pub container_name: String,
    pub container_id: String,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub host: String,
}

/// Trace context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub trace_flags: Option<String>,
}

/// Log aggregation manager
pub struct LogAggregationManager {
    client: Client,
    config: FluentbitConfig,
    daemonset_name: String,
}

impl LogAggregationManager {
    /// Create a new log aggregation manager
    pub fn new(config: FluentbitConfig, client: Client) -> Arc<Self> {
        Arc::new(Self {
            client,
            config,
            daemonset_name: "fluentbit".to_string(),
        })
    }

    /// Deploy Fluentbit DaemonSet
    pub async fn deploy(&self) -> Result<(), anyhow::Error> {
        let daemonset = self.generate_daemonset()?;
        
        let ds_api: Api<DynamicObject> = Api::namespaced(
            self.client.clone(),
            &self.config.namespace,
        );
        
        let pp = PostParams::default();
        match ds_api.create(&pp, &daemonset).await {
            Ok(_) => {
                info!("Fluentbit DaemonSet deployed successfully");
                Ok(())
            }
            Err(kube::Error::Api(e)) if e.code == 409 => {
                // Already exists, try to update
                let pp = PatchParams::default();
                ds_api.patch(&self.daemonset_name, &pp, &kube::api::Patch::Apply(&daemonset)).await?;
                info!("Fluentbit DaemonSet updated");
                Ok(())
            }
            Err(e) => Err(anyhow::anyhow!("Failed to deploy Fluentbit DaemonSet: {}", e)),
        }
    }

    /// Generate Fluentbit DaemonSet manifest
    fn generate_daemonset(&self) -> Result<DynamicObject, anyhow::Error> {
        let mut daemonset = DynamicObject::new("fluentbit", "apps/v1", "DaemonSet");
        
        // Metadata
        daemonset.metadata = ObjectMeta {
            name: Some(self.daemonset_name.clone()),
            namespace: Some(self.config.namespace.clone()),
            labels: Some({
                let mut labels = HashMap::new();
                labels.insert("app".to_string(), "fluentbit".to_string());
                labels.insert("component".to_string(), "logging".to_string());
                labels
            }),
            annotations: Some({
                let mut annotations = HashMap::new();
                annotations.insert(
                    "prometheus.io/scrape".to_string(),
                    "true".to_string()
                );
                annotations.insert(
                    "prometheus.io/port".to_string(),
                    "2020".to_string()
                );
                annotations
            }),
        };

        // Spec
        let spec = serde_json::json!({
            "selector": {
                "matchLabels": {
                    "app": "fluentbit"
                }
            },
            "template": {
                "metadata": {
                    "labels": {
                        "app": "fluentbit"
                    }
                },
                "spec": {
                    "serviceAccountName": self.config.service_account,
                    "tolerations": self.config.tolerations,
                    "nodeSelector": self.config.node_selector,
                    "containers": [{
                        "name": "fluentbit",
                        "image": self.config.image,
                        "imagePullPolicy": "IfNotPresent",
                        "ports": [
                            {"containerPort": 2020, "name": "metrics"},
                            {"containerPort": 24224, "name": "forward"}
                        ],
                        "env": [
                            {"name": "FLUENTBIT_CONFIG", "value": "/fluent-bit/etc/fluent-bit.conf"},
                            {"name": "K8S_NODE_NAME", "valueFrom": {"fieldRef": {"fieldPath": "spec.nodeName"}}},
                            {"name": "K8S_POD_NAME", "valueFrom": {"fieldRef": {"fieldPath": "metadata.name"}}},
                            {"name": "K8S_POD_NAMESPACE", "valueFrom": {"fieldRef": {"fieldPath": "metadata.namespace"}}},
                            {"name": "K8S_POD_UID", "valueFrom": {"fieldRef": {"fieldPath": "metadata.uid"}}},
                        ],
                        "volumeMounts": [
                            {"name": "config", "mountPath": "/fluent-bit/etc/"},
                            {"name": "varlog", "mountPath": "/var/log", "readOnly": true},
                            {"name": "varlibdockercontainers", "mountPath": "/var/lib/docker/containers", "readOnly": true},
                            {"name": "varlogpods", "mountPath": "/var/log/pods", "readOnly": true},
                        ],
                        "resources": {
                            "limits": self.config.resources.limits,
                            "requests": self.config.resources.requests,
                        }
                    }],
                    "volumes": [
                        {"name": "config", "configMap": {"name": "fluentbit-config"}},
                        {"name": "varlog", "hostPath": {"path": "/var/log"}},
                        {"name": "varlibdockercontainers", "hostPath": {"path": "/var/lib/docker/containers"}},
                        {"name": "varlogpods", "hostPath": {"path": "/var/log/pods"}},
                    ],
                }
            }
        });

        Ok(serde_json::from_value(spec)?)
    }

    /// Generate ConfigMap for Fluentbit configuration
    pub fn generate_configmap(&self) -> Result<DynamicObject, anyhow::Error> {
        let config_content = self.generate_fluentbit_config()?;
        
        let mut configmap = DynamicObject::new("fluentbit-config", "v1", "ConfigMap");
        
        configmap.metadata = ObjectMeta {
            name: Some("fluentbit-config".to_string()),
            namespace: Some(self.config.namespace.clone()),
            labels: Some({
                let mut labels = HashMap::new();
                labels.insert("app".to_string(), "fluentbit".to_string());
                labels
            }),
        };

        let data = serde_json::json!({
            "fluent-bit.conf": config_content,
        });
        
        configmap.data = Some(serde_json::from_value(data)?);
        
        Ok(configmap)
    }

    /// Generate Fluentbit configuration
    fn generate_fluentbit_config(&self) -> Result<String, anyhow::Error> {
        let mut config = String::new();
        
        // [SERVICE] section
        config.push_str(r#"
[SERVICE]
    Flush         5
    Log_Level     info
    Daemon        off
    Parsers_File  parsers.conf
    HTTP_Server   On
    HTTP_Listen   0.0.0.0
    HTTP_Port     2020
    Health_Check  On
"#);
        
        // [INPUT] sections
        for input in &self.config.inputs {
            config.push_str(&format!("\n[INPUT]\n    Name          {}\n", input.input_type_str()));
            
            if let Some(path) = &input.path {
                config.push_str(&format!("    Path          {}\n", path));
            }
            
            config.push_str(&format!("    Tag           {}\n", input.tag));
            
            for (key, value) in &input.options {
                config.push_str(&format!("    {}          {}\n", key, value));
            }
            
            config.push('\n');
        }
        
        // Default inputs if none specified
        if self.config.inputs.is_empty() {
            config.push_str(r#"
[INPUT]
    Name              tail
    Path              /var/log/containers/*.log
    Parser            docker
    Tag               kube.*
    Refresh_Interval  5
    Mem_Buf_Limit     50MB
    Skip_Long_Lines   On

[INPUT]
    Name              systemd
    Tag               host.*
    Systemd_Filter    _SYSTEMD_UNIT=docker.service
    Systemd_Filter    _SYSTEMD_UNIT=kubelet.service
    Read_From_Tail    On
"#);
        }
        
        // [FILTER] sections
        for filter in &self.config.filters {
            config.push_str(&format!("\n[FILTER]\n    Name                {}\n", filter.filter_type_str()));
            config.push_str(&format!("    Match               {}\n", filter.match_pattern));
            
            for rule in &filter.rules {
                config.push_str(&format!("    {}\n", self.filter_rule_to_string(rule)));
            }
            
            config.push('\n');
        }
        
        // Default filters if none specified
        if self.config.filters.is_empty() {
            config.push_str(r#"
[FILTER]
    Name                kubernetes
    Match               kube.*
    Kube_URL            https://kubernetes.default.svc:443
    Kube_CA_File        /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
    Kube_Token_File     /var/run/secrets/kubernetes.io/serviceaccount/token
    Kube_Tag_Prefix     kube.var.log.containers.
    Merge_Log           On
    Merge_Log_Key       log_processed
    Merge_Log_Trim      On
    Labels              On
    Annotations         On

[FILTER]
    Name                nest
    Match               *
    Operation           lift
    Nested_under        kubernetes
    Add_prefix          k8s_

[FILTER]
    Name                modify
    Match               *
    Add                 cluster_name stellar-k8s
    Add                 environment production
"#);
        }
        
        // [OUTPUT] sections
        for output in &self.config.outputs {
            config.push_str(&format!("\n[OUTPUT]\n    Name            {}\n", output.output_type_str()));
            config.push_str(&format!("    Match           *\n"));
            config.push_str(&format!("    Host            {}\n", output.endpoint));
            
            if let Some(auth) = &output.auth {
                if let Some(user) = &auth.username {
                    config.push_str(&format!("    HTTP_User       {}\n", user));
                }
                if let Some(pass) = &auth.password {
                    config.push_str(&format!("    HTTP_Passwd     {}\n", pass));
                }
                if let Some(token) = &auth.token {
                    config.push_str(&format!("    HTTP_Auth       On\n"));
                    config.push_str(&format!("    Header          Authorization Bearer {}\n", token));
                }
            }
            
            if let Some(tls) = &output.tls {
                if tls.enabled {
                    config.push_str("    tls             On\n");
                    if let Some(ca) = &tls.ca_file {
                        config.push_str(&format!("    tls.ca_file     {}\n", ca));
                    }
                    if let Some(cert) = &tls.cert_file {
                        config.push_str(&format!("    tls.cert_file   {}\n", cert));
                    }
                    if let Some(key) = &tls.key_file {
                        config.push_str(&format!("    tls.key_file    {}\n", key));
                    }
                    config.push_str(&format!("    tls.verify      {}\n", if tls.verify { "On" } else { "Off" }));
                }
            }
            
            for (key, value) in &output.options {
                config.push_str(&format!("    {}            {}\n", key, value));
            }
            
            config.push('\n');
        }
        
        // Default output if none specified
        if self.config.outputs.is_empty() {
            config.push_str(r#"
[OUTPUT]
    Name            stdout
    Match           *
    Format          json_lines
    json_date_format iso8601
"#);
        }
        
        // Custom parsers
        config.push_str(r#"

[PARSER]
    Name        docker
    Format      json
    Time_Key    time
    Time_Format %Y-%m-%dT%H:%M:%S.%L
    Time_Keep   On

[PARSER]
    Name        kubernetes
    Format      regex
    Regex       ^(?<time>[^ ]+) (?<stream>stdout|stderr) (?<logtag>[^ ]*) (?<message>.*)$
    Time_Key    time
    Time_Format %Y-%m-%dT%H:%M:%S.%L

[PARSER]
    Name        syslog
    Format      regex
    Regex       ^(?<time>[^ ]* [^ ]* [^ ]*) (?<host>[^ ]*) (?<ident>[a-zA-Z0-9_\/\.\-]*)(?:\[(?<pid>[0-9]+)\])?(?:[^\:]*\:)? *(?<message>.*)$
    Time_Key    time
    Time_Format %b %d %H:%M:%S
"#);
        
        Ok(config)
    }
    
    fn filter_rule_to_string(&self, rule: &FilterRule) -> String {
        match rule.action {
            FilterAction::Keep => format!("    Regex             {} {}\n", rule.key, rule.regex.as_deref().unwrap_or("")),
            FilterAction::Drop => format!("    Exclude             {} {}\n", rule.key, rule.regex.as_deref().unwrap_or("")),
            FilterAction::Replace => format!("    Replace             {} {} {}\n", rule.key, rule.regex.as_deref().unwrap_or(""), rule.value.as_deref().unwrap_or("")),
            FilterAction::Add => format!("    Add                 {} {}\n", rule.key, rule.value.as_deref().unwrap_or("")),
        }
    }
}

impl LogInput {
    fn input_type_str(&self) -> &'static str {
        match self.input_type {
            InputType::Tail => "tail",
            InputType::Systemd => "systemd",
            InputType::Kubernetes => "kubernetes",
            InputType::Tcp => "tcp",
            InputType::Udp => "udp",
            InputType::Http => "http",
        }
    }
}

impl LogFilter {
    fn filter_type_str(&self) -> &'static str {
        match self.filter_type {
            FilterType::Grep => "grep",
            FilterType::RecordModifier => "record_modifier",
            FilterType::Nest => "nest",
            FilterType::Modify => "modify",
            FilterType::StandardOutput => "stdout",
        }
    }
}

impl LogOutput {
    fn output_type_str(&self) -> &'static str {
        match self.output_type {
            OutputType::Elasticsearch => "es",
            OutputType::Loki => "loki",
            OutputType::Kafka => "kafka",
            OutputType::Stdout => "stdout",
            OutputType::File => "file",
            OutputType::Splunk => "splunk",
            OutputType::Datadog => "datadog",
            OutputType::NewRelic => "newrelic",
            OutputType::CloudWatch => "cloudwatch",
            OutputType::AzureLogAnalytics => "azure",
        }
    }
}

impl Default for FluentbitConfig {
    fn default() -> Self {
        Self {
            image: "fluent/fluent-bit:3.0".to_string(),
            namespace: "logging".to_string(),
            service_account: "fluentbit".to_string(),
            outputs: vec![],
            inputs: vec![],
            filters: vec![],
            buffer: BufferConfig {
                memory: Some("50MB".to_string()),
                storage: Some("100MB".to_string()),
                chunk_size: Some("32k".to_string()),
                flush_interval: Some("5s".to_string()),
                flush_timeout: Some("10s".to_string()),
            },
            resources: ResourceRequirements {
                limits: {
                    let mut m = HashMap::new();
                    m.insert("cpu".to_string(), "500m".to_string());
                    m.insert("memory".to_string(), "512Mi".to_string());
                    m
                },
                requests: {
                    let mut m = HashMap::new();
                    m.insert("cpu".to_string(), "100m".to_string());
                    m.insert("memory".to_string(), "128Mi".to_string());
                    m
                },
            },
            tolerations: vec![],
            node_selector: HashMap::new(),
        }
    }
}

/// Log-based alerting engine
pub struct LogAlertEngine {
    rules: Arc<RwLock<Vec<LogAlertRule>>>,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogAlertRule {
    pub id: String,
    pub name: String,
    pub query: String,
    pub condition: AlertCondition,
    pub threshold: f64,
    pub window: String,
    pub severity: Severity,
    pub enabled: bool,
    pub notifications: Vec<NotificationTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertCondition {
    CountGreaterThan,
    CountLessThan,
    RateGreaterThan,
    RateLessThan,
    PatternMatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationTarget {
    pub target_type: NotificationType,
    pub endpoint: String,
    pub template: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationType {
    Webhook,
    Email,
    Slack,
    PagerDuty,
    OpsGenie,
}

impl LogAlertEngine {
    pub fn new(client: Client) -> Arc<Self> {
        Arc::new(Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            client,
        })
    }

    pub async fn add_rule(&self, rule: LogAlertRule) {
        self.rules.write().await.push(rule);
    }

    pub async fn evaluate_rules(&self) -> Vec<AlertEvent> {
        let rules = self.rules.read().await;
        let mut events = Vec::new();
        
        for rule in rules.iter().filter(|r| r.enabled) {
            if let Some(event) = self.evaluate_rule(rule).await {
                events.push(event);
            }
        }
        
        events
    }

    async fn evaluate_rule(&self, rule: &LogAlertRule) -> Option<AlertEvent> {
        // In a real implementation, this would query the log backend (Loki, Elasticsearch, etc.)
        // For now, we return None
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub rule_id: String,
    pub rule_name: String,
    pub severity: Severity,
    pub message: String,
    pub value: f64,
    pub threshold: f64,
    pub timestamp: DateTime<Utc>,
    pub labels: HashMap<String, String>,
}

impl Default for LogInput {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            input_type: InputType::Tail,
            path: None,
            tag: "kube.*".to_string(),
            options: HashMap::new(),
        }
    }
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            filter_type: FilterType::Grep,
            match_pattern: "*".to_string(),
            rules: vec![],
        }
    }
}

impl Default for LogOutput {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            output_type: OutputType::Stdout,
            endpoint: "".to_string(),
            auth: None,
            tls: None,
            options: HashMap::new(),
        }
    }
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            memory: Some("50MB".to_string()),
            storage: Some("100MB".to_string()),
            chunk_size: Some("32k".to_string()),
            flush_interval: Some("5s".to_string()),
            flush_timeout: Some("10s".to_string()),
        }
    }
}

impl Default for ResourceRequirements {
    fn default() -> Self {
        Self {
            limits: {
                let mut m = HashMap::new();
                m.insert("cpu".to_string(), "500m".to_string());
                m.insert("memory".to_string(), "512Mi".to_string());
                m
            },
            requests: {
                let mut m = HashMap::new();
                m.insert("cpu".to_string(), "100m".to_string());
                m.insert("memory".to_string(), "128Mi".to_string());
                m
            },
        }
    }
}

impl Default for Toleration {
    fn default() -> Self {
        Self {
            key: None,
            operator: "Exists".to_string(),
            value: None,
            effect: "NoSchedule".to_string(),
            toleration_seconds: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_type_str() {
        assert_eq!(OutputType::Elasticsearch.output_type_str(), "es");
        assert_eq!(OutputType::Loki.output_type_str(), "loki");
        assert_eq!(OutputType::Stdout.output_type_str(), "stdout");
    }

    #[test]
    fn test_alert_condition() {
        assert_eq!(AlertCondition::CountGreaterThan as i32, 0);
        assert_eq!(AlertCondition::RateGreaterThan as i32, 2);
    }

    #[test]
    fn test_fluentbit_config_default() {
        let config = FluentbitConfig::default();
        assert_eq!(config.image, "fluent/fluent-bit:3.0");
        assert_eq!(config.namespace, "logging");
    }
}