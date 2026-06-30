# API Error Codes and Troubleshooting

This page documents common API error codes and how to troubleshoot them.

## Common Error Responses

### `400 Bad Request`
- Cause: malformed request body or invalid query parameters.
- Fix: validate JSON syntax and required fields.

### `401 Unauthorized`
- Cause: missing or invalid authentication token.
- Fix: confirm the operator and webhook are configured with the correct auth mechanism.

### `403 Forbidden`
- Cause: RBAC or API permission issues.
- Fix: verify Kubernetes service account permissions and admission webhook access.

### `404 Not Found`
- Cause: requested resource does not exist.
- Fix: ensure the `namespace` and `name` are correct and the `StellarNode` resource was created.

### `422 Unprocessable Entity`
- Cause: CRD validation failed or resource spec is invalid.
- Fix: review `docs/api-reference.md` for valid field names and structure.

### `500 Internal Server Error`
- Cause: transient operator failure or webhook processing error.
- Fix: inspect operator logs, restart the operator pod if necessary, and review validation policy logic.

## Debugging Steps

1. Review operator logs for stack traces and reconciliation errors.
2. Confirm the Kubernetes API server can reach the admission webhook endpoint.
3. Validate the CRD manifest against `docs/api-reference.md` and the operator OpenAPI schema.
4. Use the REST API health endpoints (`/healthz`, `/readyz`) to verify the operator state.

## Troubleshooting Examples

- If a field is rejected during `StellarNode` creation, compare the manifest against the CRD schema in `docs/api-reference.md`.
- If the webhook returns `422`, inspect the admission request payload and the validation message for the invalid property.

## Notes

Keep API error documentation aligned with changes to the operator and webhook validation rules. As new endpoints are added, update this page with their error semantics.
