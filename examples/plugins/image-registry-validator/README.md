# Image Registry Validator Plugin

Example WebAssembly admission plugin that requires `StellarNode` images to come from approved registries (`docker.io/stellar/*`, `ghcr.io/stellar/*`, `gcr.io/stellar-project/*`).

## Canonical walkthrough

Build, deploy, test, customize, and security notes live in the full Wasm webhook guide:

→ **[docs/wasm-webhook.md](../../../docs/wasm-webhook.md)**

That document is the single source of truth for Wasm plugin development. This README only identifies the example crate under `examples/plugins/image-registry-validator/`.

## Quick pointer

```bash
cd examples/plugins/image-registry-validator
cargo build --target wasm32-unknown-unknown --release
```

Then follow the deploy and test steps in [docs/wasm-webhook.md](../../../docs/wasm-webhook.md).

## License

Apache 2.0
