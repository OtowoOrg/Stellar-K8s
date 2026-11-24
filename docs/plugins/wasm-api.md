# Wasm Plugin Development — API Reference

This reference covers everything a plugin author needs to build, test, and deploy a
WebAssembly validation plugin for the Stellar Kubernetes Operator.

> **Related documents**
> - [Hello World Tutorial](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/wasm-plugins/hello-world/README.md) — step-by-step first plugin
> - [Sandboxing Troubleshooting Guide](./wasm-troubleshooting.md) — common runtime errors

---

## Table of Contents

1. [Overview](#overview)
2. [Memory Model](#memory-model)
3. [Plugin Contract](#plugin-contract)
4. [Host Functions](#host-functions)
5. [Input Data Structure](#input-data-structure)
6. [Output Data Structure](#output-data-structure)
7. [Supporting Types](#supporting-types)
8. [Resource Limits & Sandboxing](#resource-limits-sandboxing)
9. [Plugin Lifecycle](#plugin-lifecycle)
10. [REST Management API](#rest-management-api)
11. [Configuration Reference](#configuration-reference)
12. [Security Model](#security-model)
13. [Performance Guidelines](#performance-guidelines)
14. [Versioning & Stability](#versioning-stability)

---

## Overview

The Stellar-K8s operator embeds a [Wasmtime](https://wasmtime.dev/) runtime that executes
user-supplied Wasm modules as Kubernetes admission webhooks.  Each plugin receives a
serialised `ValidationInput` object, applies its policy, and returns a `ValidationOutput`
object.  Communication happens through four host functions imported from the `env` module.

```
Kubernetes API Server
        │
        │ AdmissionReview (CREATE / UPDATE / DELETE / CONNECT)
        ▼
┌───────────────────────────────────┐
│  Stellar-K8s Admission Webhook    │
│  ┌─────────────────────────────┐  │
│  │  Wasmtime Sandbox           │  │
│  │  ┌─────────────────────┐    │  │
│  │  │  plugin.wasm        │    │  │
│  │  │  · validate() → i32 │    │  │
│  │  └────────┬────────────┘    │  │
│  │           │ host functions   │  │
│  │  ┌────────▼────────────┐    │  │
│  │  │  env module         │    │  │
│  │  │  get_input_len      │    │  │
│  │  │  read_input         │    │  │
│  │  │  write_output       │    │  │
│  │  │  log_message        │    │  │
│  │  └─────────────────────┘    │  │
│  └─────────────────────────────┘  │
└───────────────────────────────────┘
```

### Supported languages

Any language that targets `wasm32-unknown-unknown` or `wasm32-wasi` and can
produce a C-ABI export named `validate` works.  The examples in this repository
use **Rust**.

---

## Memory Model

### Linear memory

Each plugin instance owns a single, contiguous **linear memory** region.  The
runtime allocates memory in 64 KiB pages.

| Parameter | Value |
|---|---|
| Default maximum memory | 16 MiB (256 pages) |
| Configurable maximum | up to operator limit (see `maxMemoryBytes`) |
| Wasm stack size | 512 KiB (enforced by Wasmtime) |
| Shared memory / threads | **disabled** |

The plugin **must** export its linear memory as `memory`.  This is the default
behaviour for `cdylib` Rust crates targeting `wasm32-unknown-unknown` — no
extra configuration is required.

### Ownership rules

The host owns two logical byte regions that it exposes to the guest through host
functions:

| Region | Owner | Access |
|---|---|---|
| Input buffer | Host | Read-only from guest (via `read_input`) |
| Output buffer | Host | Write-only from guest (via `write_output`) |
| All other guest memory | Guest | Freely usable by the plugin |

The guest must **never** retain a raw pointer past a host-function call boundary;
the host may move its buffers between calls.

### Allocation strategy (Rust)

With `wasm32-unknown-unknown` the standard global allocator (`dlmalloc`) is
linked automatically.  No `#[global_allocator]` annotation is required.  For
size-critical plugins, replace it with [`wee_alloc`](https://github.com/rustwasm/wee_alloc):

```toml
# Cargo.toml
[dependencies]
wee_alloc = "0.4"
```

```rust
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;
```

---

## Plugin Contract

### Required exports

Every plugin **must** export the following symbols or the runtime will reject it
at load time:

| Export | Type | Description |
|---|---|---|
| `validate` | `() -> i32` | Main entry point called for each admission request |
| `memory` | memory | Linear memory shared with the host |

### `validate()` return codes

| Return value | Meaning |
|---|---|
| `0` | Validation succeeded (host uses `ValidationOutput.allowed` to decide) |
| `1` | Validation failed (host also reads `ValidationOutput.allowed`) |
| Any other value | Treated as an internal plugin error; request is denied |

The return code is a secondary signal.  The host always parses the JSON written
by `write_output`; if the output buffer is empty the return code alone drives the
decision.

### Minimal skeleton (Rust)

```rust
//! my_plugin/src/lib.rs

extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}

#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // 1. read input
    let json = unsafe {
        let len = get_input_len();
        let mut buf = vec![0u8; len as usize];
        read_input(buf.as_mut_ptr(), len);
        buf
    };

    // 2. parse, validate, build output …
    let output = br#"{"allowed":true,"message":"ok"}"#;

    // 3. write output
    unsafe { write_output(output.as_ptr(), output.len() as i32) };
    0
}
```

---

## Host Functions

All host functions are imported from the **`env`** module.  Declare them with
`extern "C"` in Rust (or the equivalent foreign-function block in your language).

### `get_input_len() -> i32`

Returns the byte length of the serialised `ValidationInput` JSON waiting in the
host input buffer.

```
signature : () -> i32
module    : env
```

| Return | Meaning |
|---|---|
| `> 0` | Number of bytes to allocate before calling `read_input` |
| `0` | Input buffer is empty (should not occur in normal operation) |
| `< 0` | Host error; abort the `validate()` call and return `1` |

**Example**

```rust
let len = unsafe { get_input_len() };
if len <= 0 { return 1; }
```

---

### `read_input(ptr: *mut u8, len: i32) -> i32`

Copies up to `len` bytes from the host input buffer into the guest-owned slice
starting at `ptr`.

```
signature : (ptr: i32, len: i32) -> i32
module    : env
```

| Parameter | Description |
|---|---|
| `ptr` | Offset into guest linear memory where bytes will be written |
| `len` | Maximum number of bytes to copy; should equal `get_input_len()` |

| Return | Meaning |
|---|---|
| `== len` | All bytes copied successfully |
| `0 … len-1` | Partial read (truncated input — treat as error) |
| `< 0` | Host error (invalid pointer or out-of-bounds write) |

The host validates that `ptr … ptr+len` is within the exported `memory` bounds
before writing.  Passing an invalid range returns `-1` without writing any bytes.

**Example**

```rust
let len = unsafe { get_input_len() };
let mut buf = vec![0u8; len as usize];
let read = unsafe { read_input(buf.as_mut_ptr(), len) };
if read != len { return 1; }
```

---

### `write_output(ptr: *const u8, len: i32) -> i32`

Copies `len` bytes from guest memory at `ptr` into the host output buffer,
replacing any previous content.  The bytes must be valid UTF-8 JSON matching the
`ValidationOutput` schema.

```
signature : (ptr: i32, len: i32) -> i32
module    : env
```

| Parameter | Description |
|---|---|
| `ptr` | Offset into guest linear memory where the JSON bytes start |
| `len` | Number of bytes to copy |

| Return | Meaning |
|---|---|
| `0` | Success |
| `< 0` | Host error (invalid pointer, out-of-bounds read, or memory copy failure) |

Call `write_output` **exactly once** per `validate()` invocation, after the
decision is final.  Multiple calls overwrite the previous output; zero calls
result in an empty buffer which the host treats as a denial.

**Example**

```rust
let json = serde_json::to_vec(&output).unwrap();
let rc = unsafe { write_output(json.as_ptr(), json.len() as i32) };
if rc != 0 { /* log error */ }
```

---

### `log_message(ptr: *const u8, len: i32)`

Copies `len` bytes of UTF-8 text from guest memory at `ptr` and emits them as a
`DEBUG`-level log line tagged `wasm_plugin` in the operator log stream.

```
signature : (ptr: i32, len: i32) -> ()
module    : env
```

| Parameter | Description |
|---|---|
| `ptr` | Offset into guest linear memory where the message starts |
| `len` | Byte length of the message (no NUL terminator required) |

`log_message` never blocks and never returns an error value.  Non-UTF-8 bytes
are replaced with the Unicode replacement character (U+FFFD).

Log lines are visible with:

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep wasm_plugin
```

**Example**

```rust
fn log(msg: &str) {
    unsafe { log_message(msg.as_ptr(), msg.len() as i32); }
}

log(&format!("checking replicas: {}", replicas));
```

---

## Input Data Structure

The host serialises `ValidationInput` to JSON (camelCase keys) and places it in
the input buffer before calling `validate()`.

### Schema

```json
{
  "operation": "CREATE",
  "object": { ... },
  "oldObject": null,
  "namespace": "default",
  "name": "my-node",
  "userInfo": {
    "username": "admin",
    "uid": "abc-123",
    "groups": ["system:masters"],
    "extra": {}
  },
  "context": {}
}
```

### Field reference

| Field | Type | Required | Description |
|---|---|---|---|
| `operation` | `string` | Yes | One of `CREATE`, `UPDATE`, `DELETE`, `CONNECT` |
| `object` | `object \| null` | Yes | The incoming resource (new state for CREATE/UPDATE) |
| `oldObject` | `object \| null` | No | Previous resource state (UPDATE only; `null` otherwise) |
| `namespace` | `string` | Yes | Kubernetes namespace of the resource |
| `name` | `string` | Yes | Name of the resource |
| `userInfo` | `UserInfo` | Yes | Kubernetes user making the request |
| `context` | `map<string,string>` | No | Operator-injected key/value pairs; empty by default |

### `UserInfo`

```json
{
  "username": "alice",
  "uid": "xyz-789",
  "groups": ["dev-team", "system:authenticated"],
  "extra": {
    "scopes": ["openid"]
  }
}
```

| Field | Type | Description |
|---|---|---|
| `username` | `string` | Kubernetes username |
| `uid` | `string \| null` | Opaque user ID |
| `groups` | `string[]` | Group memberships |
| `extra` | `map<string, string[]>` | Arbitrary extra attributes |

### `object` shape (StellarNode excerpt)

The `object` field contains the full Kubernetes resource as a JSON object.
For `StellarNode` resources the relevant fields are:

```json
{
  "apiVersion": "stellar.org/v1alpha1",
  "kind": "StellarNode",
  "metadata": {
    "name": "validator-1",
    "namespace": "default",
    "labels": { "cost-center": "eng" },
    "annotations": { "owner": "alice" }
  },
  "spec": {
    "network": "Mainnet",
    "version": "docker.io/stellar/stellar-core:v21.3.0",
    "replicas": 3,
    "resources": {
      "limits":   { "cpu": "2",    "memory": "4Gi" },
      "requests": { "cpu": "500m", "memory": "1Gi" }
    }
  }
}
```

---

## Output Data Structure

Plugins must serialise a `ValidationOutput` to JSON and pass it to `write_output`
before returning from `validate()`.

### Schema

```json
{
  "allowed": true,
  "message": "Validation passed",
  "reason": null,
  "errors": [],
  "warnings": ["Consider increasing memory limit"],
  "auditAnnotations": {
    "my-plugin/checked": "true"
  }
}
```

### Field reference

| Field | Type | Required | Description |
|---|---|---|---|
| `allowed` | `boolean` | **Yes** | `true` to permit the request, `false` to deny |
| `message` | `string \| null` | No | Human-readable summary shown in `kubectl` output |
| `reason` | `string \| null` | No | Machine-readable reason code (e.g. `"PolicyViolation"`) |
| `errors` | `ValidationError[]` | No | Structured per-field errors (omit or `[]` when allowed) |
| `warnings` | `string[]` | No | Non-blocking advisory messages |
| `auditAnnotations` | `map<string,string>` | No | Key/value pairs added to the Kubernetes audit log |

### `ValidationError`

```json
{
  "field": "spec.replicas",
  "message": "must be >= 3 for Mainnet",
  "errorType": "TooSmall",
  "invalidValue": 1
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `field` | `string` | Yes | Dot-notation JSON path to the offending field |
| `message` | `string` | Yes | Human-readable description |
| `errorType` | `string \| null` | No | One of the error type codes below |
| `invalidValue` | `any \| null` | No | The actual value that failed validation |

### `errorType` codes

| Code | Meaning |
|---|---|
| `Required` | Mandatory field is absent |
| `Invalid` | Value is malformed or logically inconsistent |
| `TooLarge` | Value exceeds the allowed maximum |
| `TooSmall` | Value is below the allowed minimum |
| `InvalidPattern` | Value does not match the required pattern |
| `NotSupported` | Value is not in the allowed set |
| `Duplicate` | Value appears more than once |
| `Immutable` | Field may not be changed after creation |
| `ConstraintViolation` | Custom constraint failed |

---

## Supporting Types

### `Operation` enum

Passed in `ValidationInput.operation` as an uppercase string.

| Value | When it fires |
|---|---|
| `CREATE` | New resource is being created |
| `UPDATE` | Existing resource is being modified |
| `DELETE` | Resource is being deleted |
| `CONNECT` | Sub-resource `connect` operation |

Plugins that do not need to inspect `DELETE` requests should return
`{"allowed":true}` early for that operation.

---

## Resource Limits & Sandboxing

### Default limits

| Limit | Default | Config key |
|---|---|---|
| Execution timeout | 1 000 ms | `timeoutMs` |
| Maximum memory | 16 MiB | `maxMemoryBytes` |
| Maximum fuel (instructions) | 1 000 000 | `maxFuel` |
| Wasm stack size | 512 KiB | (not configurable per plugin) |

### Fuel metering

Wasmtime's fuel metering maps roughly to Wasm instruction count.  Every
arithmetic, memory, and control-flow instruction costs fuel.  Host function calls
do **not** cost fuel.  When fuel reaches zero the runtime traps with:

```
Plugin exceeded instruction limit
```

Increase `maxFuel` in your plugin configuration if this occurs with legitimate
workloads.

### Epoch-based timeout

In addition to fuel, the operator increments a global epoch counter every
`timeoutMs` milliseconds.  When the epoch deadline is reached the runtime traps
with:

```
Plugin execution timeout
```

Both limits are **independent** — whichever fires first terminates execution.

### Disabled capabilities

The sandbox explicitly disables:

| Capability | Status |
|---|---|
| Filesystem access | Disabled |
| Network access | Disabled |
| System calls (WASI) | Sandboxed via WASI preview 1 (no I/O) |
| Wasm threads | Disabled |
| Reference types | Disabled |
| Wasm SIMD | Enabled (read-only) |
| Bulk memory operations | Enabled |
| Multi-value returns | Enabled |

---

## Plugin Lifecycle

```
load_plugin(bytes, metadata)
        │
        ▼
  ┌─────────────┐     SHA256 mismatch
  │  Integrity  ├────────────────────► Error (plugin rejected)
  │  Check      │
  └──────┬──────┘
         │ ok
         ▼
  ┌─────────────┐     missing 'validate' or 'memory' export
  │  Export     ├────────────────────► Error (plugin rejected)
  │  Validation │
  └──────┬──────┘
         │ ok
         ▼
  ┌─────────────┐
  │  Compile &  │   (Cranelift, cached in memory)
  │  Cache      │
  └──────┬──────┘
         │
         │  per admission request
         ▼
  ┌─────────────┐
  │  Instantiate│   fresh Store, new fuel, new epoch deadline
  │  Module     │
  └──────┬──────┘
         │
         ▼
  ┌─────────────┐
  │  Call       │
  │  validate() │
  └──────┬──────┘
         │
         ▼
  ┌─────────────┐
  │  Parse      │
  │  Output     │
  └──────┬──────┘
         │
         ▼
  AdmissionReview response
```

Each invocation of `validate()` runs in a **fresh** Wasmtime `Store`; there is
no state shared between invocations.  Compiled modules are cached to avoid
re-compilation overhead.

### Fail-open behaviour

When `failOpen: true` is set in the plugin configuration:

- A plugin timeout, out-of-fuel trap, memory limit, or panic results in an
  `allowed: true` response with a warning annotation.
- The operator logs a `WARN` line identifying the plugin and the error.
- The request is permitted to proceed.

When `failOpen: false` (default), any execution error causes the request to be
denied.

---

## REST Management API

The operator exposes a plugin management API on the webhook port (default `8443`).

### `GET /plugins`

List all currently loaded plugins.

**Response 200**

```json
{
  "plugins": [
    {
      "name": "image-registry-validator",
      "version": "1.0.0",
      "description": "Validates image registries",
      "operations": ["CREATE", "UPDATE"],
      "enabled": true
    }
  ]
}
```

---

### `POST /plugins`

Load a new plugin from a base64-encoded Wasm binary.

**Request body**

```json
{
  "metadata": {
    "name": "hello-world",
    "version": "0.1.0",
    "description": "Always allows",
    "limits": {
      "timeoutMs": 500,
      "maxMemoryBytes": 8388608,
      "maxFuel": 500000
    }
  },
  "wasmBinary": "<base64-encoded .wasm bytes>",
  "operations": ["CREATE", "UPDATE"],
  "enabled": true,
  "failOpen": false
}
```

**Response 201** — plugin loaded  
**Response 400** — invalid request body or bad Wasm binary  
**Response 409** — a plugin with this name is already loaded (unload first)

---

### `DELETE /plugins/:name`

Unload a plugin by name.

**Response 204** — unloaded  
**Response 404** — no plugin with that name

---

## Configuration Reference

### `plugins.yaml` (operator config file)

```yaml
plugins:
  - metadata:
      name: hello-world
      version: "0.1.0"
      description: "Hello World validator"
      limits:
        timeoutMs: 1000         # milliseconds
        maxMemoryBytes: 16777216 # 16 MiB
        maxFuel: 1000000
    configMapRef:
      name: hello-world-plugin
      key: plugin.wasm
      namespace: stellar-operator-system
    operations:
      - CREATE
      - UPDATE
    enabled: true
    failOpen: false
```

### Plugin source fields (exactly one must be set)

| Field | Description |
|---|---|
| `configMapRef` | Load binary from a Kubernetes ConfigMap key |
| `secretRef` | Load binary from a Kubernetes Secret key |
| `wasmBinary` | Base64-encoded binary inline in the config |
| `url` | HTTP(S) URL to download the binary from |

### `configMapRef` / `secretRef` sub-fields

| Field | Required | Description |
|---|---|---|
| `name` | Yes | ConfigMap or Secret name |
| `key` | Yes | Key containing the `.wasm` bytes |
| `namespace` | No | Defaults to the operator's namespace |

### `metadata.limits` sub-fields

| Field | Type | Default | Description |
|---|---|---|---|
| `timeoutMs` | integer | `1000` | Max wall-clock time per invocation (ms) |
| `maxMemoryBytes` | integer | `16777216` | Max linear memory (bytes) |
| `maxFuel` | integer | `1000000` | Max Wasm instruction count |

---

## Security Model

### Threat model

| Threat | Mitigation |
|---|---|
| Malicious Wasm escaping sandbox | Wasmtime's strict memory isolation; no host syscalls exposed |
| Infinite loop / CPU exhaustion | Fuel metering + epoch timeout |
| Memory exhaustion | `StoreLimits` capping linear memory |
| Supply-chain attack (tampered binary) | Optional SHA256 integrity check on load |
| Sensitive data exfiltration | No filesystem, network, or environment access |
| Plugin crash destabilising operator | Each invocation runs in a fresh `Store`; panics are caught |

### Integrity verification

Set `metadata.sha256` to the lowercase hex SHA-256 of the `.wasm` file:

```bash
sha256sum target/wasm32-unknown-unknown/release/hello_world.wasm
```

```yaml
metadata:
  name: hello-world
  sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
```

The runtime rejects the binary if the hash does not match.

---

## Performance Guidelines

| Metric | Typical value |
|---|---|
| Module compile time | < 100 ms (one-time, cached) |
| Per-invocation instantiation | < 1 ms |
| Execution time (simple policy) | < 5 ms |
| Memory footprint per plugin | < 1 MiB |
| Optimised binary size | 20 – 100 KiB |

### Tips

1. **Optimise for size** — use `opt-level = "z"` and `strip = true` in `[profile.release]`.
2. **Enable LTO** — `lto = true` reduces both size and cold-start time.
3. **Set `panic = "abort"`** — removes the `std` unwinding machinery (~30 KiB savings).
4. **Avoid heavy dependencies** — each crate adds to binary size and compilation time.
5. **Run `wasm-opt`** — pass `-Oz` to reduce binary size by a further 30–50 %:
   ```bash
   wasm-opt -Oz -o plugin.opt.wasm plugin.wasm
   ```
6. **Profile with fuel** — monitor `fuel_consumed` in the execution result to detect
   unexpectedly expensive paths before hitting the `maxFuel` limit.

---

## Versioning & Stability

| API surface | Stability |
|---|---|
| Host function signatures (`get_input_len`, `read_input`, `write_output`, `log_message`) | **Stable** — will not change without a major version bump |
| `ValidationInput` JSON field names | **Stable** |
| `ValidationOutput` JSON field names | **Stable** |
| `errorType` codes | **Stable** |
| REST `/plugins` endpoints | **Stable** |
| `Operation` enum values | **Stable** (new values may be added) |
| Internal `env` module name | **Stable** |

Deprecated host functions (if any) will be listed in the [CHANGELOG](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CHANGELOG.md)
with at least one minor-version notice period before removal.
