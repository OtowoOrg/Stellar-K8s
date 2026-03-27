# Stellar-K8s Error Codes

This document provides details on all error variants encountered in the Stellar-K8s operator, their causes, and how to resolve them.

| Error Code | Name | Description | Resolution Steps |
| --- | --- | --- | --- |
| **SK8S-001** | `KubeError` | Kubernetes API error returned from `kube-rs`. | Check the Kubernetes cluster status and the accessibility of the API server. Review RBAC permissions for the operator. |
| **SK8S-002** | `SerializationError` | JSON serialization/deserialization failed. | Ensure that all custom resource definitions (CRDs) precisely match the schema of the Operator and that there are no malformed fields in your specifications. |
| **SK8S-003** | `FinalizerError` | A finalizer failed to execute during resource cleanup. | Examine operator deployment logs to understand what finalizer sub-task failed (e.g., dangling resources that can't be deleted). |
| **SK8S-004** | `ConfigError` | The operator's or resource's configuration is invalid. | Review the provided configuration for typos and validate values against the supported schemas constraints. |
| **SK8S-005** | `ValidationError` | Node specification validation failed. | Check the StellarNode Custom Resource (CR) fields validation rules. Some combined parameters may be incompatible or invalid. |
| **SK8S-006** | `NotFound` | The requested resource was not found. | Ensure that the resource actually exists in the specified namespace and spelling is correct. |
| **SK8S-007** | `InvalidNodeType` | An invalid node type was requested. | Validate the `node_type` field. Allowed node types are specific to the Stellar network topology (e.g., basic, validator, watcher). |
| **SK8S-008** | `MissingRequiredField` | A required field for the node type is missing. | Complete the node specification by providing all mandatory fields corresponding to the specified node type. |
| **SK8S-009** | `ArchiveHealthCheckError` | History archive health check failed. | The history archive might be unreachable, corrupted, or not synchronized correctly. Ensure network connectivity to the archive storage endpoint. |
| **SK8S-010** | `HttpError` | HTTP request error (typically when calling external or internal APIs). | Check your cluster's internet availability or network policies that might be blocking external/internal communication. |
| **SK8S-011** | `RemediationError` | An auto-remediation task failed during execution. | Check operator logs for the remediation sequence. Common causes include insufficient permissions, or the target pod/node is in an unstable state. |
| **SK8S-012** | `PluginError` | Error related to a dynamic Wasm plugin execution. | Ensure the WASM plugin is correctly compiled securely without internal panics, and its dependencies are satisfied. |
| **SK8S-013** | `WebhookError` | A webhook server error occurred. | Ensure webhook certificates are valid, properly installed, and the webhook service is correctly targeting matching operator pods. |
| **SK8S-014** | `NetworkError` | General network connectivity error encountered. | Look for service disruptions within the Kubernetes environment, such as CNI issues or routing issues across pods. |
| **SK8S-015** | `CertificateError` | Generating or loading certificates failed. | Review certificate configurations, especially if relying on tools like `cert-manager`. Certificates could be expired or generated maliciously. |
| **SK8S-016** | `IoError` | Standard Input/Output operational error. | Ensure the operator has sufficient privileges to interact with filesystem paths it’s expected to access (mounts, caching paths). |
| **SK8S-017** | `MaintenanceError` | Stellar node database maintenance failed. | Typical reasons include PostgreSQL resource exhaustion, permission issues, or conflicting processes locking the DB tables. |
| **SK8S-018** | `SqlxError` | General SQL database execution error. | Directly check the node database connectivity. Look for slow query executions or out-of-memory errors on the DB instance. |

## General Troubleshooting
When encountering these errors, the primary source of detailed insight will be the operator logs. You can fetch them with:
```bash
kubectl logs -n stellar-k8s-system deploy/stellar-operator
```
Look for the `[SK8S-XXX]` prefix in the logging output for rapid filtering.
