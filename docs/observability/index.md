# Observability

Entry point for metrics, logs, tracing, and dashboards for Stellar-K8s. Start with the reference that matches your task.

## Metrics

- [Metric reference](./metric-reference.md): the metrics the operator and nodes expose, with meanings and labels.
- [Metric naming conventions](./metric-naming-conventions.md): naming rules for new metrics.
- [Metrics and dashboards](./metrics-and-dashboards.md): what to chart and how to wire dashboards.
- [Grafana dashboard guide](../monitoring/GRAFANA_DASHBOARD_GUIDE.md): dashboard import and panels.
- [Monitoring setup guide](../MONITORING_SETUP_GUIDE.md) and [operational runbook](../MONITORING_OPERATIONAL_RUNBOOK.md).

## Logs and health

- [Structured logging](../structured-logging.md): log format and fields.
- [Log aggregation guide](../log-aggregation-guide.md): shipping logs to a central system.
- [Health checks](../health-checks.md): liveness and readiness behaviour.

## Tracing

- [OpenTelemetry tracing](./tracing.md): enabling and configuring traces.

## Architecture and contracts

- [Telemetry architecture](./telemetry-architecture.md): how signals flow from components to backends.
- [Unified observability contract](./unified-contract.md): the resource attributes and schema that telemetry must follow.
