# Stellar-K8s API Reference

This directory contains the API reference and integration documentation for Stellar-K8s.

## Contents

- [StellarNode CRD Reference](../api-reference.md) - field-level CRD schema (generated)
- [OpenAPI Specification](openapi.yaml) - operator REST API (Swagger-compatible)
- [API Versioning Strategy](versioning.md) - URL-path versions, deprecation, migration
- [Production Profiling Runbook](../operations/profiling-runbook.md) - CPU/heap pprof endpoints
- [Webhook API](webhook.md)
- [Metrics API](metrics.md)
- [Client Libraries and SDK Guidance](client-libraries.md)
- [Error Codes and Troubleshooting](error-codes.md)

## Swagger / OpenAPI

| Artifact | Location |
|----------|----------|
| Static OpenAPI 3.0 spec | `docs/api/openapi.yaml` |
| Live JSON (API gateway) | `GET /gateway/openapi.json` |
| Interactive Swagger UI | `GET /developer` (when gateway enabled) |

Validate the static spec locally:

```bash
make generate-openapi-spec
make check-openapi-spec
```

## Overview

Stellar-K8s exposes the following integration layers:

- `StellarNode` CRD definitions and validation rules
- Operator REST API for cluster management, health, and diagnostics
- Admission webhook request/response validation for CRD operations
- Prometheus-compatible metrics and observability endpoints

## Notes

- The canonical CRD schema is documented in [docs/api-reference.md](../api-reference.md).
- Use the [OpenAPI specification](openapi.yaml) for code generation and API clients.
- Refer to [client-libraries.md](client-libraries.md) for SDK guidance and integration patterns.

## API Contract Testing (Issue #1288)

Automated contract testing ensures every API endpoint conforms to the OpenAPI specification.

### Running Contract Tests

```bash
# Validate spec structure and endpoint coverage
make check-api-contract

# Check endpoint coverage (must exceed 90%)
make check-api-coverage

# Detect breaking changes against base branch
make check-breaking-changes
```

### What Is Checked

| Check | Description |
|-------|-------------|
| Spec validation | OpenAPI 3.0 structure and syntax |
| Endpoint coverage | Every implemented route is documented (≥90% required) |
| Request schemas | Required fields, types, constraints |
| Response schemas | Status codes, content types, field definitions |
| Authentication | Protected endpoints have security requirements |
| Error responses | 401/404/500 responses documented |
| Breaking changes | Removed endpoints, new required fields, removed response fields |

### Endpoint Coverage

The coverage report shows each endpoint's documentation completeness:

```
Method   Path                                          Doc  Schema  Err  Auth
-------- --------------------------------------------- ---- ------ ---- ----
GET      /health                                        ✓      ✓     ·    ·
GET      /api/v1/nodes                                  ✓      ✓     ✓    ✓
POST     /config/log-level                              ✓      ✓     ✓    ✓
```

Coverage score = (documented + has_response_schema + has_error_responses + auth_correct) / 4

### Breaking Change Detection

On pull requests, the CI compares the PR's OpenAPI spec against the base branch and detects:

- **Removed endpoints** — endpoint present in base but not in head
- **Removed response fields** — fields present in base 200 response but not in head
- **Newly required fields** — previously optional fields now required in request body
- **Auth changes** — protected endpoints losing security requirements
- **Removed error codes** — documented error responses removed

### Intentionally Breaking Changes

If an API change is intentionally breaking:

1. Update `docs/api/openapi.yaml` with the new contract
2. Update any affected client code
3. Document the migration in CHANGELOG.md
4. The breaking-change detector will flag it — this is expected for intentional changes
