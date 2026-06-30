# Admission Webhook Conformance Tests

This document describes the conformance test suite added in [issue #1052] that
verifies the `StellarNode` admission webhook rejects all invalid CRD payloads
with clear, actionable messages and admits all valid ones.

## Overview

The test file lives at `tests/admission_webhook_conformance.rs` and covers the
full validation pipeline end-to-end:

```
AdmissionReview → WebhookServer::validate
                      ├─ validate_spec_builtin
                      │      ├─ serde deserialization
                      │      ├─ PSS 'restricted' check
                      │      ├─ OrgValidator (labels + resource limits)
                      │      └─ StellarNodeSpec::validate()
                      └─ Wasm plugins (if any)
```

All 52 tests are **hermetic** — no Kubernetes cluster, network connection, or
external service is required.

## Running the Tests

```bash
# Run only the conformance suite
cargo test --test admission_webhook_conformance

# Run with output visible (useful for debugging failures)
cargo test --test admission_webhook_conformance -- --nocapture

# Run a single test by name
cargo test --test admission_webhook_conformance conformance_validator_missing_config_is_denied
```

## Test Categories

### 1 · Baseline — valid payloads must be admitted

| Test | Description |
|------|-------------|
| `conformance_valid_validator_is_admitted` | A fully valid Validator spec is admitted |
| `conformance_valid_horizon_is_admitted` | A fully valid Horizon spec is admitted |
| `conformance_valid_soroban_is_admitted` | A fully valid SorobanRpc spec is admitted |

### 2 · Malformed JSON / missing required fields

| Test | Description |
|------|-------------|
| `conformance_empty_object_is_denied` | `{}` cannot be deserialized as a StellarNode |
| `conformance_missing_spec_is_denied` | Payload with no `spec` key is denied |
| `conformance_null_spec_is_denied` | `spec: null` is denied |
| `conformance_none_object_is_admitted_with_no_plugins` | `None` object with no plugins is admitted |
| `conformance_non_stellarnode_json_is_denied` | Valid JSON that is not a StellarNode (e.g., `kind: Pod`) is denied |

### 3 · Invalid `nodeType`

| Test | Description |
|------|-------------|
| `conformance_unknown_node_type_is_denied` | Unknown discriminant (e.g., `Archiver`) is denied with a descriptive message |
| `conformance_empty_node_type_is_denied` | Empty string `nodeType` is denied |

### 4 · Validator-specific violations

| Test | Description |
|------|-------------|
| `conformance_validator_missing_config_is_denied` | `validatorConfig` is required for `nodeType: Validator` |
| `conformance_validator_with_two_replicas_is_denied` | Validators must have exactly 1 replica |
| `conformance_validator_with_zero_replicas_is_denied` | Zero replicas is also invalid for Validators |
| `conformance_validator_with_autoscaling_is_denied` | `autoscaling` is not supported for Validators |
| `conformance_validator_with_ingress_is_denied` | `ingress` is not supported for Validators |
| `conformance_validator_history_archive_without_urls_is_denied` | `enableHistoryArchive: true` requires non-empty `historyArchiveUrls` |
| `conformance_validator_history_archive_with_urls_is_admitted` | Valid history archive config is admitted |

### 5 · Horizon-specific violations

| Test | Description |
|------|-------------|
| `conformance_horizon_missing_config_is_denied` | `horizonConfig` is required for `nodeType: Horizon` |
| `conformance_horizon_with_validator_node_type_no_validator_config_is_denied` | Mixed type/config is denied |
| `conformance_horizon_multiple_replicas_is_admitted` | Horizon supports multiple replicas |

### 6 · SorobanRpc-specific violations

| Test | Description |
|------|-------------|
| `conformance_soroban_missing_config_is_denied` | `sorobanConfig` is required for `nodeType: SorobanRpc` |
| `conformance_soroban_multiple_replicas_is_admitted` | SorobanRpc supports multiple replicas |

### 7 · Cross-cutting spec violations

| Test | Description |
|------|-------------|
| `conformance_pdb_min_available_and_max_unavailable_conflict_is_denied` | Both PDB fields set simultaneously is invalid |
| `conformance_both_database_and_managed_database_is_denied` | External and managed database are mutually exclusive |
| `conformance_invalid_custom_network_name_is_denied` | Custom network name must conform to DNS-1123 |
| `conformance_custom_network_name_too_long_is_denied` | Custom network name exceeding 63 chars is denied |

### 8 · Organisational-standards violations

| Test | Description |
|------|-------------|
| `conformance_missing_project_id_label_is_denied` | `metadata.labels.project-id` is required |
| `conformance_missing_owner_label_is_denied` | `metadata.labels.owner` is required |
| `conformance_no_labels_is_denied` | No labels at all is denied |
| `conformance_empty_project_id_label_is_denied` | Whitespace-only label value is treated as missing |
| `conformance_validator_cpu_limit_exceeds_max_is_denied` | CPU limit > 8 cores for Validator is denied |
| `conformance_validator_memory_limit_exceeds_max_is_denied` | Memory limit > 16 GiB for Validator is denied |
| `conformance_empty_cpu_request_is_denied` | Zero / unset CPU request is denied |
| `conformance_empty_memory_request_is_denied` | Zero / unset memory request is denied |
| `conformance_empty_cpu_limit_is_denied` | Zero / unset CPU limit is denied |
| `conformance_empty_memory_limit_is_denied` | Zero / unset memory limit is denied |
| `conformance_mainnet_validator_underprovisionned_is_denied` | Mainnet Validator must meet minimum resource requests (2 CPU / 4 GiB) |

### 9 · PSS `restricted` violations

| Test | Description |
|------|-------------|
| `conformance_privileged_security_context_is_denied` | `securityContext.privileged: true` violates the PSS `restricted` profile |

### 10 · Storage validation

| Test | Description |
|------|-------------|
| `conformance_snapshot_ref_both_fields_is_denied` | `snapshotRef` cannot set both `volumeSnapshotName` and `backupUrl` |
| `conformance_snapshot_ref_no_fields_is_denied` | `snapshotRef` with neither field set is invalid |
| `conformance_local_storage_without_class_or_affinity_is_denied` | `LocalStorage` mode requires a `storageClass` or `nodeAffinity` |

### 11 · Operation semantics

| Test | Description |
|------|-------------|
| `conformance_delete_operation_bypasses_spec_validation` | `DELETE` is always admitted (resource is being removed) |
| `conformance_connect_operation_bypasses_spec_validation` | `CONNECT` is always admitted |

### 12 · `UPDATE` operation

| Test | Description |
|------|-------------|
| `conformance_valid_update_is_admitted` | A valid spec change (version bump) is admitted |
| `conformance_update_removing_validator_config_is_denied` | Removing `validatorConfig` via UPDATE is denied |
| `conformance_update_adding_autoscaling_to_validator_is_denied` | Adding `autoscaling` to a Validator via UPDATE is denied |
| `conformance_update_dropping_required_label_is_denied` | Dropping a required label via UPDATE is denied |

### 13 · Edge cases

| Test | Description |
|------|-------------|
| `conformance_rejection_messages_are_non_empty` | All denial messages are non-empty and contain only printable characters |
| `conformance_latest_version_tag_admitted_with_warning` | `version: latest` is admitted but produces an image-pinning warning |
| `conformance_mutable_version_tag_admitted_with_warning` | Mutable semver tags are admitted with a warning |
| `conformance_digest_pinned_version_admitted_without_warning` | Digest-pinned versions are admitted without any pinning warning |
| `conformance_multiple_violations_all_reported` | Multiple independent violations are all included in a single denial |

## Validation Pipeline Reference

The webhook validation pipeline runs in this order for `CREATE` and `UPDATE`
operations. The first stage that fails short-circuits the rest.

1. **Deserialization** — the request body is parsed into a `StellarNode`. Any
   serde error (unknown `nodeType`, missing required field, wrong type) is
   immediately denied.

2. **PSS `restricted` check** — the spec's security context is validated against
   the Kubernetes Pod Security Standards `restricted` profile. Any violation
   (e.g., `privileged: true`) is denied.

3. **OrgValidator** — organisational policy is enforced:
   - `metadata.labels` must include non-empty `project-id` and `owner`.
   - `spec.resources.requests` and `spec.resources.limits` must be non-zero.
   - Resource limits must not exceed the per-node-type maximum.
   - On `network: mainnet`, resource requests must meet the per-node-type
     minimum (prevents under-provisioned nodes from reaching the public
     network).

4. **StellarNodeSpec::validate()** — semantic validation:
   - Node-type-specific config sections are required.
   - Validator replica count must be exactly 1.
   - Autoscaling and ingress are not supported for Validators.
   - Storage `snapshotRef` field rules.
   - PDB mutual exclusion (`minAvailable` vs. `maxUnavailable`).
   - Database mutual exclusion (`database` vs. `managedDatabase`).
   - Custom network name DNS-1123 conformance.

5. **Wasm plugins** — organisation-specific plugins loaded at runtime (not
   covered by this conformance suite, which runs with zero plugins).

`DELETE` and `CONNECT` operations skip steps 2–5 entirely.

## Adding New Conformance Tests

1. Add a `#[tokio::test]` function named `conformance_<scenario>` in
   `tests/admission_webhook_conformance.rs`.
2. Use the `make_input` or `make_update_input` helpers to build the input.
3. Use `valid_validator_json()`, `valid_horizon_json()`, or
   `valid_soroban_json()` as a base and mutate the relevant fields.
4. Assert `result.allowed` or `!result.allowed` and, for denials, assert that
   `result.message` contains the relevant field name or keyword.
5. Add a row to the appropriate table in this document.

## Related Files

| File | Purpose |
|------|---------|
| `tests/admission_webhook_conformance.rs` | This conformance test suite |
| `src/webhook/server.rs` | `WebhookServer`, `validate_spec_builtin`, HTTP handlers |
| `src/webhook/org_validator.rs` | Organisational-standards validator |
| `src/crd/stellar_node.rs` | `StellarNodeSpec::validate()` |
| `src/crd/tests.rs` | Unit tests for `StellarNodeSpec::validate()` |
| `src/controller/pss.rs` | PSS `restricted` compliance checker |
| `docs/wasm-webhook.md` | Wasm plugin development guide |
| `docs/api-reference.md` | Full `StellarNode` CRD field reference |
