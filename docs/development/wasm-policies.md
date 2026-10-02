# WASM Validation Policy Authoring Guide for Enterprise Node Operators

This guide teaches enterprise operators how to write, compile, package, and deploy
custom WebAssembly (Wasm) validation policies for the Stellar Kubernetes Operator.
By following it end-to-end you will:

- Understand the host ABI that passes admission data into the sandboxed Wasm module
- Write a Rust plugin that enforces an image-registry allow-list
- Compile the plugin to `wasm32-unknown-unknown`
- Package it into a Kubernetes ConfigMap
- Configure fail-open vs fail-closed behaviour
- Deploy to a live cluster and verify it intercepts `StellarNode` creation events

**Time to complete:** ~45 minutes  
**Audience:** Enterprise platform engineers or DevOps teams managing Stellar-K8s clusters  
**Prerequisite knowledge:** Rust basics, `kubectl`, Kubernetes admission webhooks

> **Related documents**
> - [Wasm Plugin API Reference](../plugins/wasm-api.md) — complete ABI specification
> - [Hello World Tutorial](../../examples/wasm-plugins/hello-world/README.md) — beginner first plugin
> - [Wasm Troubleshooting](../plugins/wasm-troubleshooting.md) — common runtime errors
> - [wasm-webhook.md](../wasm-webhook.md) — system overview

---

## Table of Contents

1. [Background: How the Wasm Admission Webhook Works](#1-background-how-the-wasm-admission-webhook-works)
2. [Prerequisites](#2-prerequisites)
3. [The Host ABI in Detail](#3-the-host-abi-in-detail)
   - 3.1 [Host functions](#31-host-functions)
   - 3.2 [Input JSON schema](#32-input-json-schema)
   - 3.3 [Output JSON schema](#33-output-json-schema)
   - 3.4 [Plugin contract (required exports)](#34-plugin-contract-required-exports)
4. [Writing an Enterprise Policy Plugin](#4-writing-an-enterprise-policy-plugin)
   - 4.1 [Create the Rust crate](#41-create-the-rust-crate)
   - 4.2 [Cargo.toml](#42-cargotoml)
   - 4.3 [Declare host function imports](#43-declare-host-function-imports)
   - 4.4 [Define the data types](#44-define-the-data-types)
   - 4.5 [Implement the `validate()` entry point](#45-implement-the-validate-entry-point)
   - 4.6 [Implement the registry-allow-list policy](#46-implement-the-registry-allow-list-policy)
   - 4.7 [I/O helper functions](#47-io-helper-functions)
5. [Run Unit Tests Locally](#5-run-unit-tests-locally)
6. [Compile to WebAssembly](#6-compile-to-webassembly)
   - 6.1 [Optimise with wasm-opt (recommended)](#61-optimise-with-wasm-opt-recommended)
   - 6.2 [Verify binary size](#62-verify-binary-size)
7. [Fail-Open vs Fail-Closed Behaviour](#7-fail-open-vs-fail-closed-behaviour)
   - 7.1 [When to choose fail-open](#71-when-to-choose-fail-open)
   - 7.2 [When to choose fail-closed](#72-when-to-choose-fail-closed)
   - 7.3 [Configuring the mode](#73-configuring-the-mode)
8. [Package the Plugin into a ConfigMap](#8-package-the-plugin-into-a-configmap)
9. [Deploy and Configure the Operator](#9-deploy-and-configure-the-operator)
   - 9.1 [Update plugins.yaml](#91-update-pluginsyaml)
   - 9.2 [Apply the ValidatingWebhookConfiguration](#92-apply-the-validatingwebhookconfiguration)
   - 9.3 [Verify the plugin loaded](#93-verify-the-plugin-loaded)
10. [Validate End-to-End](#10-validate-end-to-end)
    - 10.1 [Test: request that should be denied](#101-test-request-that-should-be-denied)
    - 10.2 [Test: request that should be allowed](#102-test-request-that-should-be-allowed)
11. [Enterprise Hardening Checklist](#11-enterprise-hardening-checklist)
12. [Clean Up](#12-clean-up)

---

## 1. Background: How the Wasm Admission Webhook Works

When a `StellarNode` resource is created or updated, the Kubernetes API server
sends an `AdmissionReview` request to the Stellar-K8s webhook service.  The
webhook loads your compiled Wasm binary from a ConfigMap (or Secret), runs it
inside a Wasmtime sandbox, and forwards the allow/deny decision back to the API
server.

```
Kubernetes API Server
        │
        │  AdmissionReview (CREATE / UPDATE)
        ▼
┌─────────────────────────────────────────┐
│  Stellar-K8s Admission Webhook          │
│                                         │
│  ┌─────────────────────────────────┐    │
│  │  Wasmtime Sandbox               │    │
│  │                                 │    │
│  │   get_input_len()               │    │
│  │   read_input(ptr, len)   ──────►│    │
│  │                                 │    │
│  │   [your plugin logic]           │    │
│  │                                 │    │
│  │   write_output(ptr, len) ◄──────│    │
│  │   log_message(ptr, len)         │    │
│  └─────────────────────────────────┘    │
│                                         │
│  allowed: true / false                  │
└─────────────────────────────────────────┘
        │
        ▼
   API Server decision
```

Key properties of the sandbox:

- **No filesystem access** — plugins cannot read or write files
- **No network access** — plugins cannot make outbound connections
- **Memory limits** — configurable maximum linear memory (default 16 MiB)
- **Fuel metering** — execution is capped at a configurable instruction count
- **Epoch timeout** — wall-clock timeout independent of fuel
- **Stateless** — each admission request gets a fresh Wasmtime `Store`; no
  state persists between invocations

---

## 2. Prerequisites

Install the following tools before starting:

```bash
# Rust toolchain (1.92+ required — see README.md Prerequisites)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# wasm32-unknown-unknown compilation target
rustup target add wasm32-unknown-unknown

# wasm-opt for binary optimisation (optional but recommended)
cargo install wasm-opt

# kubectl connected to your cluster
kubectl cluster-info
```

Verify your Rust version:

```bash
rustc --version
# rustc 1.92.0 (or newer)
```

---

## 3. The Host ABI in Detail

All data exchange between the operator runtime and your plugin happens through
four C-ABI host functions imported from the `env` module.

### 3.1 Host functions

```rust
extern "C" {
    /// Returns the byte length of the JSON input in the host buffer.
    /// Returns < 0 on error.
    fn get_input_len() -> i32;

    /// Copies up to `len` bytes from the host input buffer into guest
    /// memory at `ptr`.  Returns the number of bytes copied, or < 0 on error.
    fn read_input(ptr: *mut u8, len: i32) -> i32;

    /// Copies `len` bytes from guest memory at `ptr` into the host output
    /// buffer.  Returns 0 on success, < 0 on error.  Call exactly once per
    /// validate() invocation.
    fn write_output(ptr: *const u8, len: i32) -> i32;

    /// Emits a DEBUG log line tagged `wasm_plugin` in the operator logs.
    /// Non-blocking; never returns an error.
    fn log_message(ptr: *const u8, len: i32);
}
```

> See [docs/plugins/wasm-api.md](../plugins/wasm-api.md#host-functions) for
> the complete specification, including error codes and memory-safety rules.

### 3.2 Input JSON schema

Before calling `validate()` the runtime places a serialised `ValidationInput`
object in the input buffer (camelCase keys):

```json
{
  "operation": "CREATE",
  "object": {
    "apiVersion": "stellar.org/v1alpha1",
    "kind": "StellarNode",
    "metadata": {
      "name": "prod-validator-1",
      "namespace": "stellar",
      "labels": { "cost-center": "platform" },
      "annotations": { "owner": "ops-team" }
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
  },
  "oldObject": null,
  "namespace": "stellar",
  "name": "prod-validator-1",
  "userInfo": {
    "username": "alice",
    "uid": "abc-123",
    "groups": ["system:masters"],
    "extra": {}
  },
  "context": {}
}
```

| Field | Type | Description |
|---|---|---|
| `operation` | string | `CREATE`, `UPDATE`, `DELETE`, or `CONNECT` |
| `object` | object\|null | Incoming resource (new state for CREATE/UPDATE) |
| `oldObject` | object\|null | Previous state for UPDATE; `null` otherwise |
| `namespace` | string | Kubernetes namespace of the resource |
| `name` | string | Name of the resource |
| `userInfo` | object | Kubernetes identity making the request |
| `context` | map | Operator-injected key/value pairs |

### 3.3 Output JSON schema

Your plugin must call `write_output` with a `ValidationOutput` JSON object:

```json
{
  "allowed": false,
  "message": "image registry not in the approved list",
  "reason": "PolicyViolation",
  "errors": [
    {
      "field": "spec.version",
      "message": "registry 'quay.io' is not approved; use docker.io/stellar/ or ghcr.io/myorg/",
      "errorType": "NotSupported",
      "invalidValue": "quay.io/someone/stellar-core:v21.3.0"
    }
  ],
  "warnings": [],
  "auditAnnotations": {
    "registry-enforcer.stellar.org/checked": "true"
  }
}
```

| Field | Required | Description |
|---|---|---|
| `allowed` | **Yes** | `true` to permit the request |
| `message` | No | Human-readable summary shown by `kubectl` |
| `reason` | No | Machine-readable reason code |
| `errors` | No | Structured per-field errors |
| `warnings` | No | Non-blocking advisory messages |
| `auditAnnotations` | No | Written to the Kubernetes audit log |

### 3.4 Plugin contract (required exports)

The operator rejects a plugin at load time if either export is missing:

| Export | Type | Description |
|---|---|---|
| `validate` | `() -> i32` | Main entry point — called once per admission request |
| `memory` | memory | Linear memory shared with the host |

`validate()` return codes:

| Code | Meaning |
|---|---|
| `0` | Validation succeeded (host reads `allowed` from the JSON output) |
| `1` | Validation failed (host reads `allowed` from the JSON output) |
| other | Internal plugin error — request is denied |

---

## 4. Writing an Enterprise Policy Plugin

We will build `registry-enforcer`: a plugin that allows `StellarNode` admission
only when `spec.version` references an image from your organisation's approved
registries.

The complete source is at
[examples/wasm-plugins/registry-enforcer/](../../examples/wasm-plugins/registry-enforcer/).

### 4.1 Create the Rust crate

```bash
mkdir -p my-registry-enforcer/src
cd my-registry-enforcer
```

Or clone the finished example:

```bash
cp -r examples/wasm-plugins/registry-enforcer my-registry-enforcer
cd my-registry-enforcer
```

### 4.2 Cargo.toml

```toml
[package]
name = "registry-enforcer"
version = "1.0.0"
edition = "2021"
description = "Enterprise image-registry allow-list plugin for Stellar-K8s"
license = "Apache-2.0"

# cdylib produces the .wasm binary understood by the runtime
[lib]
crate-type = ["cdylib"]

[dependencies]
serde      = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Release profile tuned for minimal Wasm binary size
[profile.release]
opt-level     = "z"   # optimise for size
lto           = true  # link-time optimisation
codegen-units = 1     # maximum optimisation
panic         = "abort" # removes unwinding (~30 KiB savings)
strip         = true  # strip debug symbols
```

### 4.3 Declare host function imports

Every plugin starts with the same four host-function declarations:

```rust
extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}
```

These are provided by the Wasmtime runtime inside the `env` module.  Your Rust
code does not need to implement them — just declare and call them.

### 4.4 Define the data types

Mirror the JSON schemas with serde-annotated structs:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationInput {
    operation: String,
    object: Option<serde_json::Value>,
    #[allow(dead_code)]
    namespace: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    user_info: UserInfo,
    #[serde(default)]
    context: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UserInfo {
    username: String,
    uid: Option<String>,
    groups: Vec<String>,
    extra: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ValidationOutput {
    allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ValidationError>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    audit_annotations: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidationError {
    field: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    invalid_value: Option<String>,
}
```

### 4.5 Implement the `validate()` entry point

`validate()` is the only symbol the runtime calls.  Follow this pattern — it is
the same in every plugin:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // Step 1 — read the input
    let input = match read_validation_input() {
        Ok(v) => v,
        Err(msg) => {
            log(&format!("registry-enforcer: failed to read input: {msg}"));
            write_denied(&format!("plugin error: {msg}"), "PluginError");
            return 1;
        }
    };

    log(&format!(
        "registry-enforcer: {} on {}/{}",
        input.operation, input.namespace, input.name
    ));

    // Step 2 — pass-through for non-mutating operations
    if input.operation != "CREATE" && input.operation != "UPDATE" {
        write_allowed("non-mutating operation; skipped");
        return 0;
    }

    // Step 3 — require an object
    let object = match &input.object {
        Some(o) => o,
        None => {
            write_denied("no object in request", "InvalidInput");
            return 1;
        }
    };

    // Step 4 — apply the registry policy
    let output = enforce_registry_policy(object);
    let rc = if output.allowed { 0 } else { 1 };
    write_output_struct(&output);
    rc
}
```

### 4.6 Implement the registry-allow-list policy

Edit `APPROVED_REGISTRIES` to match your organisation:

```rust
/// Prefixes that are approved for use as StellarNode image sources.
/// A `spec.version` value must start with one of these strings.
const APPROVED_REGISTRIES: &[&str] = &[
    "docker.io/stellar/",   // official Stellar images
    "ghcr.io/myorg/",       // your organisation's internal registry
    // add further entries as needed, e.g.:
    // "artifactory.corp.example.com/stellar/",
];

fn enforce_registry_policy(object: &serde_json::Value) -> ValidationOutput {
    let mut errors = Vec::new();
    let mut audit = BTreeMap::new();

    let version = object
        .pointer("/spec/version")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    audit.insert(
        "registry-enforcer.stellar.org/checked".into(),
        "true".into(),
    );

    if version.is_empty() {
        errors.push(ValidationError {
            field: "spec.version".into(),
            message: "spec.version is required".into(),
            error_type: Some("Required".into()),
            invalid_value: None,
        });
    } else {
        let approved = APPROVED_REGISTRIES
            .iter()
            .any(|prefix| version.starts_with(prefix));

        audit.insert(
            "registry-enforcer.stellar.org/version".into(),
            version.to_string(),
        );

        if !approved {
            let approved_list = APPROVED_REGISTRIES.join(", ");
            errors.push(ValidationError {
                field: "spec.version".into(),
                message: format!(
                    "image registry is not in the approved list; \
                     approved prefixes: {approved_list}"
                ),
                error_type: Some("NotSupported".into()),
                invalid_value: Some(version.to_string()),
            });
        }
    }

    let allowed = errors.is_empty();
    ValidationOutput {
        allowed,
        message: if allowed {
            Some("registry-enforcer: approved registry".into())
        } else {
            Some(
                errors
                    .iter()
                    .map(|e| e.message.clone())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        },
        reason: if allowed {
            None
        } else {
            Some("PolicyViolation".into())
        },
        errors,
        warnings: vec![],
        audit_annotations: audit,
    }
}
```

### 4.7 I/O helper functions

These helpers are reusable across all plugins — copy them verbatim:

```rust
fn read_validation_input() -> Result<ValidationInput, String> {
    unsafe {
        let len = get_input_len();
        if len <= 0 {
            return Err(format!("get_input_len returned {len}"));
        }
        let mut buf = vec![0u8; len as usize];
        let read = read_input(buf.as_mut_ptr(), len);
        if read != len {
            return Err(format!("read_input: expected {len} bytes, got {read}"));
        }
        serde_json::from_slice(&buf).map_err(|e| format!("JSON parse: {e}"))
    }
}

fn write_output_struct(output: &ValidationOutput) {
    match serde_json::to_vec(output) {
        Ok(json) => unsafe {
            write_output(json.as_ptr(), json.len() as i32);
        },
        Err(e) => log(&format!("registry-enforcer: serialise error: {e}")),
    }
}

fn write_allowed(message: &str) {
    write_output_struct(&ValidationOutput {
        allowed: true,
        message: Some(message.into()),
        ..Default::default()
    });
}

fn write_denied(message: &str, reason: &str) {
    write_output_struct(&ValidationOutput {
        allowed: false,
        message: Some(message.into()),
        reason: Some(reason.into()),
        ..Default::default()
    });
}

fn log(msg: &str) {
    unsafe { log_message(msg.as_ptr(), msg.len() as i32) }
}
```

---

## 5. Run Unit Tests Locally

Because `wasm32-unknown-unknown` does not support `std::process`, tests run on
your native host target.  The policy functions are pure Rust — no unsafe code is
exercised in tests.

```bash
cargo test
```

Expected output:

```
running 5 tests
test tests::approved_registry_is_allowed ... ok
test tests::unapproved_registry_is_denied ... ok
test tests::empty_version_is_denied ... ok
test tests::non_create_operation_passthrough ... ok
test tests::update_with_approved_registry_is_allowed ... ok

test result: ok. 5 passed; 0 failed
```

Add tests for every policy rule before deploying to production.

---

## 6. Compile to WebAssembly

```bash
cargo build --target wasm32-unknown-unknown --release
```

The binary is written to:

```
target/wasm32-unknown-unknown/release/registry_enforcer.wasm
```

### 6.1 Optimise with wasm-opt (recommended)

`wasm-opt` typically reduces binary size by 30–50 % and can improve cold-start
time:

```bash
wasm-opt -Oz \
  -o target/wasm32-unknown-unknown/release/registry_enforcer.opt.wasm \
     target/wasm32-unknown-unknown/release/registry_enforcer.wasm
```

Use the `.opt.wasm` file for deployment.

### 6.2 Verify binary size

The operator enforces a 2 MiB limit on Wasm binaries:

```bash
wc -c < target/wasm32-unknown-unknown/release/registry_enforcer.wasm
# should be well under 2097152 bytes
```

A typical `registry-enforcer` binary compiled with the release profile above is
around 60–90 KiB.

---

## 7. Fail-Open vs Fail-Closed Behaviour

This is one of the most operationally significant decisions when deploying a
validation plugin.

| Behaviour | `failOpen` setting | What happens on plugin error |
|---|---|---|
| **Fail-closed** | `false` (default) | Plugin crash / timeout / OOM → request is **denied** |
| **Fail-open** | `true` | Plugin crash / timeout / OOM → request is **allowed** with a warning annotation |

A "plugin error" includes any of:

- Execution timeout (`timeoutMs` exceeded)
- Out of fuel (`maxFuel` exceeded)
- Linear memory limit hit (`maxMemoryBytes` exceeded)
- Wasm trap (e.g. integer overflow with `panic = "abort"`)
- Empty output buffer (plugin returned without calling `write_output`)

### 7.1 When to choose fail-open

Use `failOpen: true` when:

- The policy is **advisory** or **non-critical** (e.g. adding audit annotations)
- The plugin is **newly deployed** and you want to observe behaviour before enforcing
- A plugin outage must not block operator upgrades or node provisioning
- You are running in a **CI/staging** environment

### 7.2 When to choose fail-closed

Use `failOpen: false` (the default) when:

- The policy enforces a **hard security requirement** (e.g. blocking unapproved registries)
- Compliance mandates that non-verified workloads must not reach the cluster
- You have high confidence in the plugin's stability and resource usage

> **Tip for production rollouts:** start with `failOpen: true` and monitor
> `kubectl logs` for `WARN wasm_plugin` lines over 24–48 hours.  Once you
> confirm the plugin is stable, switch to `failOpen: false`.

### 7.3 Configuring the mode

Set `failOpen` per plugin in `plugins.yaml`:

```yaml
plugins:
  - metadata:
      name: registry-enforcer
      version: "1.0.0"
    configMapRef:
      name: registry-enforcer-plugin
      key: plugin.wasm
      namespace: stellar-operator-system
    operations:
      - CREATE
      - UPDATE
    enabled: true
    failOpen: false   # ← hard enforcement; deny on any plugin error
```

---

## 8. Package the Plugin into a ConfigMap

The operator loads the Wasm binary from a Kubernetes ConfigMap at startup (and
whenever the ConfigMap is updated):

```bash
# Use the optimised binary if you ran wasm-opt, otherwise use the standard one
WASM_FILE=target/wasm32-unknown-unknown/release/registry_enforcer.wasm

kubectl create configmap registry-enforcer-plugin \
  --from-file=plugin.wasm="${WASM_FILE}" \
  --namespace stellar-operator-system \
  --dry-run=client -o yaml | kubectl apply -f -
```

Verify:

```bash
kubectl get configmap registry-enforcer-plugin \
  -n stellar-operator-system \
  -o jsonpath='{.binaryData.plugin\.wasm}' \
  | base64 -d | wc -c
# prints the byte size of the stored binary
```

#### Integrity pinning (recommended for production)

Compute the SHA-256 hash of your binary and record it in `plugins.yaml` so the
runtime rejects a tampered binary at load time:

```bash
sha256sum "${WASM_FILE}"
# e.g. a1b2c3d4...  target/.../registry_enforcer.wasm
```

Add the hash to the plugin metadata:

```yaml
metadata:
  name: registry-enforcer
  version: "1.0.0"
  sha256: "a1b2c3d4..."   # lowercase hex SHA-256 of the .wasm file
```

---

## 9. Deploy and Configure the Operator

### 9.1 Update plugins.yaml

Edit (or create) the operator's plugin configuration file.  If the operator is
deployed with Helm, this lives in a ConfigMap referenced by `--plugin-config`:

```yaml
# plugins.yaml
plugins:
  - metadata:
      name: registry-enforcer
      version: "1.0.0"
      description: "Denies StellarNode specs with unapproved image registries"
      limits:
        timeoutMs: 500          # 500 ms is ample for a registry check
        maxMemoryBytes: 8388608 # 8 MiB — generous for this simple policy
        maxFuel: 500000         # ~500k Wasm instructions
    configMapRef:
      name: registry-enforcer-plugin
      key: plugin.wasm
      namespace: stellar-operator-system
    operations:
      - CREATE
      - UPDATE
    enabled: true
    failOpen: false
```

Apply the ConfigMap containing `plugins.yaml`:

```bash
kubectl create configmap stellar-operator-plugin-config \
  --from-file=plugins.yaml=plugins.yaml \
  --namespace stellar-operator-system \
  --dry-run=client -o yaml | kubectl apply -f -
```

Then update the operator Deployment to reference it:

```yaml
args:
  - webhook
  - --webhook-port=8443
  - --webhook-cert=/certs/tls.crt
  - --webhook-key=/certs/tls.key
  - --plugin-config=/config/plugins.yaml
```

### 9.2 Apply the ValidatingWebhookConfiguration

If not already present, register the webhook with the API server:

```yaml
# validating-webhook.yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingWebhookConfiguration
metadata:
  name: stellar-node-validator
webhooks:
  - name: validate.stellarnode.stellar.org
    clientConfig:
      service:
        name: stellar-operator-webhook
        namespace: stellar-operator-system
        path: /validate
      caBundle: <base64-encoded-ca-cert>
    rules:
      - operations: ["CREATE", "UPDATE"]
        apiGroups: ["stellar.org"]
        apiVersions: ["v1alpha1"]
        resources: ["stellarnodes"]
    admissionReviewVersions: ["v1"]
    sideEffects: None
    timeoutSeconds: 10
    failurePolicy: Fail   # change to Ignore for fail-open at the webhook level
```

```bash
kubectl apply -f validating-webhook.yaml
```

> `failurePolicy: Fail` in the `ValidatingWebhookConfiguration` is a separate
> concern from the plugin's `failOpen` setting.  `failurePolicy` controls what
> the API server does if the **webhook service itself** is unreachable (network
> partition, crash).  `failOpen` controls what the webhook does if an individual
> **plugin** encounters a runtime error.

### 9.3 Verify the plugin loaded

After restarting the operator (or waiting for it to reload config), check the logs:

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep -E "wasm|plugin|registry-enforcer"
```

Expected log lines:

```
INFO  wasm_plugin  loaded plugin registry-enforcer v1.0.0
INFO  wasm_plugin  plugin registry-enforcer: operations=[CREATE, UPDATE] fail_open=false
```

Also verify via the management API:

```bash
# Port-forward the webhook service
kubectl port-forward -n stellar-operator-system svc/stellar-operator-webhook 8443:8443 &

curl -sk https://localhost:8443/plugins | jq .
```

Expected response:

```json
{
  "plugins": [
    {
      "name": "registry-enforcer",
      "version": "1.0.0",
      "description": "Denies StellarNode specs with unapproved image registries",
      "operations": ["CREATE", "UPDATE"],
      "enabled": true
    }
  ]
}
```

---

## 10. Validate End-to-End

### 10.1 Test: request that should be denied

Create a `StellarNode` with an unapproved registry:

```yaml
# denied-node.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-denied
  namespace: stellar
  labels:
    cost-center: platform
spec:
  nodeType: Validator
  network: Testnet
  version: "quay.io/someone/stellar-core:v21.3.0"   # ← not in allow-list
  storage:
    storageClass: standard
    size: "50Gi"
```

```bash
kubectl apply -f denied-node.yaml
```

Expected output:

```
Error from server: error when creating "denied-node.yaml": admission webhook
"validate.stellarnode.stellar.org" denied the request:
image registry is not in the approved list;
approved prefixes: docker.io/stellar/, ghcr.io/myorg/
```

### 10.2 Test: request that should be allowed

Create a `StellarNode` with an approved registry:

```yaml
# allowed-node.yaml
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: test-allowed
  namespace: stellar
  labels:
    cost-center: platform
spec:
  nodeType: Validator
  network: Testnet
  version: "docker.io/stellar/stellar-core:v21.3.0"   # ← approved
  storage:
    storageClass: standard
    size: "50Gi"
```

```bash
kubectl apply -f allowed-node.yaml
# stellarnode.stellar.org/test-allowed created
```

Confirm the audit annotation was written:

```bash
kubectl get stellarnode test-allowed -n stellar \
  -o jsonpath='{.metadata.annotations}' | jq .
# Should include: "registry-enforcer.stellar.org/checked": "true"
```

View plugin logs:

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep wasm_plugin
# registry-enforcer: CREATE on stellar/test-allowed
# registry-enforcer: approved registry
```

---

## 11. Enterprise Hardening Checklist

Before promoting a plugin to a production cluster, verify each item:

- [ ] **Unit test coverage** — every policy branch (allow, deny, edge cases) has a test
- [ ] **`cargo test` passes** on native target
- [ ] **`cargo build --target wasm32-unknown-unknown --release` succeeds** cleanly
- [ ] **Binary size** is under 2 MiB (`wc -c < *.wasm`)
- [ ] **`wasm-opt -Oz`** applied to the deployment binary
- [ ] **SHA-256 hash** recorded in `plugins.yaml` under `metadata.sha256`
- [ ] **`failOpen`** decision documented and intentional
- [ ] **Resource limits** (`timeoutMs`, `maxMemoryBytes`, `maxFuel`) sized to observed usage
- [ ] **Audit annotations** include plugin name and version for traceability
- [ ] **Deployed as fail-open** first; promoted to fail-closed after 48 h of clean observation
- [ ] **`ValidatingWebhookConfiguration.failurePolicy`** set to `Fail` for security-critical policies
- [ ] **Rollback plan** — old ConfigMap version retained and tested
- [ ] **Change control** — plugin ConfigMap updates go through your GitOps pipeline

---

## 12. Clean Up

Remove test resources after validating:

```bash
kubectl delete stellarnode test-allowed test-denied -n stellar 2>/dev/null || true
kubectl delete configmap registry-enforcer-plugin -n stellar-operator-system
kubectl delete configmap stellar-operator-plugin-config -n stellar-operator-system
kubectl delete validatingwebhookconfiguration stellar-node-validator
```

---

*Last updated: 2026-10-01 — issue [#314](https://github.com/OtowoOrg/Stellar-K8s/issues/314)*
