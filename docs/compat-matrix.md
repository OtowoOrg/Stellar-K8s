# Kubernetes Compatibility Matrix

This document describes how Stellar-K8s tests and tracks compatibility across
supported Kubernetes minor versions.

## Purpose

Kubernetes evolves rapidly.  Each minor release may graduate APIs from beta to
GA, change feature-gate defaults, or deprecate behaviour relied upon by the
operator.  The compatibility matrix provides:

- A **single source of truth** for which K8s versions are supported and why.
- **Offline unit tests** that encode the API compatibility rules, runnable in CI
  without a live cluster.
- A **structured JSON export** suitable for release notes, dashboards, and
  automated tooling.

---

## Supported Kubernetes Versions

| Version | Status      | Notes                                                   |
|---------|-------------|---------------------------------------------------------|
| 1.27    | Supported   | Minimum recommended; all features available             |
| 1.28    | Supported   | Recommended for production                              |
| 1.29    | Supported   | Recommended for production                              |
| 1.30    | **Current** | CI target; `k8s-openapi` compiled with `v1_30` feature  |

> **Minimum version**: 1.27 (as noted in the README prerequisites).  
> **CI target**: 1.30.  
> Versions older than 1.27 are not tested and not supported.

---

## Feature Coverage per Version

The following features are evaluated for each supported version.  A ✓ means the
feature is fully supported and tested; ✗ means it is not available or not
validated on that version.

| Feature              | 1.27 | 1.28 | 1.29 | 1.30 | Stable Since |
|----------------------|:----:|:----:|:----:|:----:|:-------------|
| CRD Installation     |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.16     |
| Reconciler (basic)   |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.20+    |
| Finalizer Handling   |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.7      |
| Status Subresource   |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.16     |
| Admission Webhook    |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.16     |
| PVC Expansion (GA)   |  ✓   |  ✓   |  ✓   |  ✓   | K8s 1.24     |
| Service Mesh Sidecar |  ✓   |  ✓   |  ✓   |  ✓   | Istio ≥ 1.17 |

### Feature notes

- **CRD Installation** — Uses CRD v1 (`apiextensions.k8s.io/v1`), stable since
  K8s 1.16.  The `StellarNode` CRD and all related CRDs install without issues
  on all supported versions.

- **Reconciler (basic)** — The `kube-rs` reconciliation loop depends on the
  `watch` and `list` verbs on core resources.  These have been stable since
  K8s 1.20.

- **Finalizer Handling** — Finalizers have been a first-class concept since
  K8s 1.7.  The operator uses them to protect PVCs from premature deletion.

- **Status Subresource** — The `/status` subresource on CRD v1 has been GA
  since 1.16.  The operator updates `StellarNode` status conditions and sync
  state through this subresource.

- **Admission Webhook** — Validating and mutating webhooks
  (`admissionregistration.k8s.io/v1`) have been GA since K8s 1.16.  Used for
  manifest validation and Wasm-based policy enforcement.

- **PVC Expansion** — The `ExpandPersistentVolumes` feature gate was promoted
  to GA in K8s 1.24 and enabled by default.  The disk auto-scaling feature
  relies on this.  All supported versions (1.27+) have it unconditionally
  available.

- **Service Mesh Sidecar** — Sidecar injection is not a native K8s API; it
  depends on the service-mesh controller (Istio or Linkerd) deployed in the
  cluster.  Stellar-K8s validates the mTLS overlay on 1.27+ with Istio ≥ 1.17
  or Linkerd ≥ 2.14.  This feature requires those controllers to be installed
  separately.

---

## Running the Tests

All compatibility matrix tests run **offline** (no cluster needed):

```bash
# Run all tests in the compat_matrix integration test file
cargo test --test compat_matrix

# Run with verbose output (shows the ASCII table)
cargo test --test compat_matrix -- --nocapture

# Run a specific test by name
cargo test --test compat_matrix test_supported_versions_range
cargo test --test compat_matrix test_current_version_fully_supported
```

### Test inventory

| Test name                               | What it checks                                             |
|-----------------------------------------|------------------------------------------------------------|
| `test_supported_versions_range`         | 4 versions returned; range is 1.27 – 1.30                 |
| `test_compat_matrix_construction`       | Full matrix built; all versions present                    |
| `test_matrix_json_export`               | JSON has `versions`, `features`, `results`, `summary` keys |
| `test_matrix_table_format`              | ASCII table has version headers and feature columns        |
| `test_feature_coverage`                 | All `CompatibilityFeature` variants covered for 1.30       |
| `test_k8s_version_display`              | `Display` impl produces `"1.XX"` format                   |
| `test_minimum_supported_version`        | 1.27 is the minimum version                               |
| `test_current_version_fully_supported`  | 1.30 has all features passing                             |
| `test_supported_versions_ordered`       | Versions returned in ascending order                       |
| `test_matrix_current_version_all_pass`  | 1.30 results all pass in the matrix                        |
| `test_all_results_have_test_names`      | No result has an empty `test_name`                        |
| `test_json_export_serializable`         | JSON round-trips through `serde_json`                      |
| `test_k8s_version_serde`                | `K8sVersion` serialises/deserialises correctly             |
| `test_compat_result_serde`              | `CompatTestResult` serialises/deserialises correctly       |
| `test_matrix_table_has_separator_line`  | ASCII table contains a `--+--` separator                  |
| `test_matrix_table_non_empty`           | `print_matrix_table` returns non-empty output              |
| `test_json_summary_total`               | `summary.total` equals `versions × features`              |
| `test_all_features_have_labels`         | Every feature variant has a non-empty label                |
| `test_per_version_result_count`         | Each version returns exactly `N_features` results          |

---

## How to Add a New Kubernetes Version

When a new K8s minor version is released and validated:

1. **Add the version** to `supported_k8s_versions()` in
   `tests/compat_matrix.rs`:

   ```rust
   pub fn supported_k8s_versions() -> Vec<K8sVersion> {
       vec![
           K8sVersion::new(1, 27),
           K8sVersion::new(1, 28),
           K8sVersion::new(1, 29),
           K8sVersion::new(1, 30),
           K8sVersion::new(1, 31),  // ← new version
       ]
   }
   ```

2. **Update `evaluate_feature`** if any feature has new behaviour on the new
   version (new GA graduations, removed feature gates, etc.).

3. **Update the version count assertion** in `test_supported_versions_range`:

   ```rust
   assert_eq!(versions.len(), 5, "Expected 5 supported K8s versions");
   ```

4. **Update the max version assertion**:

   ```rust
   assert_eq!(max.minor, 31, "Maximum supported version must be 1.31");
   ```

5. **Update `test_json_summary_total`** if the expected total changes.

6. **Update this document** — add the new version to the tables above.

7. **Update the Cargo.toml `k8s-openapi` feature** if moving the CI target:

   ```toml
   k8s-openapi = { version = "0.22", features = ["v1_31"] }
   ```

8. Run `cargo check --tests` and `cargo test --test compat_matrix` to confirm
   everything passes.

---

## CI Integration

The compatibility matrix tests are lightweight and run as part of the standard
`cargo test` suite.  They are included in the `ci.yml` workflow automatically
because they live in `tests/` (integration tests compiled by `cargo test
--tests`).

To add a dedicated CI step that prints the ASCII table:

```yaml
# .github/workflows/ci.yml  (example addition)
- name: Compatibility Matrix
  run: cargo test --test compat_matrix -- --nocapture 2>&1 | tee compat-matrix.txt

- name: Upload Compatibility Matrix
  uses: actions/upload-artifact@v4
  with:
    name: compat-matrix
    path: compat-matrix.txt
```

For live cluster testing across versions, the recommended approach is to use
[`kind`](https://kind.sigs.k8s.io/) with explicit version images:

```yaml
- name: Compat test K8s 1.28
  run: |
    kind create cluster --image kindest/node:v1.28.13
    cargo test --test e2e_kind
    kind delete cluster
```

See the `e2e_kind.rs` test file and the `setup-kind-cluster` composite action
in `.github/actions/setup-kind-cluster/` for the full pattern.

---

## Generating the JSON Report

To generate the compatibility matrix JSON for use in release notes or external
tooling, you can write a small Rust binary or use the library functions
directly.  The `matrix_to_json` function is public and can be called from any
code that imports the test module.

Example (add to `src/bin/` if desired):

```rust
fn main() {
    // Build the matrix using the same logic as the test module
    // (copy the public functions to a library module if needed)
    println!("See tests/compat_matrix.rs for the source of truth.");
}
```

Alternatively, capture the output of the test run:

```bash
cargo test --test compat_matrix test_matrix_json_export -- --nocapture 2>/dev/null
```

---

## References

- [`tests/compat_matrix.rs`](../tests/compat_matrix.rs) — source of truth
- [Kubernetes API deprecation policy](https://kubernetes.io/docs/reference/using-api/deprecation-policy/)
- [kube-rs changelog](https://github.com/kube-rs/kube/blob/main/CHANGELOG.md)
- [k8s-openapi version features](https://github.com/Arnavion/k8s-openapi#which-kubernetes-version-to-use)
