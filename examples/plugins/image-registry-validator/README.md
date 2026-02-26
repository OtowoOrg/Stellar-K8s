# Image Registry Validator Plugin

This is an example WebAssembly validation plugin for the Stellar Kubernetes Operator. It demonstrates how to write custom validation policies that enforce organizational requirements.

## What It Does

This plugin validates that StellarNode resources only use container images from approved registries:

- `docker.io/stellar/*`
- `ghcr.io/stellar/*`
- `gcr.io/stellar-project/*`

It also provides warnings for potentially problematic configurations, such as low memory limits.

## Building the Plugin

### Prerequisites

- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-opt` (optional, for size optimization)

```bash
# Install the Wasm target
rustup target add wasm32-unknown-unknown

# Install wasm-opt (optional)
cargo install wasm-opt
```

### Build

```bash
# Build the plugin
cargo build --target wasm32-unknown-unknown --release

# The output will be at:
# target/wasm32-unknown-unknown/release/image_registry_validator.wasm

# Optional: Optimize the Wasm binary
wasm-opt -Oz -o image_registry_validator_opt.wasm \
    target/wasm32-unknown-unknown/release/image_registry_validator.wasm
```

## Deploying the Plugin

### Option 1: ConfigMap

```bash
# Create a ConfigMap with the Wasm binary
kubectl create configmap image-registry-validator \
    --from-file=plugin.wasm=target/wasm32-unknown-unknown/release/image_registry_validator.wasm \
    -n stellar-operator-system
```

### Option 2: Direct Upload via API

```bash
# Base64 encode the Wasm binary
WASM_BASE64=$(base64 < target/wasm32-unknown-unknown/release/image_registry_validator.wasm)

# Load the plugin via the webhook API
curl -X POST http://localhost:8443/plugins \
    -H "Content-Type: application/json" \
    -d '{
        "metadata": {
            "name": "image-registry-validator",
            "version": "0.1.0",
            "description": "Validates that images come from approved registries",
            "author": "Stellar Team"
        },
        "wasm_binary": "'$WASM_BASE64'",
        "operations": ["CREATE", "UPDATE"],
        "enabled": true,
        "fail_open": false
    }'
```

## Testing the Plugin

### Test with an approved image:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-validator
spec:
  nodeType: Validator
  network: Testnet
  version: "docker.io/stellar/stellar-core:v21.3.0"  # Approved
  # ... rest of spec
```

### Test with an unapproved image:

```yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-validator
spec:
  nodeType: Validator
  network: Testnet
  version: "quay.io/myorg/stellar-core:v21.3.0"  # Not approved - will be rejected
  # ... rest of spec
```

## Plugin Interface

### Input

The plugin receives a JSON object with the following structure:

```json
{
  "operation": "CREATE",
  "object": { /* StellarNode spec */ },
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

### Output

The plugin returns a JSON object:

```json
{
  "allowed": false,
  "message": "Image 'quay.io/myorg/stellar-core:v21.3.0' is not from an approved registry",
  "reason": "PolicyViolation",
  "errors": [
    {
      "field": "spec.version",
      "message": "Image is not from an approved registry",
      "errorType": "InvalidRegistry"
    }
  ],
  "warnings": [],
  "auditAnnotations": {
    "image-registry-validator.stellar.org/checked": "true",
    "image-registry-validator.stellar.org/version": "quay.io/myorg/stellar-core:v21.3.0"
  }
}
```

## Customizing the Plugin

To modify the approved registries, edit the `APPROVED_REGISTRIES` constant in `src/lib.rs`:

```rust
const APPROVED_REGISTRIES: &[&str] = &[
    "docker.io/stellar/",
    "ghcr.io/stellar/",
    "gcr.io/stellar-project/",
    "your-registry.example.com/stellar/",  // Add your registry
];
```

## Writing Your Own Plugin

Use this as a template for your own validation plugins. Key points:

1. **Export a `validate()` function** that returns 0 for success, non-zero for failure
2. **Use the host functions** to read input and write output:
   - `get_input_len()` - Get input size
   - `read_input(ptr, len)` - Read input data
   - `write_output(ptr, len)` - Write output data
   - `log_message(ptr, len)` - Log debug messages
3. **Keep it small** - Wasm plugins should be fast and lightweight
4. **Handle errors gracefully** - Always return valid JSON output
5. **Use audit annotations** - Add metadata for audit trails

## Security Considerations

- Plugins run in a sandboxed Wasm environment with no filesystem or network access
- Memory and CPU limits are enforced by the runtime
- Plugins cannot access Kubernetes API directly
- All input/output goes through the controlled host interface

## Performance

This example plugin:
- Binary size: ~50KB (optimized)
- Execution time: <5ms typical
- Memory usage: <1MB

## License

Apache 2.0
