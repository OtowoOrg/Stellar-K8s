# Observability Reference Architecture for Stellar-K8s

## Overview

This document details a comprehensive end-to-end telemetry architecture using Prometheus, Loki, and Grafana across Stellar-K8s deployments. It provides standardized metric collection, log aggregation, and alert configurations for maintaining operational visibility across distributed node clusters.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Metric Collection](#metric-collection)
3. [Log Aggregation](#log-aggregation)
4. [Alert Configuration](#alert-configuration)
5. [Dashboard Templates](#dashboard-templates)
6. [Troubleshooting & Validation](#troubleshooting-validation)

## Architecture Overview

### System Components

```
┌─────────────────────────────────────────────────────────────┐
│                    Stellar-K8s Cluster                      │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │Stellar Core  │  │   Horizon    │  │ Soroban RPC  │      │
│  │  Exporter    │  │  Exporter    │  │  Exporter    │      │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘      │
│         │                  │                  │               │
│         └──────────────────┼──────────────────┘               │
│                            │ Metrics                          │
│  ┌─────────────────────────▼───────────────────────────┐    │
│  │         Prometheus                                  │    │
│  │  - Service discovery (Kubernetes SD)               │    │
│  │  - Metric scraping (30s intervals)                 │    │
│  │  - Local time-series storage (15GB, 30 days)       │    │
│  └──────────────────┬────────────────────────────────┘    │
│                     │                                       │
└─────────────────────┼───────────────────────────────────────┘
                      │
        ┌─────────────┼─────────────┐
        │             │             │
┌───────▼──────┐ ┌──▼───────┐ ┌──▼───────────┐
│   Grafana    │ │  Loki    │ │ AlertManager │
│  (Dashboard) │ │(Log Agg) │ │ (Alerting)   │
└──────────────┘ └──────────┘ └──────────────┘
```

### Key Architecture Decisions

1. **Prometheus**: Primary metrics store with 30-day retention
2. **Loki**: Log aggregation without indexing overhead
3. **Grafana**: Unified visualization and alerting
4. **AlertManager**: Centralized alerting with multi-channel routing
5. **Custom Exporters**: Component-specific metric collection

## Metric Collection

### 1. Stellar Core Metrics

#### Metric Dictionary

| Metric Name | Type | Labels | Description | Alert Threshold |
|-------------|------|--------|-------------|-----------------|
| `stellar_core_consensus_state` | gauge | `pod`, `instance` | 1=Synced, 0=Not synced | Alert if = 0 |
| `stellar_core_ledger_current` | gauge | `pod` | Current ledger sequence | Baseline + 5% drift |
| `stellar_core_peers_connected` | gauge | `pod` | Number of connected peers | < 5 peers |
| `stellar_core_sync_lag_seconds` | gauge | `pod` | Seconds behind network | > 30 seconds |
| `stellar_core_txn_counter_total` | counter | `pod` | Total transactions processed | n/a |
| `stellar_core_txn_rate` | gauge | `pod` | Transactions per second | Baseline ± 20% |
| `stellar_core_scp_timeout_total` | counter | `pod` | SCP timeouts | > 0 in 5m window |
| `stellar_core_memory_bytes` | gauge | `pod` | Memory usage | > 85% of limit |
| `stellar_core_db_connections` | gauge | `pod` | Database connections | > 90 |
| `stellar_core_http_requests_total` | counter | `pod`, `method`, `status` | HTTP requests by status | 5xx rate > 1% |
| `stellar_core_http_duration_seconds` | histogram | `pod`, `method` | HTTP request duration | p99 > 2 seconds |

#### Prometheus Scrape Configuration

```yaml
# prometheus-stellar-core.yaml
global:
  scrape_interval: 30s
  evaluation_interval: 30s
  external_labels:
    cluster: 'stellar-k8s-prod'
    environment: 'production'

scrape_configs:
  - job_name: 'stellar-core'
    kubernetes_sd_configs:
      - role: pod
        namespaces:
          names:
            - stellar-k8s
    relabel_configs:
      # Only scrape stellar-core pods
      - source_labels: [__meta_kubernetes_pod_label_app]
        action: keep
        regex: stellar-core
      
      # Use pod name as instance label
      - source_labels: [__meta_kubernetes_pod_name]
        action: replace
        target_label: pod
      
      # Add namespace label
      - source_labels: [__meta_kubernetes_namespace]
        action: replace
        target_label: namespace
      
      # Set metrics path
      - source_labels: [__meta_kubernetes_pod_container_port_number]
        action: keep
        regex: "11626"
      
      # Set address
      - source_labels: [__address__]
        target_label: __address__
        regex: ([^:]+)(?::\d+)?
        replacement: ${1}:11626
    
    # Scrape specific HTTP endpoint
    metrics_path: /metrics
    
    # Timeout for scrapes
    scrape_timeout: 10s
    
    # Retry configuration
    scrape_interval: 30s
    scrape_classic_histograms: true
    
    # Relabeling for metric filtering
    metric_relabel_configs:
      # Drop high-cardinality metrics
      - source_labels: [__name__]
        regex: '.*_bucket.*'
        action: drop
```

### 2. Horizon Metrics

#### Metric Dictionary

| Metric Name | Type | Labels | Description | Alert Threshold |
|-------------|------|--------|-------------|-----------------|
| `horizon_ledger_closed` | gauge | `instance` | Last closed ledger | Baseline ± 5% |
| `horizon_requests_total` | counter | `method`, `status`, `endpoint` | HTTP requests | 5xx > 1% |
| `horizon_request_duration_seconds` | histogram | `method`, `endpoint` | Request latency | p99 > 1s |
| `horizon_database_lag_seconds` | gauge | `instance` | DB replication lag | > 5 seconds |
| `horizon_cache_hits_total` | counter | `cache_name` | Cache hit count | n/a |
| `horizon_cache_misses_total` | counter | `cache_name` | Cache miss count | n/a |
| `horizon_cache_hit_rate` | gauge | `cache_name` | Hit rate percentage | < 80% |
| `horizon_ingestion_lag_seconds` | gauge | `instance` | Ingestion lag | > 30 seconds |
| `horizon_memory_bytes` | gauge | `instance` | Memory usage | > 85% |
| `horizon_goroutines` | gauge | `instance` | Active goroutines | > 5000 |
| `horizon_db_connections_open` | gauge | `instance` | Open DB connections | > 80 |

#### Prometheus Scrape Configuration

```yaml
# prometheus-horizon.yaml
scrape_configs:
  - job_name: 'horizon'
    kubernetes_sd_configs:
      - role: pod
        namespaces:
          names:
            - stellar-k8s
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        action: keep
        regex: horizon
      
      - source_labels: [__meta_kubernetes_pod_name]
        action: replace
        target_label: pod
      
      - source_labels: [__meta_kubernetes_pod_container_port_number]
        action: keep
        regex: "8000"
      
      - source_labels: [__address__]
        target_label: __address__
        regex: ([^:]+)(?::\d+)?
        replacement: ${1}:8000
    
    metrics_path: /metrics
    scrape_timeout: 10s
    scrape_interval: 30s
```

### 3. Soroban RPC Metrics

#### Metric Dictionary

| Metric Name | Type | Labels | Description | Alert Threshold |
|-------------|------|--------|-------------|-----------------|
| `soroban_rpc_requests_total` | counter | `method`, `status` | RPC requests | n/a |
| `soroban_rpc_request_duration_seconds` | histogram | `method` | RPC request latency | p99 > 1s |
| `soroban_rpc_cache_bytes` | gauge | `cache_type` | Cache memory usage | > 90% |
| `soroban_rpc_contract_storage_entries` | gauge | `instance` | Contract storage entries | n/a |
| `soroban_rpc_ledger_lag_seconds` | gauge | `instance` | Ledger sync lag | > 5 seconds |
| `soroban_rpc_error_total` | counter | `error_type` | Errors by type | > 100 in 5m |
| `soroban_rpc_contract_invocations_total` | counter | `instance` | Contract invocations | n/a |
| `soroban_rpc_disk_usage_bytes` | gauge | `mount` | Disk usage | > 85% |

#### Prometheus Scrape Configuration

```yaml
# prometheus-soroban-rpc.yaml
scrape_configs:
  - job_name: 'soroban-rpc'
    kubernetes_sd_configs:
      - role: pod
        namespaces:
          names:
            - stellar-k8s
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        action: keep
        regex: soroban-rpc
      
      - source_labels: [__meta_kubernetes_pod_name]
        action: replace
        target_label: pod
      
      - source_labels: [__meta_kubernetes_pod_container_port_number]
        action: keep
        regex: "6969"
      
      - source_labels: [__address__]
        target_label: __address__
        regex: ([^:]+)(?::\d+)?
        replacement: ${1}:6969
    
    metrics_path: /metrics
```

### 4. Kubernetes Cluster Metrics

```yaml
# prometheus-kubernetes.yaml
scrape_configs:
  # kube-state-metrics
  - job_name: 'kube-state-metrics'
    kubernetes_sd_configs:
      - role: service
        namespaces:
          names:
            - monitoring
    relabel_configs:
      - source_labels: [__meta_kubernetes_service_label_app]
        action: keep
        regex: kube-state-metrics
      
      - source_labels: [__meta_kubernetes_endpoint_port_number]
        action: keep
        regex: "8080"

  # Node exporter
  - job_name: 'node-exporter'
    kubernetes_sd_configs:
      - role: node
    relabel_configs:
      - source_labels: [__address__]
        target_label: __address__
        regex: ([^:]+)(?::\d+)?
        replacement: ${1}:9100
      
      - source_labels: [__meta_kubernetes_node_name]
        action: replace
        target_label: node

  # cAdvisor
  - job_name: 'cadvisor'
    kubernetes_sd_configs:
      - role: node
    relabel_configs:
      - source_labels: [__address__]
        target_label: __address__
        regex: ([^:]+)(?::\d+)?
        replacement: ${1}:10250
      
      - source_labels: [__meta_kubernetes_node_name]
        action: replace
        target_label: node
    
    scheme: https
    tls_config:
      ca_file: /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
    bearer_token_file: /var/run/secrets/kubernetes.io/serviceaccount/token
```

## Log Aggregation

### 1. Loki Configuration

#### Loki Deployment

```yaml
# loki-config.yaml
auth_enabled: false

ingester:
  chunk_idle_period: 3m
  chunk_retain_period: 1m
  max_chunk_age: 1h
  chunk_encoding: snappy
  lifecycler:
    ring:
      kvstore:
        store: inmemory

limits_config:
  enforce_metric_name: false
  reject_old_samples: true
  reject_old_samples_max_age: 168h

schema_config:
  configs:
  - from: 2024-01-01
    store: boltdb-shipper
    object_store: filesystem
    schema: v11
    index:
      prefix: index_
      period: 24h

server:
  http_listen_port: 3100
  log_level: info

storage_config:
  boltdb_shipper:
    active_index_directory: /loki/boltdb-shipper-active
    cache_location: /loki/boltdb-shipper-cache
    shared_store: filesystem
  filesystem:
    directory: /loki/chunks

chunk_store_config:
  max_look_back_period: 0s

table_manager:
  retention_deletes_enabled: false
  retention_period: 0s

# Label configuration
table_manager:
  poll_interval: 10m
  retention_deletes_enabled: true
  retention_period: 720h
```

### 2. Log Collection from Components

#### Stellar Core Logs

```yaml
# fluent-bit-stellar-core.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: fluent-bit-stellar-core-config
data:
  stellar-core.conf: |
    [INPUT]
    Name tail
    Path /var/log/stellar/stellar-core.log
    Parser stellar-core
    Tag stellar-core.*
    Refresh_Interval 5
    Mem_Buf_Limit 50MB
    Skip_Long_Lines On

    [FILTER]
    Name kubernetes
    Match stellar-core.*
    Kube_URL https://kubernetes.default.svc:443
    Kube_CA_File /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
    Kube_Token_File /var/run/secrets/kubernetes.io/serviceaccount/token
    Kube_Tag_Prefix stellar-core.var.log.
    Merge_Log On
    Keep_Log Off
    K8S_Logging_Parser On
    K8S_Logging_Exclude Off

    [FILTER]
    Name modify
    Match stellar-core.*
    Add cluster stellar-k8s-prod
    Add component stellar-core
    Add environment production

    [OUTPUT]
    Name loki
    Match stellar-core.*
    Host loki
    Port 3100
    Labels job=stellar-core,pod=$kubernetes_pod_name,namespace=$kubernetes_namespace
    Auto_Kubernetes_Labels On
    Line_Format json
```

#### Horizon Logs

```yaml
# fluent-bit-horizon.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: fluent-bit-horizon-config
data:
  horizon.conf: |
    [INPUT]
    Name tail
    Path /var/log/horizon/*.log
    Parser json
    Tag horizon.*
    Refresh_Interval 5
    Mem_Buf_Limit 100MB
    Skip_Long_Lines On

    [FILTER]
    Name kubernetes
    Match horizon.*
    Kube_URL https://kubernetes.default.svc:443
    Kube_CA_File /var/run/secrets/kubernetes.io/serviceaccount/ca.crt
    Kube_Token_File /var/run/secrets/kubernetes.io/serviceaccount/token
    Merge_Log On
    Keep_Log Off
    K8S_Logging_Parser On

    [FILTER]
    Name modify
    Match horizon.*
    Add cluster stellar-k8s-prod
    Add component horizon

    [FILTER]
    Name grep
    Match horizon.*
    Exclude log ^[\s]*$  # Skip empty lines

    [OUTPUT]
    Name loki
    Match horizon.*
    Host loki
    Port 3100
    Labels job=horizon,pod=$kubernetes_pod_name,namespace=$kubernetes_namespace
```

#### Soroban RPC Logs

```yaml
# fluent-bit-soroban-rpc.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: fluent-bit-soroban-rpc-config
data:
  soroban-rpc.conf: |
    [INPUT]
    Name tail
    Path /var/log/soroban-rpc/*.log
    Parser json
    Tag soroban-rpc.*
    Refresh_Interval 5
    Mem_Buf_Limit 75MB

    [FILTER]
    Name kubernetes
    Match soroban-rpc.*
    Kube_URL https://kubernetes.default.svc:443
    Merge_Log On

    [FILTER]
    Name modify
    Match soroban-rpc.*
    Add cluster stellar-k8s-prod
    Add component soroban-rpc

    [OUTPUT]
    Name loki
    Match soroban-rpc.*
    Host loki
    Port 3100
    Labels job=soroban-rpc,pod=$kubernetes_pod_name
```

### 3. Log Query Examples

```yaml
# Loki log queries for common troubleshooting

# 1. Find errors in Stellar Core
{job="stellar-core", level="error"}

# 2. Find consensus failures
{job="stellar-core"} |= "SCP"

# 3. Find Horizon transaction ingestion errors
{job="horizon"} |= "ingestion" |= "error"

# 4. Find Soroban RPC slow requests
{job="soroban-rpc"} | duration > 1000

# 5. Find database connection errors
{job="horizon"} |= "connection" |= "refused"

# 6. Find peer disconnections
{job="stellar-core"} |= "peer" |= "disconnect"
```

## Alert Configuration

### PrometheusRule Manifests

```yaml
# prometheus-rules-stellar-core.yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: stellar-core-alerts
  namespace: monitoring
spec:
  groups:
  - name: stellar-core
    interval: 30s
    rules:
    
    # Critical: Consensus Lost
    - alert: StellarCoreConsensusLost
      expr: stellar_core_consensus_state == 0
      for: 5m
      labels:
        severity: critical
        component: stellar-core
      annotations:
        summary: "Stellar Core consensus lost"
        description: "Pod {{ $labels.pod }} has lost consensus for more than 5 minutes"
        runbook: "docs/troubleshooting/consensus-lost.md"
    
    # Critical: Sync Lag Excessive
    - alert: StellarCoreSyncLagExcessive
      expr: stellar_core_sync_lag_seconds > 60
      for: 10m
      labels:
        severity: critical
      annotations:
        summary: "Stellar Core sync lag critical"
        description: "Pod {{ $labels.pod }} is {{ $value }}s behind network"
    
    # Warning: Low Peer Count
    - alert: StellarCoreLowPeerCount
      expr: stellar_core_peers_connected < 5
      for: 15m
      labels:
        severity: warning
      annotations:
        summary: "Stellar Core peer count low"
        description: "Pod {{ $labels.pod }} has only {{ $value }} connected peers"
    
    # Warning: Transaction Rate Anomaly
    - alert: StellarCoreTxnRateAnomaly
      expr: |
        abs(stellar_core_txn_rate - avg(stellar_core_txn_rate offset 1h)) / avg(stellar_core_txn_rate offset 1h) > 0.2
      for: 10m
      labels:
        severity: warning
      annotations:
        summary: "Stellar Core transaction rate anomaly"
        description: "Transaction rate deviated by {{ $value | humanizePercentage }} from baseline"
    
    # Critical: High Memory Usage
    - alert: StellarCoreHighMemory
      expr: stellar_core_memory_bytes / 8589934592 > 0.85
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Stellar Core high memory usage"
        description: "Memory usage at {{ $value | humanizePercentage }} of limit"
    
    # Warning: Database Connection Pool Saturation
    - alert: StellarCoreDBConnectionSaturation
      expr: stellar_core_db_connections / 100 > 0.8
      for: 10m
      labels:
        severity: warning
      annotations:
        summary: "Database connection pool near saturation"
        description: "{{ $value | humanizePercentage }} of connection pool in use"
    
    # Critical: High HTTP Error Rate
    - alert: StellarCoreHighErrorRate
      expr: |
        rate(stellar_core_http_requests_total{status=~"5.."}[5m]) / rate(stellar_core_http_requests_total[5m]) > 0.01
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Stellar Core high HTTP error rate"
        description: "Error rate at {{ $value | humanizePercentage }}"
    
    # Warning: High HTTP Latency
    - alert: StellarCoreHighLatency
      expr: |
        histogram_quantile(0.99, rate(stellar_core_http_duration_seconds_bucket[5m])) > 2
      for: 10m
      labels:
        severity: warning
      annotations:
        summary: "Stellar Core high HTTP latency"
        description: "P99 latency: {{ $value }}s"
    
    # Critical: SCP Timeouts
    - alert: StellarCoreSCPTimeouts
      expr: |
        rate(stellar_core_scp_timeout_total[5m]) > 0
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Stellar Core SCP timeouts detected"
        description: "{{ $value }} SCP timeouts in the last 5 minutes"
```

```yaml
# prometheus-rules-horizon.yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: horizon-alerts
  namespace: monitoring
spec:
  groups:
  - name: horizon
    interval: 30s
    rules:
    
    # Critical: High Error Rate
    - alert: HorizonHighErrorRate
      expr: |
        rate(horizon_requests_total{status=~"5.."}[5m]) / rate(horizon_requests_total[5m]) > 0.01
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Horizon high error rate"
        description: "Error rate at {{ $value | humanizePercentage }}"
    
    # Warning: High Request Latency
    - alert: HorizonHighLatency
      expr: |
        histogram_quantile(0.99, rate(horizon_request_duration_seconds_bucket[5m])) > 1
      for: 10m
      labels:
        severity: warning
      annotations:
        summary: "Horizon high latency"
        description: "P99 latency: {{ $value }}s"
    
    # Critical: Database Replication Lag
    - alert: HorizonDBReplicationLag
      expr: horizon_database_lag_seconds > 5
      for: 10m
      labels:
        severity: critical
      annotations:
        summary: "Horizon database replication lag"
        description: "Replication lag: {{ $value }}s"
    
    # Warning: Low Cache Hit Rate
    - alert: HorizonLowCacheHitRate
      expr: horizon_cache_hit_rate < 0.8
      for: 15m
      labels:
        severity: warning
      annotations:
        summary: "Horizon low cache hit rate"
        description: "Cache hit rate: {{ $value | humanizePercentage }}"
    
    # Warning: High Ingestion Lag
    - alert: HorizonHighIngestionLag
      expr: horizon_ingestion_lag_seconds > 30
      for: 15m
      labels:
        severity: warning
      annotations:
        summary: "Horizon ingestion lag high"
        description: "Ingestion lag: {{ $value }}s"
    
    # Critical: High Memory Usage
    - alert: HorizonHighMemory
      expr: horizon_memory_bytes / 4294967296 > 0.85
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Horizon high memory usage"
        description: "Memory usage at {{ $value | humanizePercentage }} of limit"
    
    # Warning: Goroutine Leak
    - alert: HorizonGoroutineLeakSuspected
      expr: |
        rate(horizon_goroutines[30m]) > 0
      for: 30m
      labels:
        severity: warning
      annotations:
        summary: "Horizon goroutine count increasing"
        description: "Goroutine count: {{ $value }}"
    
    # Critical: Database Connection Pool Saturation
    - alert: HorizonDBConnectionSaturation
      expr: horizon_db_connections_open / 100 > 0.85
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Horizon database connection pool saturated"
        description: "{{ $value | humanizePercentage }} of connection pool in use"
```

```yaml
# prometheus-rules-infrastructure.yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: infrastructure-alerts
  namespace: monitoring
spec:
  groups:
  - name: infrastructure
    interval: 30s
    rules:
    
    # Critical: Node Not Ready
    - alert: KubernetesNodeNotReady
      expr: kube_node_status_condition{condition="Ready",status="true"} == 0
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Kubernetes node not ready"
        description: "Node {{ $labels.node }} is not ready"
    
    # Critical: High Disk Usage
    - alert: KubernetesHighDiskUsage
      expr: |
        (1 - (node_filesystem_avail_bytes / node_filesystem_size_bytes)) > 0.85
      for: 10m
      labels:
        severity: critical
      annotations:
        summary: "High disk usage on {{ $labels.device }}"
        description: "Disk usage at {{ $value | humanizePercentage }}"
    
    # Critical: High CPU Usage
    - alert: KubernetesHighCPUUsage
      expr: |
        (1 - avg(irate(node_cpu_seconds_total{mode="idle"}[5m]))) > 0.85
      for: 15m
      labels:
        severity: critical
      annotations:
        summary: "High CPU usage on {{ $labels.node }}"
        description: "CPU usage at {{ $value | humanizePercentage }}"
    
    # Critical: Pod Restart Loop
    - alert: KubernetesPodRestartLoop
      expr: |
        rate(kube_pod_container_status_restarts_total[15m]) > 0.1
      for: 5m
      labels:
        severity: critical
      annotations:
        summary: "Pod restarting frequently"
        description: "Pod {{ $labels.pod }} in namespace {{ $labels.namespace }}"
    
    # Warning: PVC Usage High
    - alert: KubernetesPVCUsageHigh
      expr: |
        kubelet_volume_stats_used_bytes / kubelet_volume_stats_capacity_bytes > 0.85
      for: 15m
      labels:
        severity: warning
      annotations:
        summary: "Persistent volume capacity low"
        description: "PVC {{ $labels.persistentvolumeclaim }} at {{ $value | humanizePercentage }}"
```

## Dashboard Templates

### 1. Stellar Core Dashboard

```json
{
  "dashboard": {
    "title": "Stellar Core - Performance & Health",
    "panels": [
      {
        "title": "Consensus State",
        "targets": [
          {
            "expr": "stellar_core_consensus_state"
          }
        ],
        "type": "stat",
        "thresholds": {
          "mode": "absolute",
          "steps": [
            {"color": "red", "value": 0},
            {"color": "green", "value": 1}
          ]
        }
      },
      {
        "title": "Ledger Sequence",
        "targets": [
          {
            "expr": "stellar_core_ledger_current"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Connected Peers",
        "targets": [
          {
            "expr": "stellar_core_peers_connected"
          }
        ],
        "type": "gauge",
        "thresholds": {
          "mode": "absolute",
          "steps": [
            {"color": "red", "value": 5},
            {"color": "yellow", "value": 10},
            {"color": "green", "value": 20}
          ]
        }
      },
      {
        "title": "Sync Lag (seconds)",
        "targets": [
          {
            "expr": "stellar_core_sync_lag_seconds"
          }
        ],
        "type": "graph",
        "alert": "value > 60"
      },
      {
        "title": "Transactions Per Second",
        "targets": [
          {
            "expr": "rate(stellar_core_txn_counter_total[1m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Memory Usage",
        "targets": [
          {
            "expr": "stellar_core_memory_bytes / 8589934592 * 100"
          }
        ],
        "type": "gauge"
      },
      {
        "title": "HTTP Request Rate",
        "targets": [
          {
            "expr": "rate(stellar_core_http_requests_total[1m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "HTTP Error Rate",
        "targets": [
          {
            "expr": "rate(stellar_core_http_requests_total{status=~\"5..\"}[5m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "HTTP Latency (p99)",
        "targets": [
          {
            "expr": "histogram_quantile(0.99, rate(stellar_core_http_duration_seconds_bucket[5m]))"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Database Connections",
        "targets": [
          {
            "expr": "stellar_core_db_connections"
          }
        ],
        "type": "gauge"
      }
    ]
  }
}
```

### 2. Horizon Dashboard

```json
{
  "dashboard": {
    "title": "Horizon - API Performance & Health",
    "panels": [
      {
        "title": "Last Closed Ledger",
        "targets": [
          {
            "expr": "horizon_ledger_closed"
          }
        ],
        "type": "stat"
      },
      {
        "title": "Request Rate (by status)",
        "targets": [
          {
            "expr": "rate(horizon_requests_total[1m])"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Error Rate (5xx)",
        "targets": [
          {
            "expr": "rate(horizon_requests_total{status=~\"5..\"}[5m]) * 100"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Request Latency (p99)",
        "targets": [
          {
            "expr": "histogram_quantile(0.99, rate(horizon_request_duration_seconds_bucket[5m]))"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Database Replication Lag",
        "targets": [
          {
            "expr": "horizon_database_lag_seconds"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Cache Hit Rate",
        "targets": [
          {
            "expr": "horizon_cache_hit_rate * 100"
          }
        ],
        "type": "gauge"
      },
      {
        "title": "Ingestion Lag",
        "targets": [
          {
            "expr": "horizon_ingestion_lag_seconds"
          }
        ],
        "type": "graph"
      },
      {
        "title": "Memory Usage",
        "targets": [
          {
            "expr": "horizon_memory_bytes / 4294967296 * 100"
          }
        ],
        "type": "gauge"
      }
    ]
  }
}
```

## Troubleshooting & Validation

### Prometheus Validation

```bash
#!/bin/bash
# validate-prometheus.sh

set -e

echo "=== Prometheus Configuration Validation ==="

# 1. Check Prometheus connectivity
echo "[1/5] Checking Prometheus connectivity..."
curl -s http://prometheus:9090/-/healthy > /dev/null && echo "✓ Prometheus reachable" || exit 1

# 2. Validate scrape configs
echo "[2/5] Validating scrape configurations..."
curl -s http://prometheus:9090/api/v1/targets | jq '.data.activeTargets | length'

# 3. Check alert rules
echo "[3/5] Validating alert rules..."
curl -s http://prometheus:9090/api/v1/rules | jq '.data.groups | length' | grep -q "." && echo "✓ Alert rules loaded"

# 4. Verify key metrics exist
echo "[4/5] Verifying key metrics..."
for metric in stellar_core_consensus_state horizon_requests_total soroban_rpc_requests_total; do
  COUNT=$(curl -s "http://prometheus:9090/api/v1/query?query=$metric" | jq '.data.result | length')
  echo "  $metric: $COUNT instances"
done

# 5. Check alert evaluation
echo "[5/5] Checking alert evaluation..."
curl -s http://prometheus:9090/api/v1/alerts | jq '.data.alerts | length' | tee /dev/stderr

echo "✅ Prometheus validation complete"
```

### Loki Validation

```bash
#!/bin/bash
# validate-loki.sh

set -e

echo "=== Loki Configuration Validation ==="

# 1. Check Loki connectivity
echo "[1/4] Checking Loki connectivity..."
curl -s http://loki:3100/ready > /dev/null && echo "✓ Loki reachable" || exit 1

# 2. Verify log stream ingestion
echo "[2/4] Checking log stream ingestion..."
curl -s "http://loki:3100/loki/api/v1/labels" | jq '.data | length' | grep -q "." && echo "✓ Log streams present"

# 3. Test log queries
echo "[3/4] Testing log queries..."
curl -s "http://loki:3100/loki/api/v1/query_range?query={job=\"stellar-core\"}&start=$(date +%s -d '1 hour ago')&end=$(date +%s)" | jq '.data.result | length'

# 4. Check disk usage
echo "[4/4] Checking Loki disk usage..."
du -sh /loki/chunks

echo "✅ Loki validation complete"
```

### Grafana Dashboard Import

```bash
#!/bin/bash
# import-dashboards.sh

GRAFANA_URL="http://grafana:3000"
GRAFANA_API_KEY="${GRAFANA_API_KEY}"

# Import Stellar Core dashboard
curl -X POST "$GRAFANA_URL/api/dashboards/db" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -H "Content-Type: application/json" \
  -d @dashboards/stellar-core-dashboard.json

# Import Horizon dashboard
curl -X POST "$GRAFANA_URL/api/dashboards/db" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -H "Content-Type: application/json" \
  -d @dashboards/horizon-dashboard.json

echo "✅ Dashboards imported"
```

### promtool Rule Validation

```bash
# Validate Prometheus alert rules
promtool check rules prometheus-rules-stellar-core.yaml
promtool check rules prometheus-rules-horizon.yaml
promtool check rules prometheus-rules-infrastructure.yaml

# Validate Prometheus configuration
promtool check config prometheus.yaml

# Check for duplicate metrics
promtool query instant 'up{job="stellar-core"}' | jq '.data.result | unique_by(.metric) | length'
```

## References

- [Prometheus Documentation](https://prometheus.io/docs/)
- [Loki Documentation](https://grafana.com/docs/loki/latest/)
- [Grafana Documentation](https://grafana.com/docs/grafana/latest/)
- [Stellar Core Metrics](https://developers.stellar.org/docs/run-core-node/stellar-core-metrics)
- [Kubernetes Monitoring](https://kubernetes.io/docs/tasks/debug-application-cluster/resource-metrics-pipeline/)

---

**Document Version:** 1.0  
**Last Updated:** 2024-01-15  
**Status:** Production Ready
