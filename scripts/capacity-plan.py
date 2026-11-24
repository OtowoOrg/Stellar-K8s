#!/usr/bin/env python3
"""
capacity-plan.py - Validate and summarise the capacity planning configuration
(issue #1835).

The capacity forecasting engine publishes ``CapacityRecommendationReport``
resources. The forecast parameters, the forecast dimensions, the operator
sizing table and the ledger growth assumptions live in
``config/capacity-planning.yaml`` so they can be reviewed and validated
without a cluster.

Usage:
    python3 scripts/capacity-plan.py                 # validate and summarise
    python3 scripts/capacity-plan.py --check         # validate only
    python3 scripts/capacity-plan.py --format json   # machine readable

Exit codes:
    0  configuration is valid
    1  configuration error
    2  bad invocation
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("PyYAML is required: pip install pyyaml")

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / "config" / "capacity-planning.yaml"

EXIT_OK = 0
EXIT_CONFIG_ERROR = 1
EXIT_USAGE = 2

KNOWN_DIMENSIONS = ("Cpu", "Memory", "Storage", "ObjectCount")
QUANTITY_RE = re.compile(r"^\d+(\.\d+)?(m|Ki|Mi|Gi|Ti)?$")


class ConfigError(Exception):
    """Raised when the capacity configuration is missing data or invalid."""


# ---------------------------------------------------------------------------
# Loading and validation
# ---------------------------------------------------------------------------


def load_config(path: Path) -> dict[str, Any]:
    """Load the capacity configuration from ``path``."""
    if not path.is_file():
        raise ConfigError(f"capacity configuration not found: {path}")
    try:
        with path.open("r", encoding="utf-8") as handle:
            data = yaml.safe_load(handle)
    except yaml.YAMLError as exc:
        raise ConfigError(f"failed to parse {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise ConfigError(f"{path} must contain a YAML mapping at the top level")
    return data


def _number(container: dict[str, Any], key: str, where: str,
            minimum: float | None = None, maximum: float | None = None) -> float:
    """Return a numeric field, or raise :class:`ConfigError`."""
    if key not in container:
        raise ConfigError(f"{where}: missing required key '{key}'")
    value = container[key]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ConfigError(f"{where}: '{key}' must be a number, got {value!r}")
    if minimum is not None and value < minimum:
        raise ConfigError(f"{where}: '{key}' must be >= {minimum}, got {value!r}")
    if maximum is not None and value > maximum:
        raise ConfigError(f"{where}: '{key}' must be <= {maximum}, got {value!r}")
    return float(value)


def validate_config(data: dict[str, Any]) -> list[str]:
    """Return the list of problems found in the configuration."""
    errors: list[str] = []

    defaults = data.get("defaults")
    if not isinstance(defaults, dict):
        errors.append("capacity configuration: missing 'defaults' mapping")
        return errors

    checks = (
        ("horizonDays", 1, 365),
        ("confidenceLevel", 0.5, 0.999),
        ("minHistoryDays", 7, 3650),
        ("lookbackDays", 7, 3650),
        ("backtestStepDays", 1, 90),
        ("minLeadTimeDays", 0, 365),
        ("seasonalPeriodDays", 2, 365),
        ("seasonalMinImprovement", 0.0, 1.0),
        ("exhaustionThreshold", 0.01, None),
        ("targetUtilization", 0.1, 0.99),
        ("mapeTargetPct", 0.0, 100.0),
    )
    values: dict[str, float] = {}
    for key, minimum, maximum in checks:
        try:
            values[key] = _number(defaults, key, "defaults", minimum, maximum)
        except ConfigError as exc:
            errors.append(str(exc))

    if "minHistoryDays" in values and "lookbackDays" in values:
        if values["lookbackDays"] < values["minHistoryDays"]:
            errors.append(
                "defaults: 'lookbackDays' must be greater than or equal to 'minHistoryDays'"
            )
    if "horizonDays" in values and "minLeadTimeDays" in values:
        if values["minLeadTimeDays"] > values["horizonDays"]:
            errors.append("defaults: 'minLeadTimeDays' must not exceed 'horizonDays'")
    if "lookbackDays" in values and "backtestStepDays" in values:
        if values["backtestStepDays"] > values["lookbackDays"]:
            errors.append("defaults: 'backtestStepDays' must not exceed 'lookbackDays'")

    dimensions = data.get("dimensions")
    if not isinstance(dimensions, list) or not dimensions:
        errors.append("capacity configuration: 'dimensions' must be a non-empty list")
    else:
        for dimension in dimensions:
            if dimension not in KNOWN_DIMENSIONS:
                errors.append(
                    f"dimensions: unknown dimension {dimension!r} "
                    f"(known: {', '.join(KNOWN_DIMENSIONS)})"
                )

    sizing = data.get("sizing")
    if not isinstance(sizing, dict):
        errors.append("capacity configuration: missing 'sizing' mapping")
    else:
        entries = sizing.get("operatorResources")
        if not isinstance(entries, list) or not entries:
            errors.append("sizing: 'operatorResources' must be a non-empty list")
        else:
            previous = 0
            for index, entry in enumerate(entries):
                where = f"sizing.operatorResources[{index}]"
                if not isinstance(entry, dict):
                    errors.append(f"{where}: must be a mapping")
                    continue
                try:
                    max_nodes = _number(entry, "maxNodes", where, minimum=1)
                except ConfigError as exc:
                    errors.append(str(exc))
                    continue
                if max_nodes <= previous:
                    errors.append(f"{where}: 'maxNodes' must increase with every entry")
                previous = max_nodes
                for field in ("cpu", "memory"):
                    value = entry.get(field)
                    if not isinstance(value, str) or not QUANTITY_RE.match(value):
                        errors.append(
                            f"{where}: '{field}' must be a Kubernetes quantity, got {value!r}"
                        )

    growth = data.get("storageGrowth")
    if not isinstance(growth, dict) or not growth:
        errors.append("capacity configuration: missing 'storageGrowth' mapping")
    else:
        for network, values in growth.items():
            where = f"storageGrowth.{network}"
            if not isinstance(values, dict):
                errors.append(f"{where}: must be a mapping")
                continue
            for field in ("initialGi", "sixMonthGi", "twelveMonthGi"):
                try:
                    _number(values, field, where, minimum=1)
                except ConfigError as exc:
                    errors.append(str(exc))
            try:
                initial = float(values["initialGi"])
                six = float(values["sixMonthGi"])
                twelve = float(values["twelveMonthGi"])
            except (KeyError, TypeError, ValueError):
                continue
            if not initial <= six <= twelve:
                errors.append(
                    f"{where}: 'initialGi' <= 'sixMonthGi' <= 'twelveMonthGi' must hold"
                )

    return errors


# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------


def build_summary(data: dict[str, Any]) -> dict[str, Any]:
    """Return a structured summary of the capacity configuration."""
    defaults = data.get("defaults", {})
    return {
        "defaults": defaults,
        "dimensions": data.get("dimensions", []),
        "operatorResources": data.get("sizing", {}).get("operatorResources", []),
        "storageGrowth": data.get("storageGrowth", {}),
        "alerts": {
            "criticalDays": defaults.get("minLeadTimeDays"),
            "warningDays": 30,
            "mapeTargetPct": defaults.get("mapeTargetPct"),
        },
    }


def render_summary(data: dict[str, Any]) -> str:
    """Render the capacity plan as plain text."""
    defaults = data.get("defaults", {})
    lines: list[str] = []
    lines.append("Stellar-K8s capacity plan")
    lines.append("")

    lines.append("Forecast parameters")
    lines.append("  Horizon (days)              : %s" % defaults.get("horizonDays"))
    lines.append("  Confidence level             : %s" % defaults.get("confidenceLevel"))
    lines.append("  Minimum history (days)       : %s" % defaults.get("minHistoryDays"))
    lines.append("  Lookback window (days)       : %s" % defaults.get("lookbackDays"))
    lines.append("  Backtest step (days)         : %s" % defaults.get("backtestStepDays"))
    lines.append("  Minimum lead time (days)     : %s" % defaults.get("minLeadTimeDays"))
    lines.append("  Seasonal period (days)       : %s" % defaults.get("seasonalPeriodDays"))
    lines.append(
        "  Seasonal improvement required : %s" % defaults.get("seasonalMinImprovement")
    )
    lines.append("  Target utilization            : %s" % defaults.get("targetUtilization"))
    lines.append("  MAPE target (%%)              : %s" % defaults.get("mapeTargetPct"))
    lines.append("")

    lines.append("Forecast dimensions")
    for dimension in data.get("dimensions", []):
        lines.append(f"  - {dimension}")
    lines.append("")

    lines.append("Operator sizing by managed StellarNode count")
    lines.append(f"  {'StellarNodes':<22} {'CPU request':<14} {'Memory request':<14}")
    for entry in data.get("sizing", {}).get("operatorResources", []):
        lines.append(
            f"  up to {entry.get('maxNodes'):<15} {entry.get('cpu', ''):<14} "
            f"{entry.get('memory', ''):<14}"
        )
    lines.append("")

    lines.append("Ledger growth (storage sizing)")
    for network, values in data.get("storageGrowth", {}).items():
        lines.append(
            "  %-10s initial %s Gi, 6 months %s Gi, 12 months %s Gi"
            % (
                network,
                values.get("initialGi"),
                values.get("sixMonthGi"),
                values.get("twelveMonthGi"),
            )
        )
    lines.append("")

    lines.append("Alert thresholds")
    lines.append(
        "  Critical alert when exhaustion is predicted within %s days"
        % defaults.get("minLeadTimeDays")
    )
    lines.append("  Warning alert when exhaustion is predicted within 30 days")
    lines.append("")
    lines.append(
        "Apply the alerts from docs/capacity-planning.md to your Prometheus rules."
    )
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    """Build the command line parser."""
    parser = argparse.ArgumentParser(
        description="Validate and summarise the Stellar-K8s capacity plan."
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help=(
            "capacity configuration to read "
            f"(default: {DEFAULT_CONFIG.relative_to(REPO_ROOT)})"
        ),
    )
    parser.add_argument(
        "--format",
        choices=("table", "json"),
        default="table",
        help="output format (default: table)",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate the configuration and exit without printing a summary",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Run the validator."""
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        data = load_config(args.config)
    except ConfigError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    errors = validate_config(data)
    if errors:
        print(f"ERROR: invalid capacity configuration {args.config}:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    if args.check:
        print(f"Capacity configuration {args.config} is valid.")
        return EXIT_OK

    if args.format == "json":
        print(json.dumps(build_summary(data), indent=2, sort_keys=True))
    else:
        print(render_summary(data))

    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
