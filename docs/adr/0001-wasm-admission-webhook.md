# ADR 0001: Wasm-Based Admission Webhook for Custom Validation

## Status

Accepted

## Context

The Stellar-K8s operator needs a flexible way to validate `StellarNode` custom resources before they are persisted to the Kubernetes API server. Different organizations have varying requirements for validation:

- **Compliance Requirements**: Organizations may need to enforce specific security policies, resource quotas, or naming conventions
- **Environment-Specific Rules**: Production clusters may have stricter validation than development environments
- **Custom Business Logic**: Teams may want to validate against external systems (e.g., checking if a node is registered in an inventory system)
- **Evolving Requirements**: Validation rules change over time and shouldn't require operator redeployment

Traditional approaches to admission webhooks have limitations:

1. **Native Webhook**: Validation logic is compiled into the operator binary, requiring recompilation and redeployment for any rule changes
2. **External Webhook Service**: Requires deploying and maintaining separate services, increasing operational complexity
3. **OPA/Rego**: While flexible, requires learning a new policy language and doesn't provide the full power of a general-purpose language

We needed a solution that provides:
- **Flexibility**: Easy to add, update, or remove validation rules without operator downtime
- **Security**: Validation code runs in a sandboxed environment with resource limits
- **Performance**: Fast execution with minimal overhead
- **Developer Experience**: Use familiar programming languages (Rust, Go, etc.)
- **Isolation**: Plugin failures shouldn't crash the operator

## Decision

We will implement a **WebAssembly (Wasm)-based admission webhook** that allows custom validation logic to be loaded as Wasm plugins.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Kubernetes API Server                     │
└─────────────────────────────┬───────────────────────────────┘
                              │ AdmissionReview
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Webhook Server (Rust)                     │
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

### Key Components

1. **Wasmtime Runtime** (`src/webhook/runtime.rs`):
   - Executes Wasm plugins in a sandboxed environment
   - Enforces resource limits (memory, CPU, time)
   - Provides host functions for input/output
   - Supports parallel plugin execution

2. **Webhook Server** (`src/webhook/server.rs`):
   - Handles Kubernetes AdmissionReview requests
   - Manages plugin lifecycle (load, unload, list)
   - Aggregates results from multiple plugins
   - Provides REST API for plugin management

3. **Plugin Interface** (`src/webhook/types.rs`):
   - Standardized input/output format (JSON)
   - Validation result structure
   - Error reporting and warnings

### Plugin Interface

Plugins must export a `validate()` function:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Read input via host functions
    let input: ValidationInput = read_input();
    
    // Perform validation
    let is_valid = validate_stellar_node(&input);
    
    // Write output
    write_output(&ValidationOutput {
        allowed: is_valid,
        message: Some("Validation complete".to_string()),
        errors: vec![],
        warnings: vec![],
    });
    
    if is_valid { 0 } else { 1 }
}
```

### Security Features

1. **Sandboxing**:
   - No filesystem access
   - No network access
   - No access to host environment variables
   - Isolated memory space per plugin

2. **Resource Limits**:
   - **Memory**: Default 16MB per plugin (configurable)
   - **CPU**: Fuel metering limits instruction count (default 1M instructions)
   - **Time**: Epoch-based interruption with 1-second timeout (configurable)

3. **Integrity Verification**:
   - SHA256 checksums for plugin binaries
   - Prevents tampering and ensures authenticity

4. **Fail-Open/Fail-Close**:
   - Configurable behavior when plugins fail
   - Fail-close (default): Deny request if plugin fails
   - Fail-open: Allow request with warning if plugin fails

### Plugin Management

Plugins can be loaded via:

1. **REST API**: POST /plugins with base64-encoded Wasm binary
2. **ConfigMap**: Reference Wasm binary stored in ConfigMap
3. **Secret**: Reference Wasm binary stored in Secret (for sensitive plugins)
4. **URL**: Download from external source (with integrity check)

## Consequences

### Positive

1. **Flexibility**: Validation rules can be updated without operator redeployment
2. **Security**: Wasm sandbox provides strong isolation guarantees
3. **Performance**: Wasm execution is near-native speed (~95% of native performance)
4. **Language Agnostic**: Plugins can be written in any language that compiles to Wasm (Rust, Go, C++, AssemblyScript)
5. **Composability**: Multiple plugins can be chained together
6. **Testability**: Plugins can be tested independently of the operator
7. **Portability**: Wasm binaries are platform-independent
8. **Resource Efficiency**: Plugins share the operator's process, no separate containers needed
9. **Fail-Safe**: Plugin failures are isolated and don't crash the operator

### Negative

1. **Complexity**: Adds Wasmtime runtime dependency and plugin management logic
2. **Learning Curve**: Developers need to understand Wasm compilation and the plugin interface
3. **Debugging**: Debugging Wasm plugins is harder than native code
4. **Binary Size**: Wasmtime adds ~5MB to the operator binary
5. **Compilation Step**: Plugins must be compiled to Wasm before use
6. **Limited Standard Library**: Wasm plugins have restricted access to standard library features (no filesystem, network)
7. **Performance Overhead**: Small overhead (~5%) compared to native code due to sandboxing

### Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Plugin execution timeout blocks webhook | Epoch-based interruption ensures hard timeout |
| Malicious plugin consumes excessive memory | Memory limits enforced by Wasmtime |
| Plugin infinite loop | Fuel metering limits instruction count |
| Plugin crashes operator | Wasm sandbox isolates failures |
| Plugin binary tampering | SHA256 integrity verification |
| Plugin compatibility issues | Validate plugin exports required functions |

## Alternatives Considered

### 1. Native Webhook (Compiled into Operator)

**Pros**:
- Simplest implementation
- Best performance (no sandboxing overhead)
- Easy to debug

**Cons**:
- Requires operator recompilation for rule changes
- Requires operator redeployment and downtime
- No isolation between validation logic and operator
- All validation logic must be in Rust

**Verdict**: Rejected due to lack of flexibility

### 2. External Webhook Service

**Pros**:
- Complete isolation from operator
- Can use any language/framework
- Independent scaling

**Cons**:
- Requires deploying and maintaining separate services
- Network latency for webhook calls
- Additional operational complexity
- Requires separate RBAC, TLS certificates, monitoring
- Higher resource usage (separate pods)

**Verdict**: Rejected due to operational complexity

### 3. Open Policy Agent (OPA) with Rego

**Pros**:
- Industry-standard policy engine
- Declarative policy language
- Good tooling and ecosystem

**Cons**:
- Requires learning Rego (domain-specific language)
- Limited to policy evaluation (no complex logic)
- Harder to integrate with external systems
- Separate OPA deployment or sidecar required

**Verdict**: Rejected due to limited expressiveness and additional deployment

### 4. Lua Scripting

**Pros**:
- Lightweight and fast
- Easy to embed
- Simple scripting language

**Cons**:
- Less secure than Wasm (no strong sandboxing)
- Limited type safety
- Smaller ecosystem than Wasm
- Not as portable

**Verdict**: Rejected due to weaker security guarantees

### 5. gRPC Plugin System

**Pros**:
- Language-agnostic
- Well-defined interface
- Good performance

**Cons**:
- Requires separate plugin processes
- More complex deployment
- Higher resource usage
- Network overhead

**Verdict**: Rejected due to operational complexity

## Implementation Notes

### Wasmtime Configuration

```rust
let mut config = Config::new();
config.consume_fuel(true);           // Enable fuel metering
config.epoch_interruption(true);     // Enable timeout
config.max_wasm_stack(512 * 1024);   // 512KB stack
config.wasm_threads(false);          // Disable threads
config.wasm_simd(true);              // Enable SIMD
```

### Host Functions

The runtime provides these host functions to plugins:

- `get_input_len()`: Get length of input JSON
- `read_input(ptr, len)`: Read input into plugin memory
- `write_output(ptr, len)`: Write output from plugin memory
- `log_message(ptr, len)`: Log debug messages

### Plugin Compilation

```bash
# Install Wasm target
rustup target add wasm32-wasi

# Compile plugin
cargo build --target wasm32-wasi --release

# Optimize (optional)
wasm-opt -Oz -o plugin.wasm target/wasm32-wasi/release/plugin.wasm
```

## References

- [Kubernetes Admission Webhooks](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/)
- [WebAssembly](https://webassembly.org/)
- [Wasmtime](https://wasmtime.dev/)
- [WASI (WebAssembly System Interface)](https://wasi.dev/)
- [Wasm Webhook Implementation](../wasm-webhook.md)
- [Plugin Example](../../examples/plugins/example-validator/)

## Decision Makers

- Otowo Samuel (Maintainer)

## Date

2024-02-24

## Notes

This ADR documents the design decision made during the initial implementation of the admission webhook feature. The Wasm-based approach provides the best balance of flexibility, security, and performance for our use case.

Future enhancements may include:
- Plugin versioning and rollback
- Plugin dependency management
- Hot-reloading of plugins
- Plugin marketplace/registry
- Enhanced debugging tools
- Support for mutating webhooks (currently validating only)
