# Hello World — Wasm Plugin Tutorial

This tutorial walks you from zero to a running, deployed Wasm validation plugin
for the Stellar Kubernetes Operator.  By the end you will have:

- A compilable Rust crate that targets `wasm32-unknown-unknown`
- A loaded plugin that enforces two real admission-control policies
- Hands-on familiarity with the host ABI described in
  [`docs/plugins/wasm-api.md`](../../docs/plugins/wasm-api.md)

**Time to complete:** ~30 minutes  
**Difficulty:** Beginner (familiarity with Rust and `kubectl` assumed)

> **Troubleshooting:** If anything goes wrong, check
> [`docs/plugins/wasm-troubleshooting.md`](../../docs/plugins/wasm-troubleshooting.md).

---

## Table of Contents

1. [What the plugin does](#1-what-the-plugin-does)
2. [Prerequisites](#2-prerequisites)
3. [Project structure](#3-project-structure)
4. [Write the plugin](#4-write-the-plugin)
   - 4.1 [Cargo.toml](#41-cargotoml)
   - 4.2 [Declare host function imports](#42-declare-host-function-imports)
   - 4.3 [Define the data types](#43-define-the-data-types)
   - 4.4 [Implement `validate()`](#44-implement-validate)
   - 4.5 [Implement the policy](#45-implement-the-policy)
   - 4.6 [I/O helpers](#46-io-helpers)
5. [Run unit tests locally](#5-run-unit-tests-locally)
6. [Compile to WebAssembly](#6-compile-to-webassembly)
7. [Inspect the binary](#7-inspect-the-binary)
8. [Deploy to the cluster](#8-deploy-to-the-cluster)
   - 8.1 [Store the plugin in a ConfigMap](#81-store-the-plugin-in-a-configmap)
   - 8.2 [Update the operator plugin config](#82-update-the-operator-plugin-config)
   - 8.3 [Verify the plugin loaded](#83-verify-the-plugin-loaded)
9. [Test the plugin end-to-end](#9-test-the-plugin-end-to-end)
   - 9.1 [Test: request that should be denied](#91-test-request-that-should-be-denied)
   - 9.2 [Test: request that should be allowed](#92-test-request-that-should-be-allowed)
10. [Extend the plugin](#10-extend-the-plugin)
11. [Clean up](#11-clean-up)

---

## 1. What the plugin does

The hello-world plugin enforces two admission-control policies on every
`StellarNode` CREATE and UPDATE:

| # | Rule | Field | Condition |
|---|---|---|---|
| 1 | **Mainnet replica requirement** | `spec.replicas` | Must be `>= 3` when `spec.network == "Mainnet"` |
| 2 | **Required cost-center label** | `metadata.labels["cost-center"]` | Must be present and non-empty |

It also emits an advisory **warning** (non-blocking) when
`spec.resources.limits.memory` is less than `512Mi`.

DELETE and CONNECT operations are passed through without any checks.

---

## 2. Prerequisites

Install the following tools before starting.

### Rust toolchain

```bash
# Install Rust via rustup (https://rustup.rs)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Confirm the installation
rustc --version   # e.g. rustc 1.78.0
cargo --version   # e.g. cargo 1.78.0
```

### wasm32-unknown-unknown target

```bash
rustup target add wasm32-unknown-unknown

# Confirm
rustup target list --installed | grep wasm32-unknown-unknown
# should print: wasm32-unknown-unknown (installed)
```

### Optional: wasm-opt (binary size optimiser)

`wasm-opt` is part of the [Binaryen](https://github.com/WebAssembly/binaryen)
toolchain.  It is not required to build a working plugin but reduces binary size
by ~30–50 %.

```bash
# macOS
brew install binaryen

# Ubuntu / Debian
apt-get install binaryen

# Or via cargo
cargo install wasm-opt
```

### kubectl and a running cluster

You need `kubectl` configured against a cluster with the Stellar-K8s operator
deployed.

```bash
kubectl version --client
kubectl get deployment -n stellar-operator-system stellar-operator
```

---

## 3. Project structure

The finished project looks like this:

```
examples/wasm-plugins/hello-world/
├── Cargo.toml          # crate manifest
├── build.sh            # convenience build script
├── README.md           # this file
└── src/
    └── lib.rs          # plugin source code
```

You are reading the completed tutorial.  The source files in this directory are
the finished product — you can read them alongside each step below.

---

## 4. Write the plugin

### 4.1 `Cargo.toml`

Create the manifest.  Two key points:

- `crate-type = ["cdylib"]` — produces a C-ABI dynamic library, which the Wasm
  linker turns into a `.wasm` file with exported symbols.
- The `[profile.release]` section ensures a small, fast binary.

```toml
[package]
name = "hello-world"
version = "0.1.0"
edition = "2021"
description = "Hello World Wasm validation plugin for Stellar-K8s"
license = "Apache-2.0"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

[profile.release]
opt-level = "z"       # optimise for size
lto = true            # link-time optimisation
codegen-units = 1
panic = "abort"       # no unwinding machinery (~30 KiB savings)
strip = true
```

> **Why `panic = "abort"`?**  The standard `panic = "unwind"` links in the
> Rust unwinding runtime which adds ~30 KiB to the Wasm binary.  Since we
> cannot sensibly recover from a panic inside a sandboxed plugin, aborting is
> the right choice and keeps the binary small.

---

### 4.2 Declare host function imports

The Stellar-K8s runtime provides four functions in the `env` module.  Declare
them with `extern "C"` so the Wasm linker knows to import them:

```rust
extern "C" {
    fn get_input_len() -> i32;
    fn read_input(ptr: *mut u8, len: i32) -> i32;
    fn write_output(ptr: *const u8, len: i32) -> i32;
    fn log_message(ptr: *const u8, len: i32);
}
```

These must match the signatures in
[`docs/plugins/wasm-api.md — Host Functions`](../../docs/plugins/wasm-api.md#host-functions)
exactly.  The compiler will error if the types don't align.

---

### 4.3 Define the data types

Define Rust structs that map to the JSON the runtime passes in and expects back.
Use `serde`'s `rename_all = "camelCase"` to match the operator's JSON keys:

```rust
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidationInput {
    operation: String,
    object: Option<serde_json::Value>,
    namespace: String,
    name: String,
    user_info: UserInfo,
    #[serde(default)]
    context: std::collections::BTreeMap<String, String>,
}

#[derive(serde::Serialize)]
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
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    audit_annotations: std::collections::BTreeMap<String, String>,
}
```

`serde_json::Value` for `object` is intentional — StellarNode specs can evolve
and the plugin should not break when new fields are added.

---

### 4.4 Implement `validate()`

The `#[no_mangle]` attribute prevents the Rust compiler from mangling the
function name so the runtime can find the export `validate`:

```rust
#[no_mangle]
pub extern "C" fn validate() -> i32 {
    // 1. Read the JSON input from the host buffer.
    let input = match read_validation_input() {
        Ok(v) => v,
        Err(msg) => {
            log(&format!("hello-world: input error: {msg}"));
            write_denied(&format!("plugin error: {msg}"), "PluginError");
            return 1;
        }
    };

    // 2. Pass DELETE / CONNECT through unchanged.
    if input.operation != "CREATE" && input.operation != "UPDATE" {
        write_allowed("skipped non-mutating operation", &input.operation);
        return 0;
    }

    // 3. Require an object (always present for CREATE/UPDATE).
    let object = match &input.object {
        Some(o) => o,
        None => {
            write_denied("no object in request", "InvalidInput");
            return 1;
        }
    };

    // 4. Apply policy rules and write the output.
    let output = apply_policy(object);
    let rc = if output.allowed { 0 } else { 1 };
    write_output_struct(&output);
    rc
}
```

**Why check `input.operation` before `input.object`?**  For DELETE operations
the `object` field may be `null`; checking the operation first avoids a
misleading "no object" error message.

---

### 4.5 Implement the policy

The core logic lives in `apply_policy`.  It receives the raw JSON object and
returns a `ValidationOutput`:

```rust
fn apply_policy(object: &serde_json::Value) -> ValidationOutput {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut audit = std::collections::BTreeMap::new();

    // Rule 1 — required "cost-center" label
    let has_label = object
        .pointer("/metadata/labels/cost-center")
        .map(|v| !v.as_str().unwrap_or("").is_empty())
        .unwrap_or(false);

    if !has_label {
        errors.push(ValidationError {
            field: "metadata.labels.cost-center".into(),
            message: r#"label "cost-center" is required"#.into(),
        });
    }

    // Rule 2 — Mainnet needs >= 3 replicas
    let network  = object.pointer("/spec/network").and_then(|v| v.as_str()).unwrap_or("");
    let replicas = object.pointer("/spec/replicas").and_then(|v| v.as_i64()).unwrap_or(1);

    if network == "Mainnet" && replicas < 3 {
        errors.push(ValidationError {
            field: "spec.replicas".into(),
            message: format!("Mainnet nodes need replicas >= 3, got {replicas}"),
        });
    }

    // Advisory — low memory
    // (omitted here for brevity — see src/lib.rs for the full version)

    audit.insert("hello-world.stellar.org/checked".into(), "true".into());

    let allowed = errors.is_empty();
    ValidationOutput {
        allowed,
        message: if allowed { Some("all checks passed".into()) }
                 else { Some(errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ")) },
        reason:  if allowed { None } else { Some("PolicyViolation".into()) },
        errors,
        warnings,
        audit_annotations: audit,
    }
}
```

`serde_json::Value::pointer` uses [JSON Pointer syntax (RFC 6901)](https://www.rfc-editor.org/rfc/rfc6901)
to navigate nested objects without boilerplate `get` / `unwrap` chains.

---

### 4.6 I/O helpers

Three small helpers keep the entry point readable:

```rust
fn read_validation_input() -> Result<ValidationInput, String> {
    unsafe {
        let len = get_input_len();
        if len <= 0 { return Err(format!("get_input_len returned {len}")); }
        let mut buf = vec![0u8; len as usize];
        let read = read_input(buf.as_mut_ptr(), len);
        if read != len { return Err(format!("expected {len} bytes, got {read}")); }
        serde_json::from_slice(&buf).map_err(|e| format!("parse error: {e}"))
    }
}

fn write_output_struct(output: &ValidationOutput) {
    if let Ok(json) = serde_json::to_vec(output) {
        unsafe { write_output(json.as_ptr(), json.len() as i32); }
    }
}

fn log(msg: &str) {
    unsafe { log_message(msg.as_ptr(), msg.len() as i32); }
}
```

> **Safety note:** All calls to host functions are `unsafe` because they cross
> the Wasm/host boundary.  The host validates pointer ranges before reading or
> writing, so a bad pointer returns `-1` rather than causing undefined
> behaviour.  Always check return values in production plugins.

---

## 5. Run unit tests locally

The `#[cfg(test)]` module at the bottom of `src/lib.rs` tests `apply_policy`
on the native target — no Wasm toolchain needed.

```bash
cd examples/wasm-plugins/hello-world
cargo test
```

Expected output:

```
running 7 tests
test tests::mainnet_with_3_replicas_and_label_is_allowed ... ok
test tests::mainnet_with_1_replica_is_denied ... ok
test tests::missing_cost_center_is_denied ... ok
test tests::multiple_violations_reported_together ... ok
test tests::testnet_with_1_replica_is_allowed ... ok
test tests::low_memory_produces_warning ... ok
test tests::parse_memory_mib_works ... ok

test result: ok. 7 passed; 0 failed; 0 ignored
```

> Tests run against `apply_policy` directly — no host functions are called,
> so no Wasm runtime is required.

---

## 6. Compile to WebAssembly

### Using the build script (recommended)

```bash
chmod +x build.sh
./build.sh
```

The script:
1. Adds the `wasm32-unknown-unknown` target if it is missing.
2. Runs `cargo test` (native) to catch logic errors before compiling.
3. Runs `cargo build --target wasm32-unknown-unknown --release`.
4. Prints the output path and file size.

### Manually

```bash
cargo build --target wasm32-unknown-unknown --release
```

The output is at:

```
target/wasm32-unknown-unknown/release/hello_world.wasm
```

### With size optimisation (optional)

```bash
./build.sh --opt
# or manually:
wasm-opt -Oz \
  -o target/wasm32-unknown-unknown/release/hello_world.opt.wasm \
     target/wasm32-unknown-unknown/release/hello_world.wasm
```

### Expected binary sizes

| Build | Typical size |
|---|---|
| Debug | ~800 KiB |
| Release (`opt-level = "z"`, `strip = true`) | ~50–80 KiB |
| Release + `wasm-opt -Oz` | ~35–55 KiB |

---

## 7. Inspect the binary

Verify the compiled module exports the required symbols before deploying.

### Using `wasm-objdump` (wasm-tools / wabt)

```bash
# Install wabt
brew install wabt        # macOS
apt-get install wabt     # Ubuntu

wasm-objdump -x target/wasm32-unknown-unknown/release/hello_world.wasm \
  | grep -E "Export|Import"
```

Look for:

```
Export[2]:
 - func[0] <validate> -> "validate"
 - memory[0] -> "memory"

Import[4]:
 - func env.get_input_len -> [0]
 - func env.read_input    -> [1]
 - func env.write_output  -> [2]
 - func env.log_message   -> [3]
```

Both `validate` and `memory` must appear in the Export section, and all four
host functions must appear in the Import section.

### Using `wasm-tools` (Rust)

```bash
cargo install wasm-tools
wasm-tools validate target/wasm32-unknown-unknown/release/hello_world.wasm
# prints: hello_world.wasm is a valid WebAssembly file
```

---

## 8. Deploy to the cluster

### 8.1 Store the plugin in a ConfigMap

```bash
WASM=target/wasm32-unknown-unknown/release/hello_world.wasm

kubectl create configmap hello-world-plugin \
  --from-file=plugin.wasm="$WASM" \
  --namespace stellar-operator-system \
  --dry-run=client -o yaml | kubectl apply -f -
```

Verify:

```bash
kubectl get configmap hello-world-plugin -n stellar-operator-system
# NAME                  DATA   AGE
# hello-world-plugin    1      5s
```

### 8.2 Update the operator plugin config

Locate your operator's plugin configuration file (typically mounted at
`/config/plugins.yaml` inside the operator pod, or managed via a ConfigMap).
Add the hello-world entry:

```yaml
plugins:
  - metadata:
      name: hello-world
      version: "0.1.0"
      description: "Hello World tutorial plugin"
      limits:
        timeoutMs: 1000
        maxMemoryBytes: 16777216   # 16 MiB
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

Apply the updated config and restart the operator:

```bash
kubectl rollout restart deployment/stellar-operator -n stellar-operator-system
kubectl rollout status  deployment/stellar-operator -n stellar-operator-system
```

### 8.3 Verify the plugin loaded

```bash
# Tail the operator logs and look for the load confirmation
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep "hello-world"
```

Expected log line:

```
INFO stellar_operator::webhook::runtime: Plugin hello-world loaded successfully
```

You can also query the management API (replace `<POD>` with the operator pod name):

```bash
kubectl exec -n stellar-operator-system <POD> -- \
  curl -sk https://localhost:8443/plugins | jq '.plugins[].name'
```

---

## 9. Test the plugin end-to-end

### 9.1 Test: request that should be denied

Apply a StellarNode that violates both rules (no cost-center label, Mainnet with
1 replica):

```bash
cat <<'EOF' | kubectl apply -f -
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: bad-node
  namespace: default
  # cost-center label intentionally missing
spec:
  network: Mainnet
  replicas: 1               # too few for Mainnet
  version: "docker.io/stellar/stellar-core:v21.3.0"
  resources:
    limits:
      cpu: "1"
      memory: "2Gi"
EOF
```

Expected output:

```
Error from server: error when creating "STDIN":
admission webhook "validate.stellarnode.stellar.org" denied the request:
[hello-world] label "cost-center" is required on every StellarNode;
[hello-world] Mainnet StellarNodes must have spec.replicas >= 3, got 1
```

### 9.2 Test: request that should be allowed

Apply a valid StellarNode:

```bash
cat <<'EOF' | kubectl apply -f -
apiVersion: stellar.org/v1alpha1
kind: StellarNode
metadata:
  name: good-node
  namespace: default
  labels:
    cost-center: "engineering"
spec:
  network: Mainnet
  replicas: 3
  version: "docker.io/stellar/stellar-core:v21.3.0"
  resources:
    limits:
      cpu: "2"
      memory: "4Gi"
EOF
```

Expected output:

```
stellarnode.stellar.org/good-node created
```

Verify the audit annotation was written:

```bash
kubectl get stellarnode good-node -o jsonpath=\
  '{.metadata.annotations.hello-world\.stellar\.org/checked}'
# true
```

View plugin debug logs:

```bash
kubectl logs -n stellar-operator-system deployment/stellar-operator \
  | grep "wasm_plugin"
# hello-world: validating CREATE operation
# hello-world: network=Mainnet, replicas=3
```

---

## 10. Extend the plugin

Some ideas to practice with:

### Add a new policy rule

Inside `apply_policy`, add a check for the `owner` annotation:

```rust
let has_owner = object
    .pointer("/metadata/annotations/owner")
    .map(|v| !v.as_str().unwrap_or("").is_empty())
    .unwrap_or(false);

if !has_owner {
    errors.push(ValidationError {
        field: "metadata.annotations.owner".into(),
        message: r#"annotation "owner" is required"#.into(),
    });
}
```

Then add a matching unit test:

```rust
#[test]
fn missing_owner_annotation_is_denied() {
    let obj = serde_json::json!({
        "metadata": {
            "labels": { "cost-center": "eng" },
            "annotations": {}            // owner missing
        },
        "spec": { "network": "Testnet", "replicas": 1 }
    });
    let out = apply_policy(&obj);
    assert!(!out.allowed);
    assert!(out.errors.iter().any(|e| e.field.contains("owner")));
}
```

### Use `context` to pass operator-controlled config

The `context` map lets the operator inject configuration at runtime without
recompiling the plugin:

```yaml
# In plugins.yaml
pluginConfig:
  allowed_networks: "Mainnet,Testnet"
```

In the plugin, read it from `input.context`:

```rust
let allowed = input.context
    .get("allowed_networks")
    .map(|v| v.split(',').any(|n| n.trim() == network))
    .unwrap_or(true); // default: allow all networks
```

---

## 11. Clean up

```bash
# Remove the test StellarNodes
kubectl delete stellarnode good-node bad-node --ignore-not-found

# Remove the plugin ConfigMap
kubectl delete configmap hello-world-plugin -n stellar-operator-system

# Remove the plugin entry from plugins.yaml, then restart the operator
kubectl rollout restart deployment/stellar-operator -n stellar-operator-system
```

---

## Next steps

- Read the full [Wasm Plugin API Reference](../../docs/plugins/wasm-api.md)
- Study the [image-registry-validator](../../../examples/plugins/image-registry-validator/)
  for a more complete example
- Check the [Sandboxing Troubleshooting Guide](../../docs/plugins/wasm-troubleshooting.md)
  if your plugin behaves unexpectedly
