# Log Aggregation Guide

This guide describes recommended setups for log aggregation using Loki + Promtail, Elasticsearch + Filebeat, and Fluentd. It includes example parsing rules, dashboard templates, troubleshooting, performance tuning, and cost-optimization tips.

## Prerequisites

- Kubernetes cluster (examples use `kubectl`).
- `helm` for installing charts (optional but recommended).
- Basic Grafana/Kibana access for dashboards.

---

## 1. Loki + Promtail

Overview: Loki stores logs efficiently by indexing labels (not full text). Promtail runs as a DaemonSet to scrape pod logs and push to Loki.

Helm (quick install):

```bash
helm repo add grafana https://grafana.github.io/helm-charts
helm repo update
helm install loki grafana/loki-stack --set promtail.enabled=true
```

Example `promtail` scrape config (values or ConfigMap):

```yaml
server:
  http_listen_port: 9080

clients:
  - url: http://loki:3100/loki/api/v1/push

positions:
  filename: /tmp/positions.yaml

scrape_configs:
  - job_name: kubernetes-pods
    kubernetes_sd_configs:
      - role: pod
    pipeline_stages:
      - cri: {}
      - labelallow:
          - app
          - namespace
    relabel_configs:
      - source_labels: [__meta_kubernetes_pod_label_app]
        target_label: app
      - source_labels: [__meta_kubernetes_namespace]
        target_label: namespace
```

Parsing example using `pipeline_stages` (Promtail):

```yaml
pipeline_stages:
  - regex:
      expression: "^(?P<ts>\\S+) (?P<level>INFO|WARN|ERROR) (?P<msg>.*)$"
  - timestamp:
      source: ts
      format: RFC3339
  - labels:
      level:
```

Grafana dashboard: use `Loki` data source and build panels using `{job="kubernetes-pods"}` log queries. Export/Import JSON from Grafana when ready.

---

## 2. Elasticsearch + Filebeat

Overview: Filebeat ships logs to Elasticsearch (or Logstash). Use the official Filebeat Helm chart or manifest with Kubernetes autodiscover.

Helm quick install (Elasticsearch + Kibana via Elastic Helm charts):

```bash
helm repo add elastic https://helm.elastic.co
helm repo update
helm install elasticsearch elastic/elasticsearch
helm install kibana elastic/kibana
```

Filebeat example config (`values.yaml` or ConfigMap):

```yaml
filebeat.autodiscover:
  providers:
    - type: kubernetes
      hints.enabled: true

filebeat.inputs:
  - type: container
    paths:
      - /var/log/containers/*.log

processors:
  - add_kubernetes_metadata: {}
  - decode_json_fields:
      fields: ["message"]
      target: "json"

output.elasticsearch:
  hosts: ["http://elasticsearch:9200"]
  index: "filebeat-%{+yyyy.MM.dd}"
```

Example Filebeat Kubernetes hint annotation on a Pod:

```yaml
metadata:
  annotations:
    co.elastic.logs/enabled: "true"
    co.elastic.logs/module: "nginx"
```

Parsing rules: Filebeat supports ingest pipelines in Elasticsearch. Example pipeline for Nginx:

```json
{
  "description": "nginx pipeline",
  "processors": [
    {"grok": {"field": "message", "patterns": ["%{COMBINEDAPACHELOG}"]}},
    {"date": {"field": "timestamp", "formats": ["dd/MMM/yyyy:HH:mm:ss Z"]}}
  ]
}
```

Kibana dashboards: Use Sample dashboards from Filebeat `/package` assets or import saved objects.

---

## 3. Fluentd

Overview: Fluentd is a flexible log router with many plugins (e.g., elasticsearch, forward). It can parse, buffer, and transform logs.

Example Fluentd `ConfigMap` snippet for Kubernetes (outputs to Elasticsearch):

```xml
<source>
  @type tail
  path /var/log/containers/*.log
  pos_file /var/log/fluentd-containers.log.pos
  tag kubernetes.*
  format cri
  read_from_head true
</source>

<filter kubernetes.**>
  @type kubernetes_metadata
</filter>

<match **>
  @type elasticsearch
  host elasticsearch
  port 9200
  logstash_format true
  include_tag_key true
  type_name _doc
</match>
```

Fluentd parsing example using `grok` and `regexp`:

```xml
<filter kubernetes.**>
  @type parser
  key_name message
  reserve_data true
  <parse>
    @type regexp
    expression /(?<time>\d{4}-\d{2}-\d{2}T\S+) (?<level>\w+) (?<msg>.*)/
  </parse>
</filter>
```

---

## 4. Example log parsing rules (summary)

- Promtail: `pipeline_stages` with `regex`, `timestamp`, `labels`.
- Filebeat: `processors` + Elasticsearch ingest pipelines (`grok`, `date`).
- Fluentd: `parser` and `grok`/`regexp` filters and plugins.

Provide these example patterns for common apps:

- JSON logs: decode JSON into fields (Promtail `json` stage, Filebeat `decode_json_fields`, Fluentd `parser json`).
- Nginx: `%{COMBINEDAPACHELOG}` grok pattern (Filebeat/ES ingest or Fluentd).
- Java stack traces: use multiline rules in Filebeat or Promtail to join lines by `/^\t|^\\s+at /` patterns.

---

## 5. Dashboard templates

- Loki/Grafana: create panels using `logs` and `log labels`. Example panel queries:
  - Error rate: `count_over_time({namespace="default", level="ERROR"}[5m])`
  - Logs table: `{app="myapp"}`

- Elasticsearch/Kibana: build visualizations on `@timestamp`, `kubernetes.namespace`, `log.level`, and `message.keyword`.

Include exported JSONs in `docs/log-dashboards/` when ready. For now, create dashboards in Grafana/Kibana and export to repository.

---

## 6. Troubleshooting

- Promtail/Loki: check Promtail logs (`kubectl logs ds/promtail -n monitoring`), check Loki ingester and query-frontend pods.
- Filebeat/ES: check Filebeat pods, `kubectl logs ds/filebeat`, and Elasticsearch cluster health (`curl -sS http://elasticsearch:9200/_cluster/health?pretty`).
- Fluentd: check Fluentd logs for parser errors and buffer overflows.
- Common issues: wrong endpoints, RBAC blocking access to `/var/log/containers`, permission to read files, misconfigured parsers causing dropped events.

Useful commands:

```bash
kubectl -n monitoring get pods
kubectl -n monitoring logs deploy/loki
curl -sS http://elasticsearch:9200/_cluster/health?pretty
```

---

## 7. Performance tuning recommendations

- Loki: tune `chunk_target_size`, `max_chunk_age`, and use boltdb-shipper or object storage for large deployments.
- Elasticsearch: use appropriate shard/replica counts, enable ILM, tune refresh interval for indices ingesting logs (`index.refresh_interval`).
- Filebeat/Fluentd: increase `bulk_max_size` or buffer sizes, use backoff settings, and tune resource requests/limits for the DaemonSet.
- Use sampling and log level filtering to reduce volume.

---

## 8. Cost optimization tips

- Retention: set shorter retention for verbose logs; move old logs to cheaper object storage.
- Indexing: avoid indexing full message text when not needed; index only useful fields.
- Compression: enable compression on storage and transport where available.
- Sampling & Aggregation: only keep full logs for errors/high-value traces; aggregate metrics for volume analysis.

---

## Next steps / recommended repo work

- Add example Helm `values.yaml` and Kubernetes manifests for Promtail, Filebeat, and Fluentd to `examples/logging/`.
- Add exported Grafana & Kibana dashboard JSON in `docs/log-dashboards/`.
- Add automated tests or validation scripts to verify DaemonSets can read logs and push to targets.

---

If you'd like, I can now:

- Generate example manifests for each agent in `examples/logging/`.
- Create Grafana and Kibana dashboard JSON templates and add them to `docs/log-dashboards/`.

Tell me which next step you'd like me to take.
