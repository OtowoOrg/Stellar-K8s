# Operator Environment Variables Cheat Sheet

Quick reference for the key environment variables used by the Stellar-K8s operator.

| Variable | Default | Description |
| :--- | :--- | :--- |
| `STELLAR_OPERATOR_CONFIG` | _(none)_ | Path to the operator configuration file. |
| `STELLAR_OPERATOR_IMAGE` | _(none)_ | Container image reference used by the operator for managed workloads. |
| `OPERATOR_NAMESPACE` | `default` | Kubernetes namespace where the operator and webhooks run. |
| `OPERATOR_LEASE_NAME` | `stellar-operator-leader` | Name of the Kubernetes `Lease` resource used for leader election. |
| `POD_NAMESPACE` | `default` | Namespace of the running operator pod (typically injected via Downward API). |
| `POD_NAME` | `HOSTNAME` | Name of the running operator pod (falls back to `HOSTNAME`). |
| `REST_API_PORT` | `8080` | Port on which the operator REST API server listens. |
| `RUST_LOG` | `info` | Log level filter (e.g., `info`, `debug`, `stellar_k8s=debug`). |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | _(disabled)_ | OTLP gRPC/HTTP endpoint for OpenTelemetry trace export. |
| `SCHEMA_MIGRATOR_ENABLED` | `false` | Enables automatic CRD/schema migration checks on startup. |

## Related Documentation

- [Operations Overview](../operations/index.md)
- [CRD Architecture Manual](crd-manual.md)
