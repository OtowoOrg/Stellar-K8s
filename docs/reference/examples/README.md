# CRD configuration examples

Production-oriented `StellarNode` manifests for the three published
`spec.nodeType` values. Horizon and Soroban RPC are node types on
`StellarNode`; they are not separate CRDs.

| File | nodeType | network |
|---|---|---|
| [validator-mainnet.yaml](validator-mainnet.yaml) | `Validator` | `mainnet` |
| [horizon-api.yaml](horizon-api.yaml) | `Horizon` | `mainnet` |
| [soroban-rpc.yaml](soroban-rpc.yaml) | `SorobanRpc` | `testnet` |

Every example includes the OpenAPI-required spec fields:
`nodeType`, `network`, `version`, `minAvailable`, `maxUnavailable`, and
`topologySpreadConstraints`.

Validate against the published CRD:

```bash
kubectl apply -f config/crd/stellarnode-crd.yaml
kubectl apply --dry-run=client -f docs/reference/examples/
```

See the [CRD Architecture Reference Manual](../crd-manual.md) for the full
schema catalog.
