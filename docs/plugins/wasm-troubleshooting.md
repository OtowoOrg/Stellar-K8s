# Wasm Plugin Sandboxing — Troubleshooting Guide

This guide covers every common error you can encounter when building, loading,
or running a Wasm validation plugin in the Stellar-K8s operator.  Each entry
includes the exact error text, root cause, diagnostic steps, and a fix.

> **Related documents**
> - [Wasm Plugin API Reference](./wasm-api.md)
> - [Hello World Tutorial](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/wasm-plugins/hello-world/README.md)

---

## Table of Contents

1. [How to read operator logs](#how-to-read-operator-logs)
2. [Plugin load errors](#plugin-load-errors)
   - [Missing `validate` export](#missing-validate-export)
   - [Missing `memory` export](#missing-memory-export)
   - [SHA256 integrity mismatch](#sha256-integrity-mismatch)
   - [Invalid Wasm binary / bad magic bytes](#invalid-wasm-binary-bad-magic-bytes)
   - [ConfigMap or Secret not found](#configmap-or-secret-not-found)
   - [Plugin already loaded (409 conflict)](#plugin-already-loaded-409-conflict)
3. [Runtime / sandboxing errors](#runtime-sandboxing-errors)
   - [Out of fuel — instruction limit exceeded](#out-of-fuel-instruction-limit-exceeded)
   - [Execution timeout](#execution-timeout)
   - [Out of memory](#out-of-memory)
   - [Wasm stack overflow](#wasm-stack-overflow)
   - [Plugin panic / unreachable trap](#plugin-panic-unreachable-trap)
   - [Invalid memory access (out-of-bounds)](#invalid-memory-access-out-of-bounds)
4. [I/O and JSON errors](#io-and-json-errors)
   - [Empty output buffer — request silently denied](#empty-output-buffer-request-silently-denied)
   - [Failed to parse plugin output](#failed-to-parse-plugin-output)
   - [get_input_len returns 0 or negative](#get_input_len-returns-0-or-negative)
   - [read_input partial read](#read_input-partial-read)
   - [write_output returns -1](#write_output-returns-1)
5. [Deterministic failures](#deterministic-failures)
   - [Plugin always denies — even valid resources](#plugin-always-denies-even-valid-resources)
   - [Plugin always allows — policy not enforced](#plugin-always-allows-policy-not-enforced)
   - [Wrong field path — policy never matches](#wrong-field-path-policy-never-matches)
6. [Build and compilation errors](#build-and-compilation-errors)
   - [Unknown target `wasm32-unknown-unknown`](#unknown-target-wasm32-unknown-unknown)
   - [Linker error: symbol not found](#linker-error-symbol-not-found)
   - [Binary too large](#binary-too-large)
   - [std feature not available for wasm32-unknown-unknown](#std-feature-not-available-for-wasm32-unknown-unknown)
7. [Deployment errors](#deployment-errors)
   - [Webhook timeout — request takes too long](#webhook-timeout-request-takes-too-long)
   - [All requests denied after plugin is loaded](#all-requests-denied-after-plugin-is-loaded)
   - [Plugin not called — operation not matched](#plugin-not-called-operation-not-matched)
8. [Fail-open behaviour](#fail-open-behaviour)
9. [Diagnostic quick reference](#diagnostic-quick-reference)

---

## How to read operator logs

All Wasm-related log lines carry one of two targets:

| Log target | What it covers |
|---|---|
| `stellar_operator::webhook::runtime` | Plugin load, unload, execution lifecycle |
| `wasm_plugin` | `log_message` calls emitted by the plugin itself |

### Stream logs in real time

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator -f \
  | grep -E "wasm_plugin|webhook::runtime|PluginError"
```

### Increase log verbosity

If the default log level hides `DEBUG` lines, patch the operator deployment:

```bash
kubectl set env deployment/stellar-operator \
  -n stellar-operator-system \
  RUST_LOG="stellar_operator::webhook=debug,wasm_plugin=debug"
kubectl rollout restart deployment/stellar-operator -n stellar-operator-system
```

### Inspect a specific admission event

```bash
# Create the resource and capture the full API server error
kubectl apply -f my-node.yaml 2>&1

# Then check the operator logs for the corresponding wasm_plugin lines
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  --since=30s | grep wasm_plugin
```

---

## Plugin load errors

### Missing `validate` export

**Error message**

```
PluginError: Plugin my-plugin must export a 'validate' function
```

**Cause**

The compiled Wasm module does not have a public symbol named `validate`.  This
happens when:
- The entry point is named differently (e.g. `run`, `check`, `validate_node`)
- The `#[no_mangle]` attribute is missing
- The function is not `pub`
- The crate type is `lib` (produces `rlib`) instead of `cdylib`

**Fix**

Ensure your entry point is declared exactly as:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 { ... }
```

And your `Cargo.toml` specifies:

```toml
[lib]
crate-type = ["cdylib"]
```

Verify the export is present before deploying:

```bash
wasm-objdump -x target/wasm32-unknown-unknown/release/my_plugin.wasm \
  | grep '"validate"'
```

---

### Missing `memory` export

**Error message**

```
PluginError: Plugin my-plugin must export 'memory'
```

**Cause**

The Wasm module does not export its linear memory.  With
`wasm32-unknown-unknown` and `crate-type = ["cdylib"]` this is exported
automatically.  It can be suppressed if you use a custom linker script or a
language other than Rust that requires an explicit memory export.

**Fix**

For Rust, ensure you are using:

```toml
[lib]
crate-type = ["cdylib"]   # NOT ["staticlib"] or ["lib"]
```

For non-Rust languages, add an explicit export in your Wasm text or use your
toolchain's `--export-memory` flag.  For example, with `wasm-ld`:

```
wasm-ld --export=memory ...
```

---

### SHA256 integrity mismatch

**Error message**

```
PluginError: Plugin my-plugin integrity check failed:
  expected abc123..., got def456...
```

**Cause**

The `metadata.sha256` field in `plugins.yaml` does not match the actual binary
loaded from the ConfigMap, Secret, or URL.  Common causes:

- The binary was recompiled after the hash was recorded
- The ConfigMap was updated but the hash was not
- Base64 encoding introduced padding differences
- The wrong key was referenced in the ConfigMap

**Fix**

Recompute the hash from the **exact file** you are deploying:

```bash
sha256sum target/wasm32-unknown-unknown/release/my_plugin.wasm
# e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  my_plugin.wasm
```

Update `plugins.yaml`:

```yaml
metadata:
  sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
```

If you want to skip integrity verification during development, omit the
`sha256` field entirely.

---

### Invalid Wasm binary / bad magic bytes

**Error message**

```
PluginError: Failed to compile plugin my-plugin:
  expected magic number but got 0x89504e47 (PNG header)
```

or

```
PluginError: Failed to compile plugin my-plugin:
  invalid magic number at offset 0
```

**Cause**

The binary stored in the ConfigMap is not a valid Wasm file.  Common causes:

- The wrong file was passed to `kubectl create configmap`
- The file was base64-encoded a second time before being stored
- The file was corrupted during transfer

**Fix**

```bash
# Inspect the first 8 bytes of the file
xxd target/wasm32-unknown-unknown/release/my_plugin.wasm | head -1
# A valid Wasm file starts with: 00 61 73 6d 01 00 00 00  (".asm" + version)

# Extract the binary from the ConfigMap and check it
kubectl get configmap my-plugin -n stellar-operator-system \
  -o jsonpath='{.binaryData.plugin\.wasm}' | base64 -d | xxd | head -1
```

If the bytes look wrong, recreate the ConfigMap:

```bash
kubectl delete configmap my-plugin -n stellar-operator-system
kubectl create configmap my-plugin \
  --from-file=plugin.wasm=target/wasm32-unknown-unknown/release/my_plugin.wasm \
  -n stellar-operator-system
```

---

### ConfigMap or Secret not found

**Error message** (in operator logs)

```
ERROR stellar_operator::webhook: Failed to load plugin my-plugin:
  ConfigMap "my-plugin" not found in namespace "stellar-operator-system"
```

**Cause**

The `configMapRef.name` or `secretRef.name` in `plugins.yaml` does not match
any resource in the specified namespace.

**Fix**

```bash
# List ConfigMaps in the operator namespace
kubectl get configmap -n stellar-operator-system

# Check the exact name and key
kubectl get configmap my-plugin -n stellar-operator-system -o yaml \
  | grep -E "name:|  plugin"
```

Ensure `configMapRef.namespace` matches the namespace where the ConfigMap lives.
If it is in a different namespace:

```yaml
configMapRef:
  name: my-plugin
  key: plugin.wasm
  namespace: my-other-namespace   # explicit namespace
```

---

### Plugin already loaded (409 conflict)

**Error message** (REST API)

```
HTTP 409 Conflict
{"error": "plugin 'my-plugin' is already loaded; unload it first"}
```

**Fix**

Unload the existing plugin before loading the new version:

```bash
curl -X DELETE https://webhook-service:8443/plugins/my-plugin

# Then reload
curl -X POST https://webhook-service:8443/plugins \
  -H "Content-Type: application/json" \
  -d @plugin-manifest.json
```

Or via a rolling update: update the ConfigMap, then trigger a rollout:

```bash
kubectl create configmap my-plugin \
  --from-file=plugin.wasm=my_plugin.wasm \
  -n stellar-operator-system \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl rollout restart deployment/stellar-operator -n stellar-operator-system
```

---

## Runtime / sandboxing errors

### Out of fuel — instruction limit exceeded

**Error message**

```
PluginError: Plugin exceeded instruction limit
```

**Cause**

The plugin consumed more Wasmtime fuel than `maxFuel` allows before returning.
Fuel maps roughly to Wasm instruction count (~1 fuel per instruction).  Typical
causes:

- Deserialising a large JSON object with a deeply nested structure
- An accidental infinite or near-infinite loop
- Heavy string formatting in log messages

**Diagnosis**

Add `log_message` calls to identify which section is expensive:

```rust
log("starting registry check");
// ... potentially expensive code ...
log("registry check done");
```

Watch the logs to see which section never produces a "done" message.

**Fix**

Option A — increase `maxFuel` in `plugins.yaml`:

```yaml
limits:
  maxFuel: 5000000   # raise from default 1_000_000
```

Option B — optimise the hot path (avoid repeated JSON traversal, cache
intermediate results):

```rust
// Instead of traversing the tree multiple times:
let spec = object.get("spec");
let network  = spec.and_then(|s| s.get("network")).and_then(|v| v.as_str()).unwrap_or("");
let replicas = spec.and_then(|s| s.get("replicas")).and_then(|v| v.as_i64()).unwrap_or(0);
```

Option C — if the loop is intentional and bounded, add a counter guard:

```rust
for (i, item) in items.iter().enumerate() {
    if i > 1000 { break; } // safety bound
    // ...
}
```

---

### Execution timeout

**Error message**

```
PluginError: Plugin execution timeout
```

**Cause**

The plugin did not return within `timeoutMs` milliseconds (default: 1 000 ms).
Wasmtime's epoch-based interruption fires independently of fuel.  Causes:

- Genuinely slow policy logic (complex regex, large data structures)
- Blocking I/O attempt (will always time out — no I/O is available)
- Tight loop that consumes fuel slowly enough to avoid the fuel trap first

**Fix**

Option A — increase `timeoutMs`:

```yaml
limits:
  timeoutMs: 3000   # 3 seconds
```

Option B — profile with fuel to find the slow section (fuel consumed is reported
in execution results).

Option C — ensure your plugin is not attempting any form of blocking wait.
Network calls, filesystem reads, and `std::thread::sleep` will trap immediately
or stall until the epoch fires.

---

### Out of memory

**Error message**

```
PluginError: Plugin execution failed: memory allocation failed
```

or

```
PluginError: Plugin execution failed: wasm `unreachable` instruction executed
  (OOM abort inside plugin)
```

**Cause**

The plugin requested more linear memory than `maxMemoryBytes` allows (default:
16 MiB), or the Wasm stack overflowed during deep recursion.

Common causes:

- Deserialising very large JSON blobs (e.g. a StellarNode with many containers)
- Accumulating all validation errors into a large `Vec` before returning
- Deep recursive calls (use iterative algorithms instead)

**Diagnosis**

```bash
# Check how much memory is actually being used
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep "memory_used"
```

**Fix**

Option A — increase `maxMemoryBytes`:

```yaml
limits:
  maxMemoryBytes: 33554432   # 32 MiB
```

Option B — reduce allocations in the plugin.  Use string slices instead of
`String`, avoid collecting iterators unless necessary, and reuse buffers.

Option C — for large inputs, process fields lazily with `serde_json::from_slice`
streaming rather than deserialising the entire document.

---

### Wasm stack overflow

**Error message**

```
PluginError: Plugin execution failed: call stack exhausted
```

**Cause**

The Wasm call stack depth exceeded 512 KiB (the fixed Wasmtime stack limit).
This is separate from heap memory.  Common causes:

- Recursive functions without a depth limit
- Mutual recursion that grows unboundedly
- Very deep serde deserialization of nested JSON (> ~50 levels)

**Fix**

Replace recursive logic with an iterative equivalent:

```rust
// Recursive (may overflow for deep trees)
fn walk(node: &Value) -> bool {
    node.as_array()
        .map(|arr| arr.iter().all(walk))
        .unwrap_or(true)
}

// Iterative (bounded stack depth)
fn walk(root: &Value) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if let Some(arr) = node.as_array() {
            stack.extend(arr.iter());
        }
    }
    true
}
```

---

### Plugin panic / unreachable trap

**Error message**

```
PluginError: Plugin execution failed:
  wasm `unreachable` instruction executed
```

**Cause**

The plugin executed a Wasm `unreachable` instruction.  With `panic = "abort"`
(recommended), a Rust `panic!`, failed `unwrap()`, index-out-of-bounds, or
integer overflow compiles to `unreachable`.

**Diagnosis**

Reproduce the failure in a unit test on the native target:

```bash
cargo test -- --nocapture 2>&1 | grep "panicked"
```

**Fix**

Replace `unwrap()` / `expect()` with explicit error handling:

```rust
// Bad — panics if "spec" is missing
let network = object["spec"]["network"].as_str().unwrap();

// Good — returns a safe default
let network = object
    .pointer("/spec/network")
    .and_then(|v| v.as_str())
    .unwrap_or("");
```

If integer overflow is the concern, use `checked_add` / `saturating_add`:

```rust
// Risky in release mode with overflow checks disabled in Wasm
let total = a + b;

// Safe
let total = a.saturating_add(b);
```

---

### Invalid memory access (out-of-bounds)

**Error message**

```
PluginError: Plugin execution failed:
  memory out of bounds: data segment does not fit
```

or

```
PluginError: Plugin execution failed:
  out of bounds memory access
```

**Cause**

The plugin passed an invalid pointer to a host function, or the Wasm linker
placed data segments beyond the declared memory size.  With Rust and
`wasm32-unknown-unknown` this is uncommon but can be triggered by:

- Manually constructed raw pointers with incorrect offsets
- Calling host functions with a `len` larger than the buffer

**Fix**

Always derive pointers from actual allocated slices:

```rust
let mut buf = vec![0u8; len as usize];
let read = unsafe { read_input(buf.as_mut_ptr(), buf.len() as i32) };
```

Never pass a hard-coded or arithmetic-derived pointer offset — always use
`.as_ptr()` / `.as_mut_ptr()` on a live slice.

---

## I/O and JSON errors

### Empty output buffer — request silently denied

**Symptom**

`kubectl apply` returns a denial with the message:

```
admission webhook denied the request: Plugin returned error code: 0
```

or the request is denied with no message even though your policy should allow it.

**Cause**

`write_output` was never called (or returned an error that was silently ignored),
leaving the output buffer empty.  The runtime falls back to interpreting the
return code: code `0` with no output → `allowed`, but some runtime versions
treat empty output as a denial.

**Fix**

Always call `write_output` exactly once, unconditionally, before returning:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    let output = match run_policy() {
        Ok(o)  => o,
        Err(e) => ValidationOutput::denied(format!("plugin error: {e}")),
    };
    write_output_struct(&output);      // ALWAYS write before returning
    if output.allowed { 0 } else { 1 }
}
```

---

### Failed to parse plugin output

**Error message**

```
PluginError: Failed to parse plugin output:
  missing field `allowed` at line 1 column 2
```

**Cause**

The JSON written to the output buffer is either malformed or missing the
required `allowed` field.

**Diagnosis**

Add a `log_message` call just before `write_output` to print the JSON:

```rust
let json = serde_json::to_string(&output).unwrap();
log(&format!("output JSON: {json}"));
unsafe { write_output(json.as_ptr(), json.len() as i32); }
```

Then read it from the operator logs.

**Fix**

Ensure `allowed` is always present.  With the recommended `ValidationOutput`
struct and `#[derive(Serialize)]`, this is automatic as long as the struct is
fully initialised.  If you are building the JSON manually, double-check the
field name is lowercase `allowed` (not `Allowed` or `allow`).

---

### `get_input_len` returns 0 or negative

**Symptom**

Plugin logs show:

```
hello-world: failed to read input: get_input_len returned 0
```

**Cause**

The host input buffer is empty, which should not happen in normal operation.
Possible causes:

- The plugin was invoked outside of an admission request (e.g. during a health
  check or diagnostic call)
- A bug in a custom test harness that calls `validate()` without pre-loading input

**Fix**

Return a graceful denial rather than crashing:

```rust
let len = unsafe { get_input_len() };
if len <= 0 {
    write_denied("empty input buffer", "PluginError");
    return 1;
}
```

---

### `read_input` partial read

**Symptom**

Plugin logs show:

```
hello-world: failed to read input: read_input: expected 1024 bytes, got 512
```

**Cause**

The buffer passed to `read_input` was smaller than the value returned by
`get_input_len`.  This can happen if:

- `get_input_len` is called, then new input arrives before `read_input` (not
  possible in the current runtime, but defensive coding is good practice)
- A manual pointer calculation underestimates the buffer size

**Fix**

Always allocate based on the return value of `get_input_len`:

```rust
let len = unsafe { get_input_len() };
let mut buf = vec![0u8; len as usize];   // exactly `len` bytes
let read = unsafe { read_input(buf.as_mut_ptr(), len) };
if read != len { /* error */ }
```

---

### `write_output` returns -1

**Symptom**

Plugin logs show:

```
hello-world: write_output returned -1
```

**Cause**

The pointer and length passed to `write_output` pointed outside the module's
linear memory, or `len` was negative.

**Fix**

```rust
fn write_output_struct(output: &ValidationOutput) {
    match serde_json::to_vec(output) {
        Err(e) => { log(&format!("serialisation error: {e}")); return; }
        Ok(json) => {
            let rc = unsafe { write_output(json.as_ptr(), json.len() as i32) };
            if rc != 0 {
                log(&format!("write_output failed with code {rc}"));
            }
        }
    }
}
```

The length will be negative only if `json.len() > i32::MAX` (~2 GiB), which
cannot happen in a 16 MiB sandbox.  Always use `json.len() as i32` — the cast
is safe within the memory limits.

---

## Deterministic failures

### Plugin always denies — even valid resources

**Symptom**

Every admission request is denied, including ones that should clearly pass.

**Diagnosis checklist**

1. Check `write_output` is being called — add a log line immediately before it.
2. Confirm `allowed: true` is being set in the output struct for passing cases.
3. Check for an early-return path that skips `write_output` and returns `1`.
4. Run unit tests: `cargo test -- --nocapture` to see all logic branches.
5. Check the `failOpen` setting — if `false` and the plugin errors, it denies.

**Minimal diagnostic plugin**

Temporarily replace your plugin with a pass-through to confirm the runtime is
working:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    let output = br#"{"allowed":true,"message":"diagnostic pass-through"}"#;
    unsafe { write_output(output.as_ptr(), output.len() as i32) };
    0
}
```

If this still denies, the problem is in the runtime or configuration, not your
plugin logic.

---

### Plugin always allows — policy not enforced

**Symptom**

Resources that violate your policy are being admitted without error.

**Diagnosis checklist**

1. Confirm the plugin is in the `operations` list for the operation you are
   testing (e.g. you may have `["CREATE"]` but are testing `UPDATE`).
2. Confirm `enabled: true` in `plugins.yaml`.
3. Check that `write_output` is called — an empty output buffer with return
   code `0` may be interpreted as allowed.
4. Add `log_message` calls to trace which branch is taken.
5. Run unit tests with the failing input to reproduce locally.

---

### Wrong field path — policy never matches

**Symptom**

A rule that checks `spec.network` never triggers, even when the field is clearly
set.

**Cause**

`serde_json::Value::pointer` uses `/`-separated paths with a leading `/`:

```rust
// Wrong — returns None always
object.get("spec.network")

// Wrong — also returns None
object.pointer("spec/network")

// Correct
object.pointer("/spec/network")
```

**Fix**

Always use a leading `/` with `pointer`:

```rust
let network = object
    .pointer("/spec/network")   // note the leading slash
    .and_then(|v| v.as_str())
    .unwrap_or("");
```

Alternatively, chain `.get()` calls:

```rust
let network = object
    .get("spec")
    .and_then(|s| s.get("network"))
    .and_then(|v| v.as_str())
    .unwrap_or("");
```

---

## Build and compilation errors

### Unknown target `wasm32-unknown-unknown`

**Error message**

```
error[E0463]: can't find crate for `std`
note: the `wasm32-unknown-unknown` target may not be installed
```

**Fix**

```bash
rustup target add wasm32-unknown-unknown
```

---

### Linker error: symbol not found

**Error message**

```
error: linking with `rust-lld` failed: exit status: 1
  = note: rust-lld: error: undefined symbol: __some_libc_function
```

**Cause**

You (or a dependency) are calling a function that does not exist in
`wasm32-unknown-unknown` (no OS, no libc).

**Diagnosis**

Find which crate pulls in the problematic symbol:

```bash
cargo tree --target wasm32-unknown-unknown | grep <crate-name>
```

**Fix**

- Replace OS-dependent crates with Wasm-compatible alternatives.
- For `std::time::SystemTime` → use a counter or omit timestamps.
- For file I/O → remove it; plugins have no filesystem access.
- For `rand` → use `rand`'s `wasm-bindgen` feature or avoid randomness.
- Add `default-features = false` to dependencies that gate std features:

```toml
[dependencies]
some-crate = { version = "1.0", default-features = false }
```

---

### Binary too large

**Symptom**

The `.wasm` file is several MiB, Kubernetes rejects the ConfigMap, or the plugin
takes hundreds of milliseconds to load.

**Fix**

Apply all of the following in `Cargo.toml`:

```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Then run `wasm-opt`:

```bash
wasm-opt -Oz -o plugin.opt.wasm plugin.wasm
```

If the binary is still large, audit your dependencies:

```bash
cargo bloat --target wasm32-unknown-unknown --release --crates
```

The `cargo-bloat` tool identifies which crates contribute most to binary size.

---

### `std` feature not available for `wasm32-unknown-unknown`

**Error message**

```
error[E0433]: failed to resolve: use of undeclared crate or module `std`
```

or a dependency fails to compile citing missing platform support.

**Cause**

`wasm32-unknown-unknown` is a `no_std` target — the Rust standard library is
not available in the traditional sense.  However, Rust's `std` *is* available
for this target as a pre-compiled sysroot, so this error usually means a
dependency is checking `#[cfg(target_os = "...")]` and failing.

**Fix**

Use `no_std`-compatible crates, or add the `wasm` feature flag if the crate
provides one.  Check the crate's documentation for Wasm support.

For `serde` and `serde_json` (used in the hello-world plugin), full `std`
support on `wasm32-unknown-unknown` is available and works out of the box.

---

## Deployment errors

### Webhook timeout — request takes too long

**Error message** (in `kubectl apply`)

```
Error from server: error when creating "my-node.yaml":
  Internal error occurred: failed calling webhook
  "validate.stellarnode.stellar.org": failed to call webhook:
  Post "https://...": context deadline exceeded
```

**Cause**

The Kubernetes API server's webhook timeout (configured in
`ValidatingWebhookConfiguration`) expired before the operator responded.  The
operator plugin timeout is separate from this.

**Fix**

Increase the webhook timeout in `ValidatingWebhookConfiguration`:

```yaml
webhooks:
  - name: validate.stellarnode.stellar.org
    timeoutSeconds: 15   # raise from default 10
    ...
```

Also ensure the plugin's `timeoutMs` is well under the webhook timeout:

```yaml
limits:
  timeoutMs: 5000   # 5 s, safely under the 15 s webhook timeout
```

---

### All requests denied after plugin is loaded

**Symptom**

Every `kubectl apply` for a StellarNode fails immediately after loading a new
plugin, even for resources that worked before.

**Diagnosis**

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  --since=5m | grep -E "ERROR|WARN|denied"
```

**Common causes and fixes**

| Cause | Fix |
|---|---|
| Plugin crashes on startup (bad wasm binary) | Check load logs; redeploy the binary |
| `failOpen: false` and plugin always errors | Set `failOpen: true` temporarily to unblock; fix the plugin |
| ConfigMap was updated mid-request | Wait for the operator to reload; check rollout status |
| Plugin has a logic bug that always denies | Run unit tests; check with the diagnostic pass-through above |

---

### Plugin not called — operation not matched

**Symptom**

The plugin does not log anything and the policy is not enforced, but other
plugins work fine.

**Cause**

The `operations` list in `plugins.yaml` does not include the operation you are
testing.

**Fix**

```yaml
operations:
  - CREATE
  - UPDATE   # add if you want to validate updates
  - DELETE   # add if you want to validate deletes
```

Also confirm `enabled: true`:

```yaml
enabled: true
```

---

## Fail-open behaviour

When `failOpen: true` is set, any plugin error (timeout, OOM, panic, bad JSON)
results in the request being **allowed** with a warning annotation rather than
denied.  This is intentional for non-critical checks.

**Verifying fail-open is active**

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep "fail-open"
# WARN stellar_operator::webhook::runtime:
#   Plugin my-plugin failed (fail-open): Plugin execution timeout
```

**When to use `failOpen: true`**

- Advisory checks that should not block deployments
- New plugins being rolled out to production for the first time
- Non-security-critical policies (e.g. label suggestions)

**When to use `failOpen: false` (default)**

- Security policies (image registry enforcement, network restrictions)
- Compliance checks that must be enforced
- Any policy where a denial on error is the safer default

---

## Diagnostic quick reference

| Problem | First command to run |
|---|---|
| Plugin not loading | `kubectl logs -n stellar-operator-system deploy/stellar-operator \| grep "Plugin"` |
| Plugin errors at runtime | `kubectl logs … \| grep -E "PluginError\|wasm_plugin"` |
| All requests denied | `kubectl logs … --since=5m \| grep "denied"` |
| Need to see DEBUG output | `kubectl set env deploy/stellar-operator RUST_LOG="stellar_operator::webhook=debug"` |
| Check what plugins are loaded | `kubectl exec <pod> -- curl -sk https://localhost:8443/plugins` |
| Verify Wasm exports | `wasm-objdump -x plugin.wasm \| grep -E "Export\|Import"` |
| Binary integrity | `sha256sum plugin.wasm` (compare to `metadata.sha256`) |
| Inspect ConfigMap binary | `kubectl get cm my-plugin -o jsonpath='{.binaryData.plugin\.wasm}' \| base64 -d \| xxd \| head` |
| Run unit tests locally | `cargo test -- --nocapture` |
| Profile binary size | `cargo bloat --target wasm32-unknown-unknown --release --crates` |
