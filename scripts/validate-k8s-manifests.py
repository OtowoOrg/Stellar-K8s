#!/usr/bin/env python3
"""
validate-k8s-manifests.py — Kubernetes Manifest & CRD Schema Validation (issue #1394)

Validates all Kubernetes manifests in config/, examples/, bundle/, and
charts/stellar-operator/rendered/ against Kubernetes OpenAPI schemas and
custom CRD definitions using kubeconform with locally-generated JSON schemas.

Used by both:
  - CI workflow: .github/workflows/k8s-manifest-validation.yml
  - Pre-commit hook: k8s-manifest-crd-validation
"""

import glob
import os
import subprocess
import sys
import yaml

KUBERNETES_VERSION = "1.30.0"
KUBECONFORM_VERSION = "v0.6.4"

# Files/dirs that contain YAML but are NOT Kubernetes manifests.
# These are excluded from kubeconform validation (but still checked for
# valid YAML syntax in the structure pass).
NON_K8S_YAML_EXCLUDES = [
    "config/operator-config.yaml",
    "config/yaml-validation.yaml",
    "config/stellar-bench.yaml",
    "config/shell-safety.yaml",
    # Cost and capacity planning models (issues #1834, #1835). They are plain
    # configuration consumed by scripts/cost-estimate.py and
    # scripts/capacity-plan.py, not Kubernetes API objects.
    "config/cost-estimate.yaml",
    "config/capacity-planning.yaml",
]

# Fragments starting with underscore are partial YAML snippets.
FRAGMENT_PREFIX = "_"

# Health sidecar image resolution (issue #1552).
# The sidecar must resolve to ghcr.io/stellar/stellar-k8s:{version} or fall
# back to the operator image. The legacy derivation
# container_image().replace('stellar-core', 'stellar-k8s') produced
# docker.io/stellar/stellar-k8s, which does not exist.
LEGACY_SIDECAR_IMAGE = "docker.io/stellar/stellar-k8s"
VALID_SIDECAR_IMAGE_PREFIX = "ghcr.io/stellar/stellar-k8s"

# Seed secret key standardization (issue #1554).
PREFERRED_SEED_KEY = "STELLAR_CORE_SEED"
LEGACY_SEED_KEY = "SEED"
=======


def find_project_root():
    """Walk up from this script to find the repository root."""
    return os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))


def ensure_kubeconform():
    """Verify kubeconform is available; attempt to install it if not."""
    if subprocess.run(["which", "kubeconform"], capture_output=True).returncode == 0:
        return True

    # Try installing from GitHub release (works on Linux CI runners)
    print("-> kubeconform not found, attempting install...")
    try:
        subprocess.run(
            [
                "bash", "-c",
                f'wget -qO- https://github.com/yannh/kubeconform/releases/download/'
                f'{KUBECONFORM_VERSION}/kubeconform-linux-amd64.tar.gz | tar -xz '
                f'&& sudo mv kubeconform /usr/local/bin/'
            ],
            check=True, capture_output=True,
        )
        print("  [OK] kubeconform installed successfully")
        return True
    except (subprocess.CalledProcessError, FileNotFoundError):
        print("  [FAIL] Could not install kubeconform", file=sys.stderr)
        return False


def is_k8s_manifest(filepath, root_dir):
    """Check if a file is likely a Kubernetes manifest (vs config/fragment YAML)."""
    rel = os.path.relpath(filepath, root_dir).replace("\\", "/")
    # Exclude known non-K8s YAML files
    if rel in NON_K8S_YAML_EXCLUDES:
        return False
    # Exclude underscore-prefixed fragments
    basename = os.path.basename(filepath)
    if basename.startswith(FRAGMENT_PREFIX):
        return False
    return True


def validate_yaml_structure(filepath, root_dir):
    """Validate YAML syntax and mandatory Kubernetes manifest fields."""
    errors = []
    rel = os.path.relpath(filepath, root_dir).replace("\\", "/")
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            docs = list(yaml.safe_load_all(f))
    except yaml.YAMLError as e:
        return [f"{rel}: YAML parse error: {e}"]

    is_k8s = is_k8s_manifest(filepath, root_dir)

    for idx, doc in enumerate(docs):
        if doc is None:
            continue
        if not isinstance(doc, dict):
            errors.append(f"{rel}: document {idx} is not a YAML mapping")
            continue

        # Skip K8s field checks for non-manifest YAML
        if not is_k8s:
            continue

        kind = doc.get("kind")
        api_version = doc.get("apiVersion")
        metadata = doc.get("metadata")

        if not kind:
            errors.append(f"{rel}: document {idx} missing 'kind'")
        if not api_version:
            errors.append(f"{rel}: document {idx} missing 'apiVersion'")
        if not metadata or not isinstance(metadata, dict) or not metadata.get("name"):
            errors.append(f"{rel}: document {idx} missing 'metadata.name'")

        # Structural checks for CRDs
        if kind == "CustomResourceDefinition":
            spec = doc.get("spec", {})
            if not spec.get("group"):
                errors.append(f"{rel}: CRD missing spec.group")
            if not spec.get("names", {}).get("kind"):
                errors.append(f"{rel}: CRD missing spec.names.kind")
            if not spec.get("versions"):
                errors.append(f"{rel}: CRD missing spec.versions")

    return errors


def _iter_containers(doc):
    """Yield container dicts from a Pod-spec-bearing manifest."""
    spec = doc.get("spec")
    if not isinstance(spec, dict):
        return
    # Pod template (Deployments, StatefulSets, etc.)
    template = spec.get("template")
    if isinstance(template, dict):
        pod_spec = template.get("spec")
        if isinstance(pod_spec, dict):
            for key in ("containers", "initContainers"):
                for container in pod_spec.get(key) or []:
                    if isinstance(container, dict):
                        yield container
    # Bare Pod
    for key in ("containers", "initContainers"):
        for container in spec.get(key) or []:
            if isinstance(container, dict):
                yield container


def _iter_secret_keys(doc):
    """Yield secret data/stringData keys from a Secret manifest."""
    for field in ("data", "stringData"):
        data = doc.get(field)
        if isinstance(data, dict):
            for key in data:
                yield key


def validate_sidecar_image(doc, rel, idx):
    """Ensure no manifest references the non-existent legacy sidecar image."""
    errors = []
    for container in _iter_containers(doc):
        image = container.get("image")
        if not isinstance(image, str):
            continue
        if image.startswith(LEGACY_SIDECAR_IMAGE):
            errors.append(
                f"{rel}: document {idx} container '{container.get('name', '?')}' "
                f"uses non-existent image '{image}'; use "
                f"'{VALID_SIDECAR_IMAGE_PREFIX}:{{version}}' or the operator image"
            )
    return errors


def validate_seed_secret_keys(doc, rel, idx):
    """Warn when a Secret only exposes the legacy SEED key (issue #1554)."""
    keys = list(_iter_secret_keys(doc))
    if not keys:
        return []
    if LEGACY_SEED_KEY in keys and PREFERRED_SEED_KEY not in keys:
        # Non-fatal: existing deployments must keep working during migration.
        print(
            f"  [WARN] {rel}: document {idx} uses legacy secret key "
            f"'{LEGACY_SEED_KEY}'; migrate to '{PREFERRED_SEED_KEY}' "
            f"(see scripts/migrate-seed-key.sh)"
        )
    return []


def validate_example_conventions(filepath, root_dir):
    """Check example manifests for image and seed-key conventions."""
    rel = os.path.relpath(filepath, root_dir).replace("\\", "/")
    if not rel.startswith("examples/"):
        return []

    errors = []
    try:
        with open(filepath, "r", encoding="utf-8") as f:
            docs = list(yaml.safe_load_all(f))
    except yaml.YAMLError:
        # YAML syntax errors are reported by validate_yaml_structure.
        return []

    for idx, doc in enumerate(docs):
        if not isinstance(doc, dict):
            continue
        errors.extend(validate_sidecar_image(doc, rel, idx))
        errors.extend(validate_seed_secret_keys(doc, rel, idx))
    return errors


def collect_manifest_files(root_dir):
    """Gather all static manifest files (excluding raw Helm templates)."""
    patterns = [
        # CRDs and standard resources
        os.path.join(root_dir, "config", "crd", "*.yaml"),
        os.path.join(root_dir, "config", "cert-manager", "*.yaml"),
        os.path.join(root_dir, "config", "chaos-drills", "*.yaml"),
        # Top-level config manifests
        os.path.join(root_dir, "config", "*.yaml"),
        # Gatekeeper constraints and templates
        os.path.join(root_dir, "config", "manifests", "gatekeeper", "*.yaml"),
        # OLM ClusterServiceVersion base (issue #1364: this is a real
        # apiVersion/kind manifest — operators.coreos.com/v1alpha1
        # ClusterServiceVersion — that kustomization.yaml assembles into the
        # bundle, but it lived outside every validated path: bundle/ only
        # ships metadata/annotations.yaml (excluded, non-manifest), so the
        # actual CSV source was never kubeconform-checked.
        os.path.join(root_dir, "config", "manifests", "bases", "*.yaml"),
        # CR samples and example CRs
        os.path.join(root_dir, "config", "samples", "*.yaml"),
        # Example manifests (mix of CRs and standard resources)
        os.path.join(root_dir, "examples", "*.yaml"),
        # Pre-rendered Helm output (already valid YAML, no Go syntax)
        os.path.join(root_dir, "charts", "stellar-operator", "rendered", "*.yaml"),
        # OLM bundle
        os.path.join(root_dir, "bundle", "**", "*.yaml"),
    ]

    manifest_files = []
    for pattern in patterns:
        manifest_files.extend(glob.glob(pattern, recursive=True))

    # Deduplicate and sort
    return sorted(set(manifest_files))


def filter_k8s_manifests(manifest_files, root_dir):
    """Filter to only files that are likely Kubernetes manifests."""
    return [f for f in manifest_files if is_k8s_manifest(f, root_dir)]


def validate_with_kubeconform(manifest_files, root_dir):
    """Run kubeconform against manifest files with local CRD schemas.

    Uses -ignore-missing-schemas because:
      - CRDs (apiextensions.k8s.io/v1) are validated structurally, not via schema
      - Gatekeeper ConstraintTemplates/Constraints need gatekeeper's own schema set
      - cert-manager, Prometheus, and other third-party CRDs are external
      - Custom resources (StellarNode, etc.) use locally generated schemas
        via the -schema-location flag; unknown kinds are skipped gracefully
    """
    schema_flags = [
        "-strict",
        "-summary",
        "-ignore-missing-schemas",
        "-kubernetes-version", KUBERNETES_VERSION,
        "-schema-location", "default",
        "-schema-location", os.path.join(
            root_dir, "schemas", "crd",
            "{{ .ResourceKind }}{{ .KindSuffix }}"
        ),
    ]

    cmd = ["kubeconform"] + schema_flags + manifest_files
    print(f"-> Running kubeconform on {len(manifest_files)} files...")
    res = subprocess.run(cmd, capture_output=True, text=True)
    print(res.stdout)
    if res.stderr:
        print(res.stderr, file=sys.stderr)
    return res.returncode == 0


def validate_rendered_helm_with_kubeconform(root_dir):
    """Validate Helm-rendered manifests with kubeconform."""
    rendered_dir = os.path.join(root_dir, "charts", "stellar-operator", "rendered")
    if not os.path.isdir(rendered_dir):
        print("[WARN] No rendered Helm manifests found, skipping Helm validation")
        return True

    rendered_files = sorted(glob.glob(os.path.join(rendered_dir, "*.yaml")))
    if not rendered_files:
        print("[WARN] No rendered Helm files found, skipping")
        return True

    schema_flags = [
        "-strict",
        "-summary",
        "-ignore-missing-schemas",
        "-kubernetes-version", KUBERNETES_VERSION,
        "-schema-location", "default",
        "-schema-location", os.path.join(
            root_dir, "schemas", "crd",
            "{{ .ResourceKind }}{{ .KindSuffix }}"
        ),
    ]

    cmd = ["kubeconform"] + schema_flags + rendered_files
    print(f"-> Validating {len(rendered_files)} Helm-rendered manifests...")
    res = subprocess.run(cmd, capture_output=True, text=True)
    print(res.stdout)
    if res.stderr:
        print(res.stderr, file=sys.stderr)
    return res.returncode == 0


def main():
    root_dir = find_project_root()
    print("-" * 60)
    print("Kubernetes Manifest & CRD Schema Validation (issue #1394)")
    print("-" * 60)
    print(f"  K8s version:     {KUBERNETES_VERSION}")
    print(f"  CRD schemas:     schemas/crd/")
    print()

    # 1. Check dependencies
    if not ensure_kubeconform():
        print("ERROR: kubeconform is required but not available", file=sys.stderr)
        sys.exit(1)

    # 2. Collect manifest files
    manifest_files = collect_manifest_files(root_dir)
    print(f"Found {len(manifest_files)} manifest files to validate.\n")

    all_errors = []

    # 3. YAML structure validation
    print("-" * 60)
    print("Phase 1: YAML structure validation")
    print("-" * 60)
    for filepath in manifest_files:
        rel_path = os.path.relpath(filepath, root_dir)
        errs = validate_yaml_structure(filepath, root_dir)
        if errs:
            all_errors.extend(errs)
            print(f"  [FAIL] {rel_path}")
            for err in errs:
                print(f"      {err}")
        else:
            print(f"  [OK] {rel_path}")

    # 3b. Example convention checks (issues #1552, #1554)
    print()
    print("-" * 60)
    print("Phase 1b: Example image & seed-key conventions")
    print("-" * 60)
    for filepath in manifest_files:
        rel_path = os.path.relpath(filepath, root_dir)
        if not rel_path.replace("\\", "/").startswith("examples/"):
            continue
        errs = validate_example_conventions(filepath, root_dir)
        if errs:
            all_errors.extend(errs)
            print(f"  [FAIL] {rel_path}")
            for err in errs:
                print(f"      {err}")
        else:
            print(f"  [OK] {rel_path}")

    # 4. kubeconform schema validation (static manifests)
    k8s_manifests = filter_k8s_manifests(manifest_files, root_dir)
    print()
    print("-" * 60)
    print(f"Phase 2: kubeconform schema validation ({len(k8s_manifests)} K8s manifests)")
    print("-" * 60)
    if k8s_manifests:
        if not validate_with_kubeconform(k8s_manifests, root_dir):
            all_errors.append("kubeconform schema validation failed on static manifests")

    # 5. kubeconform schema validation (Helm-rendered)
    print()
    print("-" * 60)
    print("Phase 3: kubeconform schema validation (Helm-rendered)")
    print("-" * 60)
    if not validate_rendered_helm_with_kubeconform(root_dir):
        all_errors.append("kubeconform schema validation failed on Helm-rendered manifests")

    # 6. Report
    print()
    print("=" * 60)
    if all_errors:
        print("VALIDATION FAILED:")
        for err in all_errors:
            print(f"  - {err}")
        sys.exit(1)
    else:
        print("VALIDATION PASSED: All Kubernetes manifests and CRD schemas passed.")
        sys.exit(0)


if __name__ == "__main__":
    main()
