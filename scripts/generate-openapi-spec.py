#!/usr/bin/env python3
"""Validate and check the operator REST OpenAPI specification."""

import argparse
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    print("Error: PyYAML is required. Install with: pip install pyyaml", file=sys.stderr)
    sys.exit(1)

DEFAULT_SPEC = Path("docs/api/openapi.yaml")

# Routes implemented in src/rest_api/server.rs (public + primary protected paths).
EXPECTED_PATHS = {
    "/health",
    "/healthz",
    "/readyz",
    "/livez",
    "/leader",
    "/api/versions",
    "/api/v1/nodes",
    "/api/v1/nodes/{namespace}/{name}",
    "/v1/health/summary",
    "/v1/health/nodes",
    "/v1/health/incidents",
    "/config/log-level",
    "/api/v1/compliance/report",
    "/api/v1/compliance/status",
    "/api/v1/compliance/regulatory-report",
    "/api/v1/horizon/cache/status",
    "/api/v1/dashboard/overview",
    "/api/v1/dashboard/metrics",
    "/api/v1/analytics/logs",
    "/api/v1/config/analyze",
    "/api/v1/security/posture",
    "/api/v1/capacity/plan",
    "/api/v1/capacity/what-if",
    "/api/v1/optimization/recommendations",
    "/api/v1/optimization/simulate",
    "/api/v1/optimization/forecast",
    "/api/v1/traffic/dashboard",
    "/api/v1/dashboard/nodes/{namespace}/{name}/logs",
    "/api/v1/dashboard/nodes/{namespace}/{name}/conditions",
    "/api/v1/dashboard/nodes/{namespace}/{name}/dr",
    "/api/v1/dashboard/nodes/{namespace}/{name}/metrics",
    "/api/v1/dashboard/nodes/{namespace}/{name}/actions",
    "/api/v1/dashboard/operator/logs",
    "/api/v1/quorum/topology",
    "/api/v1/docs/search-index",
    "/api/v1/jobs",
    "/api/v1/jobs/stats",
    "/api/v1/audit-log",
    "/api/v1/audit-log/search",
    "/api/v1/audit-log/anomalies",
    "/api/v1/debug/pprof/profile",
    "/api/v1/debug/pprof/heap",
    "/metrics",
}


def parse_args():
    parser = argparse.ArgumentParser(description="Validate OpenAPI spec for Stellar-K8s operator API")
    parser.add_argument("--spec", type=Path, default=DEFAULT_SPEC, help="Path to openapi.yaml")
    parser.add_argument(
        "--check",
        action="store_true",
        help="Exit non-zero if required implementation paths are missing from the spec",
    )
    return parser.parse_args()


def load_spec(path: Path) -> dict:
    if not path.is_file():
        raise FileNotFoundError(f"OpenAPI spec not found: {path}")
    with path.open(encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def validate_structure(spec: dict) -> list[str]:
    errors: list[str] = []
    if spec.get("openapi", "").startswith("3.") is False:
        errors.append("openapi version must be 3.x")
    if "info" not in spec:
        errors.append("missing info section")
    if "paths" not in spec or not isinstance(spec["paths"], dict):
        errors.append("missing or invalid paths section")
    return errors


def main() -> int:
    args = parse_args()
    try:
        spec = load_spec(args.spec)
    except (FileNotFoundError, yaml.YAMLError) as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    errors = validate_structure(spec)
    if errors:
        for err in errors:
            print(f"✗ {err}")
        return 1

    documented = set(spec.get("paths", {}).keys())
    missing = sorted(EXPECTED_PATHS - documented)
    extra = sorted(documented - EXPECTED_PATHS)

    print(f"✓ OpenAPI spec is valid: {args.spec}")
    print(f"  Documented paths: {len(documented)}")

    if missing:
        print("✗ Missing paths (implemented in server.rs):")
        for path in missing:
            print(f"    - {path}")

    if extra:
        print("  Additional documented paths (not in core checklist):")
        for path in extra:
            print(f"    + {path}")

    if args.check and missing:
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
