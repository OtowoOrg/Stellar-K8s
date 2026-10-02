# registry-enforcer — Enterprise Image-Registry Allow-List Plugin

An enterprise-grade Stellar-K8s Wasm validation plugin that enforces an
image-registry allow-list on every `StellarNode` CREATE and UPDATE.

Any `spec.version` that does not begin with an approved registry prefix is
denied with a structured error before the object is persisted.

> **New to Wasm plugins?** Follow the step-by-step guide at
> [`docs/development/wasm-policies.md`](../../../docs/development/wasm-policies.md)
> before working with this example.

---

## What it enforces

| Field | Rule |
|---|---|
| `spec.version` | Must start with one of the prefixes in `APPROVED_REGISTRIES` |
| `spec.version` | Must be present and non-empty |

Edit `APPROVED_REGISTRIES` in `src/lib.rs` to match your organisation's
approved image sources.

## Quick start

```bash
# 1. Add the wasm32 target (once per machine)
rustup target add wasm32-unknown-unknown

# 2. Run unit tests on the native target
cargo test

# 3. Compile to WebAssembly
cargo build --target wasm32-unknown-unknown --release

# 4. (Optional) Optimise binary size with wasm-opt
wasm-opt -Oz \
  -o target/wasm32-unknown-unknown/release/registry_enforcer.opt.wasm \
     target/wasm32-unknown-unknown/release/registry_enforcer.wasm

# 5. Package into a Kubernetes ConfigMap
kubectl create configmap registry-enforcer-plugin \
  --from-file=plugin.wasm=target/wasm32-unknown-unknown/release/registry_enforcer.wasm \
  --namespace stellar-operator-system \
  --dry-run=client -o yaml | kubectl apply -f -
```

## Configuration

Add the following entry to your operator's `plugins.yaml`:

```yaml
plugins:
  - metadata:
      name: registry-enforcer
      version: "1.0.0"
      description: "Denies StellarNode specs with unapproved image registries"
      limits:
        timeoutMs: 500
        maxMemoryBytes: 8388608   # 8 MiB
        maxFuel: 500000
    configMapRef:
      name: registry-enforcer-plugin
      key: plugin.wasm
      namespace: stellar-operator-system
    operations:
      - CREATE
      - UPDATE
    enabled: true
    failOpen: false   # deny requests when the plugin encounters an error
```

See [`docs/development/wasm-policies.md`](../../../docs/development/wasm-policies.md)
for full deployment instructions, fail-open/fail-closed guidance, and the
end-to-end validation walkthrough.

## Related documentation

- [WASM Policy Authoring Guide](../../../docs/development/wasm-policies.md)
- [Wasm Plugin API Reference](../../../docs/plugins/wasm-api.md)
- [Hello World Tutorial](../hello-world/README.md)
- [Troubleshooting](../../../docs/plugins/wasm-troubleshooting.md)
