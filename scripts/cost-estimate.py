#!/usr/bin/env python3
"""
cost-estimate.py - Monthly cost estimator for Stellar-K8s deployments (issue #1834).

Reads the pricing model in ``config/cost-estimate.yaml`` and prints a monthly
estimate per deployment profile. The same model backs the reference tables in
``docs/cost-estimate.md``, so the documentation and the tooling cannot drift
apart silently.

Usage:
    python3 scripts/cost-estimate.py                       # every profile
    python3 scripts/cost-estimate.py --profile mainnet-ha  # one profile
    python3 scripts/cost-estimate.py --discount reserved1y # apply a discount
    python3 scripts/cost-estimate.py --format json         # machine readable
    python3 scripts/cost-estimate.py --check               # validate only

Exit codes:
    0  estimate printed (or configuration valid)
    1  configuration error
    2  bad invocation
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

try:
    import yaml
except ImportError:  # pragma: no cover
    sys.exit("PyYAML is required: pip install pyyaml")

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CONFIG = REPO_ROOT / "config" / "cost-estimate.yaml"

EXIT_OK = 0
EXIT_CONFIG_ERROR = 1
EXIT_USAGE = 2


class ConfigError(Exception):
    """Raised when the cost model is missing data or holds invalid values."""


# ---------------------------------------------------------------------------
# Loading and validation
# ---------------------------------------------------------------------------


def load_config(path: Path) -> dict[str, Any]:
    """Load the cost model from ``path``."""
    if not path.is_file():
        raise ConfigError(f"cost model not found: {path}")
    try:
        with path.open("r", encoding="utf-8") as handle:
            data = yaml.safe_load(handle)
    except yaml.YAMLError as exc:
        raise ConfigError(f"failed to parse {path}: {exc}") from exc
    if not isinstance(data, dict):
        raise ConfigError(f"{path} must contain a YAML mapping at the top level")
    return data


def _require_number(container: dict[str, Any], key: str, where: str,
                    minimum: float = 0.0) -> float:
    """Return ``container[key]`` as a float, or raise :class:`ConfigError`."""
    if key not in container:
        raise ConfigError(f"{where}: missing required key '{key}'")
    value = container[key]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ConfigError(f"{where}: '{key}' must be a number, got {value!r}")
    if value < minimum:
        raise ConfigError(f"{where}: '{key}' must be >= {minimum}, got {value!r}")
    return float(value)


def _require_str(container: dict[str, Any], key: str, where: str) -> str:
    """Return a non-empty string field, or raise :class:`ConfigError`."""
    if key not in container:
        raise ConfigError(f"{where}: missing required key '{key}'")
    value = container[key]
    if not isinstance(value, str) or not value.strip():
        raise ConfigError(f"{where}: '{key}' must be a non-empty string")
    return value.strip()


def validate_config(data: dict[str, Any]) -> list[str]:
    """Return the list of problems found in the cost model (empty when valid)."""
    errors: list[str] = []

    try:
        _require_str(data, "region", "cost model")
        _require_str(data, "currency", "cost model")
        _require_number(data, "hoursPerMonth", "cost model", minimum=1)
    except ConfigError as exc:
        errors.append(str(exc))

    pricing = data.get("pricing")
    if not isinstance(pricing, dict):
        errors.append("cost model: missing 'pricing' mapping")
    else:
        for key in ("storageGbMonth", "egressGb"):
            try:
                _require_number(pricing, key, "pricing")
            except ConfigError as exc:
                errors.append(str(exc))

    discounts = data.get("discounts", {})
    if not isinstance(discounts, dict):
        errors.append("cost model: 'discounts' must be a mapping")
    else:
        for name, value in discounts.items():
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                errors.append(f"discounts: '{name}' must be a number, got {value!r}")
            elif not 0 <= float(value) <= 1:
                errors.append(
                    f"discounts: '{name}' must be a fraction between 0 and 1, got {value!r}"
                )

    profiles = data.get("profiles")
    if not isinstance(profiles, dict) or not profiles:
        errors.append("cost model: 'profiles' must be a non-empty mapping")
        return errors

    for profile_name, profile in profiles.items():
        where = f"profile '{profile_name}'"
        if not isinstance(profile, dict):
            errors.append(f"{where}: must be a mapping")
            continue
        try:
            _require_str(profile, "description", where)
        except ConfigError as exc:
            errors.append(str(exc))
        components = profile.get("components")
        if not isinstance(components, list) or not components:
            errors.append(f"{where}: 'components' must be a non-empty list")
            continue
        for index, component in enumerate(components):
            component_where = f"{where} component #{index + 1}"
            if not isinstance(component, dict):
                errors.append(f"{component_where}: must be a mapping")
                continue
            try:
                _require_str(component, "name", component_where)
                _require_str(component, "instance", component_where)
                count = _require_number(component, "count", component_where, minimum=1)
                if count != int(count):
                    errors.append(f"{component_where}: 'count' must be a whole number")
                _require_number(component, "hourly", component_where)
                _require_number(component, "storageGb", component_where)
            except ConfigError as exc:
                errors.append(str(exc))

    return errors


# ---------------------------------------------------------------------------
# Estimation
# ---------------------------------------------------------------------------


def estimate_profile(profile: dict[str, Any], hours: float,
                     storage_price: float) -> dict[str, Any]:
    """Return the per-component and total monthly cost for one profile."""
    components: list[dict[str, Any]] = []
    compute_total = 0.0
    storage_total = 0.0

    for component in profile.get("components", []):
        count = float(component["count"])
        compute = float(component["hourly"]) * hours * count
        storage = float(component["storageGb"]) * storage_price * count
        compute_total += compute
        storage_total += storage
        components.append(
            {
                "name": component["name"],
                "count": int(count),
                "instance": component["instance"],
                "compute": round(compute, 2),
                "storage": round(storage, 2),
                "total": round(compute + storage, 2),
            }
        )

    return {
        "description": profile.get("description", ""),
        "components": components,
        "compute": round(compute_total, 2),
        "storage": round(storage_total, 2),
        "total": round(compute_total + storage_total, 2),
    }


def _money(value: float) -> str:
    """Format a monetary amount with two decimals."""
    return f"${value:,.2f}"


def render_table(data: dict[str, Any], estimates: dict[str, dict[str, Any]],
                 discount_name: str | None) -> str:
    """Render the estimates as a plain-text table."""
    currency = data.get("currency", "USD")
    lines: list[str] = []
    lines.append(f"Stellar-K8s cost estimate - region {data.get('region', 'unknown')}")
    lines.append(
        f"Prices: on-demand, {data.get('hoursPerMonth', 0)} h/month, "
        f"storage ${data.get('pricing', {}).get('storageGbMonth', 0):.2f}/GB-month"
    )
    if discount_name:
        rate = float(data.get("discounts", {}).get(discount_name, 0))
        lines.append(f"Discount applied: {discount_name} (-{rate * 100:.0f}% on compute)")
    lines.append("")

    for profile_name, estimate in estimates.items():
        lines.append(f"[{profile_name}] {estimate['description']}")
        header = f"  {'Component':<22} {'Qty':>3} {'Instance':<14} {'Compute/mo':>11} {'Storage/mo':>11} {'Total/mo':>11}"
        lines.append(header)
        lines.append("  " + "-" * (len(header) - 2))
        for component in estimate["components"]:
            compute = component["compute"]
            if discount_name:
                rate = float(data.get("discounts", {}).get(discount_name, 0))
                compute = round(compute * (1 - rate), 2)
            lines.append(
                f"  {component['name']:<22} {component['count']:>3} "
                f"{component['instance']:<14} {_money(compute):>11} "
                f"{_money(component['storage']):>11} "
                f"{_money(compute + component['storage']):>11}"
            )
        compute_total = estimate["compute"]
        if discount_name:
            rate = float(data.get("discounts", {}).get(discount_name, 0))
            compute_total = round(compute_total * (1 - rate), 2)
        total = round(compute_total + estimate["storage"], 2)
        lines.append(
            f"  {'TOTAL':<22} {'':>3} {'':<14} {_money(compute_total):>11} "
            f"{_money(estimate['storage']):>11} {_money(total):>11}"
        )
        lines.append("")

    lines.append(f"All amounts are approximate and in {currency}.")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    """Build the command line parser."""
    parser = argparse.ArgumentParser(
        description="Estimate the monthly cost of a Stellar-K8s deployment."
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help=f"cost model to read (default: {DEFAULT_CONFIG.relative_to(REPO_ROOT)})",
    )
    parser.add_argument(
        "--profile",
        action="append",
        default=[],
        help="profile to estimate (repeatable; default: every profile)",
    )
    parser.add_argument(
        "--discount",
        default=None,
        help="apply a discount from the 'discounts' mapping (e.g. reserved1y)",
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
        help="validate the cost model and exit without printing estimates",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Run the estimator."""
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        data = load_config(args.config)
    except ConfigError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    errors = validate_config(data)
    if errors:
        print(f"ERROR: invalid cost model {args.config}:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return EXIT_CONFIG_ERROR

    if args.check:
        print(f"Cost model {args.config} is valid ({len(data['profiles'])} profile(s)).")
        return EXIT_OK

    profiles = data["profiles"]
    selected = args.profile or list(profiles.keys())
    unknown = [name for name in selected if name not in profiles]
    if unknown:
        print(
            f"ERROR: unknown profile(s): {', '.join(unknown)}. "
            f"Available: {', '.join(profiles.keys())}",
            file=sys.stderr,
        )
        return EXIT_USAGE

    if args.discount is not None:
        discounts = data.get("discounts", {})
        if args.discount not in discounts:
            print(
                f"ERROR: unknown discount '{args.discount}'. "
                f"Available: {', '.join(sorted(discounts.keys())) or 'none'}",
                file=sys.stderr,
            )
            return EXIT_USAGE

    hours = float(data["hoursPerMonth"])
    storage_price = float(data["pricing"]["storageGbMonth"])
    estimates = {
        name: estimate_profile(profiles[name], hours, storage_price)
        for name in selected
    }

    if args.format == "json":
        payload = {
            "region": data.get("region"),
            "currency": data.get("currency"),
            "discount": args.discount,
            "profiles": estimates,
        }
        print(json.dumps(payload, indent=2, sort_keys=True))
    else:
        print(render_table(data, estimates, args.discount))

    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
