# Manifest Validation Report

During the validation of the YAML manifests in the `examples/` directory against the API schema, several examples were using fields that do not exist or were misspelled according to the latest `StellarNodeSpec`.

The following broken manifests were identified and fixed:

1. **`examples/cross-cluster-submariner.yaml`**
   - **Error**: Invalid field `spec.cluster`.
   - **Fix**: Removed the `cluster` declarations for the nodes.

2. **`examples/cross-cluster-external-dns.yaml`**
   - **Error**: Invalid field `spec.cluster`.
   - **Fix**: Removed the `cluster` declaration.

3. **`examples/cross-cluster-direct-ip.yaml`**
   - **Error**: Invalid field `spec.cluster`.
   - **Fix**: Removed the `cluster` declaration.

4. **`examples/cross-cluster-istio.yaml`**
   - **Error**: Invalid field `spec.cluster`.
   - **Fix**: Removed the `cluster` declaration under the `spec` block (the `labels.cluster` block was kept as it is valid).

5. **`examples/peer-discovery-example.yaml`**
   - **Error**: Typo in `spec.validatorConfig.quorum_set`.
   - **Fix**: Replaced `quorum_set` with the correct camelCase field `quorumSet` across the three validators.

6. **`examples/cve-handling-examples.yaml`**
   - **Error**: The `horizonConfig` and `sorobanConfig` blocks referenced non-existent fields (`port`, `rpcPort`, `maxConcurrentRequests`) and omitted required fields.
   - **Fix**: Removed the non-existent fields and replaced them with the required `databaseSecretRef` and `stellarCoreUrl` fields.
