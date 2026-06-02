# Client Libraries and SDK Guidance

This document provides guidance for writing SDKs and integration clients for Stellar-K8s.

## Current Status

No official client SDK exists in this repository today. Integrators may use the OpenAPI specification in `openapi.yaml` to generate client code.

## Recommended Integration Patterns

- Use the `StellarNode` CRD and Kubernetes API for declarative cluster management.
- Use the operator REST API for health, diagnostics, and workflow automation.
- Use the admission webhook and CRD validation metadata for pre-flight checks.

## Example SDK Targets

- Go: generate with `oapi-codegen` or `go-swagger`
- Python: generate with `openapi-python-client`
- JavaScript/TypeScript: generate with `openapi-generator-cli`

## Sample Integration Workflows

1. List available clusters and identify invalid nodes.
2. Submit a `StellarNode` manifest through Kubernetes API.
3. Query operator health and reconciliation state using the REST API.
4. Monitor `/metrics` for stability and performance.

## Future Work

- Publish a first-party SDK when the REST API stabilizes.
- Add language-specific examples for Go, Python, and JavaScript.
- Generate SDK docs from the OpenAPI spec and keep them synchronized with releases.
