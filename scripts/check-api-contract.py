#!/usr/bin/env python3
"""OpenAPI contract validation, endpoint coverage, and breaking-change detection.

Stellar-K8s — Issue #1288

This script validates the OpenAPI specification against the implemented API,
checks endpoint coverage, and detects breaking changes between spec versions.

Usage:
    python3 scripts/check-api-contract.py --spec docs/api/openapi.yaml --check
    python3 scripts/check-api-contract.py --spec docs/api/openapi.yaml --coverage
    python3 scripts/check-api-contract.py --base base_spec.yaml --head head_spec.yaml --breaking

Exit codes:
    0  All checks passed
    1  Contract violations or breaking changes detected
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:
    sys.exit("PyYAML is required: pip install pyyaml")


REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SPEC = REPO_ROOT / "docs" / "api" / "openapi.yaml"

# ── Known API routes from src/rest_api/server.rs ──────────────────────────────
# This is the canonical set of implemented endpoints.
IMPLEMENTED_ENDPOINTS = {
    # Health probes (public)
    "GET /health": {
        "auth": False,
        "description": "Basic health check",
        "response_schema": "HealthResponse",
    },
    "GET /healthz": {
        "auth": False,
        "description": "Kubernetes liveness-style health probe",
        "response_schema": "ProbeResponse",
    },
    "GET /readyz": {
        "auth": False,
        "description": "Kubernetes readiness probe",
        "response_schema": "ProbeResponse",
    },
    "GET /livez": {
        "auth": False,
        "description": "Kubernetes liveness probe",
        "response_schema": "ProbeResponse",
    },
    # Versioning (public)
    "GET /api/versions": {
        "auth": False,
        "description": "API version catalog",
        "response_schema": "VersionCatalog",
    },
    # Leader
    "GET /leader": {
        "auth": True,
        "description": "Leader election status",
        "response_schema": "LeaderResponse",
    },
    # Nodes
    "GET /api/v1/nodes": {
        "auth": True,
        "description": "List StellarNodes",
        "response_schema": "NodeListResponse",
    },
    "GET /api/v1/nodes/{namespace}/{name}": {
        "auth": True,
        "description": "Get StellarNode",
        "response_schema": "NodeDetailResponse",
    },
    # Health summary (legacy /v1/ prefix)
    "GET /v1/health/summary": {
        "auth": True,
        "description": "Cluster health summary",
    },
    "GET /v1/health/nodes": {
        "auth": True,
        "description": "Per-node health status",
    },
    "GET /v1/health/incidents": {
        "auth": True,
        "description": "Active health incidents",
    },
    # Configuration
    "GET /config/log-level": {
        "auth": True,
        "description": "Get current log level",
        "response_schema": "LogLevelResponse",
    },
    "POST /config/log-level": {
        "auth": True,
        "description": "Set log level",
        "request_schema": "LogLevelRequest",
        "response_schema": "LogLevelResponse",
    },
    # Compliance
    "GET /api/v1/compliance/report": {
        "auth": True,
        "description": "Compliance report",
    },
    "GET /api/v1/compliance/status": {
        "auth": True,
        "description": "Compliance status snapshot",
    },
    "GET /api/v1/compliance/regulatory-report": {
        "auth": True,
        "description": "Regulatory compliance report",
    },
    # Dashboard
    "GET /api/v1/horizon/cache/status": {
        "auth": True,
        "description": "Horizon cache observability",
    },
    "GET /api/v1/dashboard/overview": {
        "auth": True,
        "description": "Dashboard overview",
        "response_schema": "DashboardOverview",
    },
    "GET /api/v1/dashboard/metrics": {
        "auth": True,
        "description": "Dashboard metrics bundle",
    },
    "GET /api/v1/analytics/logs": {
        "auth": True,
        "description": "Log analytics summary",
        "response_schema": "LogAnalyticsResponse",
    },
    "POST /api/v1/config/analyze": {
        "auth": True,
        "description": "Analyze configuration impact",
        "request_schema": "StellarNodeSpec",
        "response_schema": "ConfigImpactResponse",
    },
    "GET /api/v1/security/posture": {
        "auth": True,
        "description": "Security posture assessment",
        "response_schema": "SecurityPostureResponse",
    },
    "GET /api/v1/capacity/plan": {
        "auth": True,
        "description": "Capacity planning recommendations",
        "response_schema": "CapacityPlanningResponse",
    },
    "POST /api/v1/capacity/what-if": {
        "auth": True,
        "description": "Run what-if capacity scenario",
        "request_schema": "WhatIfRequest",
    },
    # Optimization
    "GET /api/v1/optimization/recommendations": {
        "auth": True,
        "description": "Resource optimization recommendations",
    },
    "POST /api/v1/optimization/simulate": {
        "auth": True,
        "description": "Simulate optimization changes",
    },
    "GET /api/v1/optimization/forecast": {
        "auth": True,
        "description": "Resource optimization forecast",
    },
    # Traffic
    "GET /api/v1/traffic/dashboard": {
        "auth": True,
        "description": "Traffic dashboard data",
    },
    # Dashboard node-specific
    "GET /api/v1/dashboard/nodes/{namespace}/{name}/logs": {
        "auth": True,
        "description": "Get node logs",
        "response_schema": "NodeLogsResponse",
    },
    "GET /api/v1/dashboard/nodes/{namespace}/{name}/conditions": {
        "auth": True,
        "description": "Get node conditions",
        "response_schema": "NodeConditionsResponse",
    },
    "GET /api/v1/dashboard/nodes/{namespace}/{name}/dr": {
        "auth": True,
        "description": "Get node DR status",
        "response_schema": "DRStatusResponse",
    },
    "GET /api/v1/dashboard/nodes/{namespace}/{name}/metrics": {
        "auth": True,
        "description": "Get node metrics",
        "response_schema": "MetricsSummary",
    },
    "POST /api/v1/dashboard/nodes/{namespace}/{name}/actions": {
        "auth": True,
        "description": "Execute node action",
        "request_schema": "NodeActionRequest",
        "response_schema": "NodeActionResponse",
    },
    # Operator logs
    "GET /api/v1/dashboard/operator/logs": {
        "auth": True,
        "description": "Get operator logs",
        "response_schema": "OperatorLogsResponse",
    },
    # Quorum
    "GET /api/v1/quorum/topology": {
        "auth": True,
        "description": "SCP quorum topology snapshot",
    },
    # Docs
    "GET /api/v1/docs/search-index": {
        "auth": False,
        "description": "Documentation search index",
    },
    # Jobs
    "GET /api/v1/jobs": {
        "auth": True,
        "description": "List background jobs",
        "response_schema": "JobListResponse",
    },
    "GET /api/v1/jobs/stats": {
        "auth": True,
        "description": "Background job statistics",
        "response_schema": "JobStatsResponse",
    },
    # Audit
    "GET /api/v1/audit-log": {
        "auth": True,
        "description": "List audit log entries",
        "response_schema": "AuditLogResponse",
    },
    "GET /api/v1/audit-log/search": {
        "auth": True,
        "description": "Search audit log entries",
        "response_schema": "AuditLogResponse",
    },
    "GET /api/v1/audit-log/anomalies": {
        "auth": True,
        "description": "List audit anomalies",
        "response_schema": "AuditAnomalyResponse",
    },
    # Metrics
    "GET /metrics": {
        "auth": False,
        "description": "Prometheus metrics",
    },
}


@dataclass
class EndpointCoverage:
    """Coverage information for one endpoint."""

    method: str
    path: str
    documented: bool
    has_request_schema: bool = False
    has_response_schema: bool = False
    has_error_responses: bool = False
    has_auth: bool = False
    implements_auth_requirement: bool = False
    description: str = ""

    @property
    def coverage_score(self) -> float:
        """Score 0-1 based on documentation completeness."""
        checks = [
            self.documented,
            self.has_response_schema,
            self.has_error_responses,
            self.has_auth == self.implements_auth_requirement,
        ]
        return sum(checks) / len(checks)

    @property
    def is_fully_covered(self) -> bool:
        return self.coverage_score >= 0.75


@dataclass
class BreakingChange:
    """A detected breaking API change."""

    category: str
    endpoint: str
    description: str
    severity: str = "breaking"

    def __str__(self) -> str:
        return f"[{self.severity.upper()}] {self.category}: {self.endpoint} — {self.description}"


def load_spec(path: Path) -> dict[str, Any]:
    """Load an OpenAPI specification file."""
    if not path.is_file():
        raise SystemExit(f"Spec file not found: {path}")
    with path.open(encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def extract_spec_endpoints(spec: dict) -> dict[str, dict]:
    """Extract all endpoints from an OpenAPI spec as method+path keys."""
    endpoints = {}
    for path, path_item in spec.get("paths", {}).items():
        if not isinstance(path_item, dict):
            continue
        for method in ("get", "post", "put", "patch", "delete", "head", "options"):
            if method in path_item:
                key = f"{method.upper()} {path}"
                operation = path_item[method]
                endpoints[key] = {
                    "summary": operation.get("summary", ""),
                    "description": operation.get("description", ""),
                    "operationId": operation.get("operationId", ""),
                    "security": operation.get("security", []),
                    "parameters": operation.get("parameters", []),
                    "requestBody": operation.get("requestBody"),
                    "responses": operation.get("responses", {}),
                    "tags": operation.get("tags", []),
                }
    return endpoints


def check_contract(spec: dict) -> list[str]:
    """Validate that the spec covers all implemented endpoints."""
    errors = []
    spec_endpoints = extract_spec_endpoints(spec)

    for endpoint_key, impl_info in IMPLEMENTED_ENDPOINTS.items():
        if endpoint_key not in spec_endpoints:
            errors.append(f"Missing from spec: {endpoint_key} ({impl_info['description']})")
            continue

        spec_ep = spec_endpoints[endpoint_key]

        # Check response schemas
        if "200" not in spec_ep["responses"]:
            errors.append(f"{endpoint_key}: missing 200 response definition")
        elif "content" not in spec_ep["responses"]["200"]:
            errors.append(f"{endpoint_key}: 200 response has no content schema")

        # Check auth requirement
        has_security = bool(spec_ep["security"])
        if impl_info.get("auth") and not has_security:
            errors.append(f"{endpoint_key}: requires auth but spec has no security requirement")

        # Check error responses for protected endpoints
        if impl_info.get("auth"):
            responses = spec_ep["responses"]
            if "401" not in responses:
                errors.append(f"{endpoint_key}: protected endpoint missing 401 response")

    return errors


def compute_coverage(spec: dict) -> list[EndpointCoverage]:
    """Compute endpoint coverage metrics."""
    spec_endpoints = extract_spec_endpoints(spec)
    coverage = []

    for endpoint_key, impl_info in IMPLEMENTED_ENDPOINTS.items():
        method, path = endpoint_key.split(" ", 1)
        documented = endpoint_key in spec_endpoints

        cov = EndpointCoverage(
            method=method,
            path=path,
            documented=documented,
            description=impl_info.get("description", ""),
            implements_auth_requirement=impl_info.get("auth", False),
        )

        if documented:
            spec_ep = spec_endpoints[endpoint_key]
            cov.has_response_schema = "200" in spec_ep["responses"] and bool(
                spec_ep["responses"]["200"].get("content")
            )
            cov.has_error_responses = any(
                code.startswith("4") or code.startswith("5")
                for code in spec_ep["responses"]
            )
            cov.has_auth = bool(spec_ep["security"])

        coverage.append(cov)

    return coverage


def detect_breaking_changes(base_spec: dict, head_spec: dict) -> list[BreakingChange]:
    """Detect breaking API changes between two spec versions."""
    breaking: list[BreakingChange] = []
    base_endpoints = extract_spec_endpoints(base_spec)
    head_endpoints = extract_spec_endpoints(head_spec)

    # Check for removed endpoints
    for endpoint in base_endpoints:
        if endpoint not in head_endpoints:
            breaking.append(
                BreakingChange(
                    category="removed-endpoint",
                    endpoint=endpoint,
                    description="Endpoint removed from API specification",
                )
            )

    # Check for breaking changes in shared endpoints
    for endpoint in set(base_endpoints) & set(head_endpoints):
        base_ep = base_endpoints[endpoint]
        head_ep = head_endpoints[endpoint]

        # Check for removed required fields in request body
        base_request = base_ep.get("requestBody", {})
        head_request = head_ep.get("requestBody", {})
        if base_request and head_request:
            base_schema = _extract_request_schema(base_request)
            head_schema = _extract_request_schema(head_request)
            if base_schema and head_schema:
                base_required = set(base_schema.get("required", []))
                head_required = set(head_schema.get("required", []))
                new_required = head_required - base_required
                # Also check if previously optional fields were removed from head
                base_props = set(base_schema.get("properties", {}).keys())
                head_props = set(head_schema.get("properties", {}).keys())
                removed_props = base_props - head_props
                if removed_props:
                    breaking.append(
                        BreakingChange(
                            category="removed-request-field",
                            endpoint=endpoint,
                            description=f"Request schema removed fields: {', '.join(sorted(removed_props))}",
                        )
                    )

        # Check for removed response fields
        for status_code in ("200", "201"):
            base_resp = base_ep["responses"].get(status_code, {})
            head_resp = head_ep["responses"].get(status_code, {})
            base_resp_schema = _extract_response_schema(base_resp)
            head_resp_schema = _extract_response_schema(head_resp)
            if base_resp_schema and head_resp_schema:
                base_resp_props = set(base_resp_schema.get("properties", {}).keys())
                head_resp_props = set(head_resp_schema.get("properties", {}).keys())
                removed_resp = base_resp_props - head_resp_props
                if removed_resp:
                    breaking.append(
                        BreakingChange(
                            category="removed-response-field",
                            endpoint=endpoint,
                            description=f"Response {status_code} removed fields: {', '.join(sorted(removed_resp))}",
                        )
                    )

        # Check for newly added required request fields
        base_request = base_ep.get("requestBody", {})
        head_request = head_ep.get("requestBody", {})
        if base_request and head_request:
            base_schema = _extract_request_schema(base_request)
            head_schema = _extract_request_schema(head_request)
            if base_schema and head_schema:
                base_required = set(base_schema.get("required", []))
                head_required = set(head_schema.get("required", []))
                newly_required = head_required - base_required
                if newly_required:
                    breaking.append(
                        BreakingChange(
                            category="newly-required-field",
                            endpoint=endpoint,
                            description=f"New required request fields: {', '.join(sorted(newly_required))}",
                        )
                    )

        # Check for auth requirement changes
        base_auth = bool(base_ep["security"])
        head_auth = bool(head_ep["security"])
        if base_auth and not head_auth:
            breaking.append(
                BreakingChange(
                    category="auth-removed",
                    endpoint=endpoint,
                    description="Authentication requirement removed from previously protected endpoint",
                )
            )

        # Check for status code changes
        base_codes = set(base_ep["responses"].keys())
        head_codes = set(head_ep["responses"].keys())
        removed_codes = base_codes - head_codes
        for code in removed_codes:
            if code.startswith("4") or code.startswith("5"):
                breaking.append(
                    BreakingChange(
                        category="removed-status-code",
                        endpoint=endpoint,
                        description=f"Error response {code} removed",
                    )
                )

    return breaking


def _extract_request_schema(request_body: dict) -> dict | None:
    """Extract JSON schema from a requestBody definition."""
    content = request_body.get("content", {})
    json_content = content.get("application/json", {})
    return json_content.get("schema")


def _extract_response_schema(response: dict) -> dict | None:
    """Extract JSON schema from a response definition."""
    content = response.get("content", {})
    json_content = content.get("application/json", {})
    return json_content.get("schema")


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="OpenAPI contract validation for Stellar-K8s"
    )
    sub = parser.add_subparsers(dest="command")

    # Check command
    check_p = sub.add_parser("check", help="Validate spec covers implemented endpoints")
    check_p.add_argument("--spec", type=Path, default=DEFAULT_SPEC)

    # Coverage command
    cov_p = sub.add_parser("coverage", help="Compute endpoint coverage report")
    cov_p.add_argument("--spec", type=Path, default=DEFAULT_SPEC)
    cov_p.add_argument("--format", choices=["text", "json"], default="text")
    cov_p.add_argument("--min-coverage", type=float, default=90.0,
                        help="Minimum coverage percentage to pass (default: 90)")

    # Breaking command
    break_p = sub.add_parser("breaking", help="Detect breaking changes between specs")
    break_p.add_argument("--base", type=Path, required=True, help="Base (before) spec")
    break_p.add_argument("--head", type=Path, required=True, help="Head (after) spec")

    # Legacy --check mode (backward compatible)
    parser.add_argument("--spec", type=Path, default=DEFAULT_SPEC)
    parser.add_argument("--check", action="store_true", help="Validate spec (legacy)")
    parser.add_argument("--coverage", action="store_true", help="Coverage report (legacy)")

    args = parser.parse_args(argv)

    # Handle legacy flags
    if args.check or (not args.command):
        spec = load_spec(args.spec)
        errors = check_contract(spec)
        if errors:
            print(f"Contract check FAILED ({len(errors)} violation(s)):")
            for err in errors:
                print(f"  ✗ {err}")
            return 1
        print(f"✓ OpenAPI contract check passed: {args.spec}")
        print(f"  All {len(IMPLEMENTED_ENDPOINTS)} implemented endpoints are documented")
        return 0

    if args.command == "check":
        spec = load_spec(args.spec)
        errors = check_contract(spec)
        if errors:
            print(f"Contract check FAILED ({len(errors)} violation(s)):")
            for err in errors:
                print(f"  ✗ {err}")
            return 1
        print(f"✓ OpenAPI contract check passed: {args.spec}")
        return 0

    if args.command == "coverage":
        spec = load_spec(args.spec)
        coverage = compute_coverage(spec)
        covered = sum(1 for c in coverage if c.documented)
        fully_covered = sum(1 for c in coverage if c.is_fully_covered)
        total = len(coverage)
        pct = (fully_covered / total * 100) if total else 0

        if args.format == "json":
            result = {
                "total_endpoints": total,
                "documented": covered,
                "fully_covered": fully_covered,
                "coverage_percent": round(pct, 1),
                "threshold": args.min_coverage,
                "passed": pct >= args.min_coverage,
                "endpoints": [
                    {
                        "method": c.method,
                        "path": c.path,
                        "documented": c.documented,
                        "has_response_schema": c.has_response_schema,
                        "has_error_responses": c.has_error_responses,
                        "has_auth": c.has_auth,
                        "implements_auth": c.implements_auth_requirement,
                        "score": round(c.coverage_score, 2),
                    }
                    for c in coverage
                ],
            }
            print(json.dumps(result, indent=2))
        else:
            print(f"\nEndpoint Coverage Report")
            print(f"{'='*60}")
            print(f"Total endpoints:     {total}")
            print(f"Documented:         {covered}/{total}")
            print(f"Fully covered:      {fully_covered}/{total}")
            print(f"Coverage:           {pct:.1f}%")
            print(f"Threshold:          {args.min_coverage}%")
            print(f"{'='*60}")
            print(f"\n{'Method':<8} {'Path':<45} {'Doc':>4} {'Schema':>6} {'Err':>4} {'Auth':>4}")
            print(f"{'-'*8} {'-'*45} {'-'*4} {'-'*6} {'-'*4} {'-'*4}")
            for c in coverage:
                doc = "✓" if c.documented else "✗"
                schema = "✓" if c.has_response_schema else "✗"
                err = "✓" if c.has_error_responses else "✗"
                auth = "✓" if c.has_auth else "·"
                print(f"{c.method:<8} {c.path:<45} {doc:>4} {schema:>6} {err:>4} {auth:>4}")

        if pct < args.min_coverage:
            print(f"\n✗ Coverage {pct:.1f}% is below threshold {args.min_coverage}%")
            return 1
        print(f"\n✓ Coverage {pct:.1f}% meets threshold {args.min_coverage}%")
        return 0

    if args.command == "breaking":
        base_spec = load_spec(args.base)
        head_spec = load_spec(args.head)
        breaking = detect_breaking_changes(base_spec, head_spec)
        if breaking:
            print(f"Breaking changes detected ({len(breaking)}):")
            for change in breaking:
                print(f"  ✗ {change}")
            return 1
        print("✓ No breaking changes detected")
        return 0

    parser.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
