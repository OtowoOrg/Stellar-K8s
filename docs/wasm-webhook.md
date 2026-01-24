# Wasm Admission Webhook

The Stellar-K8s operator includes a Wasm-based admission webhook that allows you to implement custom validation logic for `StellarNode` resources using WebAssembly plugins.

## Overview

The admission webhook intercepts `CREATE` and `UPDATE` requests for `StellarNode` resources and executes one or more Wasm plugins to validate them. This provides:

- **Custom Validation Logic**: Implement organization-specific policies
- **Secure Sandboxing**: Plugins run in an isolated Wasm environment
- **Resource Limits**: Configurable memory, CPU, and time limits
- **Fail-Open/Fail-Close**: Configure behavior when plugins fail
- **Audit Logging**: Track validation decisions

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Kubernetes API Server                     │
└─────────────────────────────┬───────────────────────────────┘
                              │ AdmissionReview
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Webhook Server                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │   Plugin 1  │  │   Plugin 2  │  │   Plugin N  │          │
│  │   (Wasm)    │  │   (Wasm)    │  │   (Wasm)    │          │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘          │
│         │                │                │                  │
│  ┌──────▼────────────────▼────────────────▼──────┐          │
│  │            Wasmtime Runtime                    │          │
│  │  • Memory limits   • Fuel metering            │          │
│  │  • Timeout control • No filesystem/network    │          │
│  └───────────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────────────┘
```

## Installation

### Prerequisites

- Kubernetes cluster 1.19+
- cert-manager installed (for TLS certificates)
- kubectl configured

### Deploy the Webhook

```bash
# Install cert-manager if not present
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.14.0/cert-manager.yaml

# Wait for cert-manager to be ready
kubectl wait --for=condition=Available deployment/cert-manager-webhook -n cert-manager --timeout=60s

# Deploy the webhook
kubectl apply -f charts/stellar-operator/templates/webhook.yaml
```

### Verify Installation

```bash
# Check webhook pods
kubectl get pods -n stellar-webhook

# Check webhook service
kubectl get svc -n stellar-webhook

# Check ValidatingWebhookConfiguration
kubectl get validatingwebhookconfiguration stellar-webhook
```

## Writing Validation Plugins

### Plugin Interface

Plugins must export the following functions:

```rust
// Required: Main validation entry point
// Returns 0 for success, non-zero for failure
#[no_mangle]
pub extern "C" fn validate() -> i32;

// Required: Memory export for host communication
#[no_mangle]
pub static mut MEMORY: [u8; 65536];
```

### Host Functions

The runtime provides these host functions:

```rust
extern "C" {
    // Get the length of the input JSON
    fn get_input_len() -> i32;
    
    // Read input into plugin memory
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    
    // Write output from plugin memory
    fn write_output(ptr: *const u8, len: i32) -> i32;
    
    // Log a debug message
    fn log_message(ptr: *const u8, len: i32);
}
```

### Input Format

```json
{
  "operation": "CREATE",
  "object": {
    "apiVersion": "stellar.io/v1alpha1",
    "kind": "StellarNode",
    "metadata": {
      "name": "my-node",
      "namespace": "default"
    },
    "spec": {
      "nodeType": "horizon",
      "network": "testnet"
    }
  },
  "oldObject": null,
  "namespace": "default",
  "name": "my-node",
  "userInfo": {
    "username": "admin",
    "groups": ["system:masters"]
  },
  "context": {}
}
```

### Output Format

```json
{
  "allowed": true,
  "message": "Validation passed",
  "warnings": ["Consider adding resource limits"],
  "errors": [
    {
      "field": "spec.replicas",
      "message": "Replicas must be positive",
      "type": "FieldValueInvalid"
    }
  ],
  "auditAnnotations": {
    "stellar.io/validated-by": "my-plugin"
  }
}
```

### Example Plugin (Rust)

See [examples/plugins/example-validator](../examples/plugins/example-validator/) for a complete example.

```rust
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ValidationInput {
    operation: String,
    object: Option<StellarNode>,
    // ... other fields
}

#[derive(Serialize)]
struct ValidationOutput {
    allowed: bool,
    message: Option<String>,
    warnings: Vec<String>,
    errors: Vec<ValidationError>,
}

#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Read input
    let input: ValidationInput = read_validation_input();
    
    // Validate
    let output = if is_valid(&input) {
        ValidationOutput { allowed: true, .. }
    } else {
        ValidationOutput { allowed: false, .. }
    };
    
    // Write output
    write_validation_output(&output);
    
    if output.allowed { 0 } else { 1 }
}
```

### Building Plugins

```bash
# Install wasm32-wasi target
rustup target add wasm32-wasi

# Build the plugin
cd examples/plugins/example-validator
cargo build --target wasm32-wasi --release

# The plugin is at target/wasm32-wasi/release/example_validator.wasm
```

## Managing Plugins

### Loading Plugins via API

```bash
# Encode the Wasm binary
WASM_BASE64=$(base64 < example_validator.wasm)

# Load the plugin
curl -X POST https://stellar-webhook.stellar-webhook:443/plugins \
  -H "Content-Type: application/json" \
  -d @- <<EOF
{
  "metadata": {
    "name": "example-validator",
    "version": "1.0.0",
    "description": "Example validation plugin",
    "sha256": "$(sha256sum example_validator.wasm | cut -d' ' -f1)"
  },
  "wasm_binary": "${WASM_BASE64}",
  "operations": ["Create", "Update"],
  "enabled": true,
  "fail_open": false
}
EOF
```

### Loading Plugins via ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: stellar-webhook-plugins
  namespace: stellar-webhook
data:
  example-validator.wasm: |
    <base64-encoded-wasm>
  plugins.json: |
    {
      "plugins": [
        {
          "metadata": {
            "name": "example-validator",
            "version": "1.0.0"
          },
          "configMapRef": {
            "name": "stellar-webhook-plugins",
            "key": "example-validator.wasm"
          },
          "operations": ["Create", "Update"],
          "enabled": true
        }
      ]
    }
```

### Listing Plugins

```bash
curl https://stellar-webhook.stellar-webhook:443/plugins
```

### Removing Plugins

```bash
curl -X DELETE https://stellar-webhook.stellar-webhook:443/plugins/example-validator
```

## Configuration

### Plugin Limits

Each plugin can have custom resource limits:

```json
{
  "metadata": {
    "name": "my-plugin",
    "limits": {
      "timeout_ms": 1000,
      "max_memory_bytes": 16777216,
      "max_fuel": 1000000
    }
  }
}
```

| Setting | Default | Description |
|---------|---------|-------------|
| `timeout_ms` | 1000 | Maximum execution time in milliseconds |
| `max_memory_bytes` | 16MB | Maximum memory the plugin can allocate |
| `max_fuel` | 1,000,000 | Maximum Wasm instructions (fuel units) |

### Fail-Open vs Fail-Close

- **Fail-Close** (`fail_open: false`): If a plugin fails, the admission request is denied
- **Fail-Open** (`fail_open: true`): If a plugin fails, the request is allowed with a warning

### Webhook Timeout

The Kubernetes ValidatingWebhookConfiguration has a 10-second timeout:

```yaml
webhooks:
  - name: stellarnode.stellar.io
    timeoutSeconds: 10
```

## Security Considerations

### Plugin Sandboxing

Plugins run in a secure Wasm sandbox with:

- **No filesystem access**: Plugins cannot read or write files
- **No network access**: Plugins cannot make network connections
- **Memory isolation**: Each plugin gets its own memory space
- **Instruction limits**: Fuel metering prevents infinite loops
- **Time limits**: Epoch-based interruption for timeouts

### Plugin Integrity

Use SHA256 checksums to verify plugin integrity:

```json
{
  "metadata": {
    "sha256": "a3b2c1d4e5f67890..."
  }
}
```

### RBAC

The webhook service account has minimal permissions:

```yaml
rules:
  - apiGroups: ["stellar.io"]
    resources: ["stellarnodes"]
    verbs: ["get", "list", "watch"]
  - apiGroups: [""]
    resources: ["configmaps", "secrets"]
    verbs: ["get", "list", "watch"]
```

## Troubleshooting

### Check Webhook Logs

```bash
kubectl logs -n stellar-webhook -l app.kubernetes.io/name=stellar-webhook
```

### Test Validation

```bash
# Create a test StellarNode
kubectl apply -f - <<EOF
apiVersion: stellar.io/v1alpha1
kind: StellarNode
metadata:
  name: test-validation
spec:
  nodeType: horizon
  network: testnet
EOF

# Check the response
kubectl get stellarnode test-validation -o yaml
```

### Debug Mode

Enable debug logging:

```yaml
env:
  - name: RUST_LOG
    value: "debug,stellar_k8s=trace"
```

### Common Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| Webhook timeout | Plugin too slow | Increase timeout or optimize plugin |
| Certificate error | TLS misconfiguration | Check cert-manager certificates |
| Plugin not found | Plugin not loaded | Verify plugin is in the plugins list |
| Memory limit exceeded | Plugin uses too much memory | Increase `max_memory_bytes` |

## Metrics

The webhook exposes Prometheus metrics on port 9090:

| Metric | Type | Description |
|--------|------|-------------|
| `webhook_requests_total` | Counter | Total validation requests |
| `webhook_request_duration_seconds` | Histogram | Request processing time |
| `plugin_executions_total` | Counter | Plugin execution count |
| `plugin_execution_duration_seconds` | Histogram | Plugin execution time |
| `plugin_errors_total` | Counter | Plugin execution errors |

## API Reference

### POST /validate

Kubernetes admission webhook endpoint.

### GET /health, GET /healthz

Health check endpoint.

### GET /ready

Readiness check endpoint (requires at least one plugin loaded).

### GET /plugins

List loaded plugins.

### POST /plugins

Load a new plugin.

### DELETE /plugins/:name

Unload a plugin.
