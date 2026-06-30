# Stellar-K8s API Reference

This directory contains the API reference and integration documentation for Stellar-K8s.

## Contents

- [CRD API Reference](../api-reference.md)
- [OpenAPI Specification](openapi.yaml)
- [Webhook API](webhook.md)
- [Metrics API](metrics.md)
- [Client Libraries and SDK Guidance](client-libraries.md)
- [Error Codes and Troubleshooting](error-codes.md)

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
