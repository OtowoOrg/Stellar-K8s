#!/usr/bin/env python3
"""Convert a Docker Compose deployment into starter Stellar-K8s manifests.

This helper is intentionally conservative: it produces a good first draft
for `StellarNode` resources plus supporting Secrets/Namespace objects, and it
prints warnings for Compose settings that must be reviewed manually.
"""

from __future__ import annotations

import argparse
import re
import sys
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Tuple

try:
    import yaml
except ImportError:
    print("Error: PyYAML is required. Install with: pip install pyyaml", file=sys.stderr)
    sys.exit(1)


SUPPORTED_NETWORKS = {"mainnet", "testnet", "futurenet", "custom"}
DATA_PATH_HINTS = (
    "/var/lib/stellar",
    "/opt/stellar",
    "/data",
    "/var/lib/postgresql/data",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Convert a Docker Compose file into starter Stellar-K8s manifests."
    )
    parser.add_argument("--input", required=True, help="Path to docker-compose YAML")
    parser.add_argument(
        "--output",
        required=True,
        help="Path to write the generated Kubernetes YAML",
    )
    parser.add_argument(
        "--namespace",
        default="stellar",
        help="Target Kubernetes namespace for generated objects",
    )
    parser.add_argument(
        "--network",
        default="testnet",
        choices=sorted(SUPPORTED_NETWORKS),
        help="Stellar network to place on generated StellarNode resources",
    )
    parser.add_argument(
        "--storage-class",
        default="standard",
        help="Default storage class used when a service mounts persistent data",
    )
    parser.add_argument(
        "--emit-namespace",
        action="store_true",
        help="Emit a Namespace manifest before generated resources",
    )
    return parser.parse_args()


def load_yaml(path: Path) -> Dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = yaml.safe_load(handle)
    if not isinstance(data, dict):
        raise ValueError("Compose file must parse to a mapping")
    return data


def normalize_env(env: Any) -> Dict[str, str]:
    if env is None:
        return {}
    if isinstance(env, dict):
        return {str(k): "" if v is None else str(v) for k, v in env.items()}
    result: Dict[str, str] = {}
    if isinstance(env, list):
        for item in env:
            if not isinstance(item, str):
                continue
            key, sep, value = item.partition("=")
            result[key] = value if sep else ""
        return result
    return {}


def sanitize_name(value: str) -> str:
    cleaned = re.sub(r"[^a-z0-9-]+", "-", value.lower())
    cleaned = re.sub(r"-{2,}", "-", cleaned).strip("-")
    return cleaned or "stellar-node"


def extract_version(image: str, default: str = "latest") -> str:
    if not image:
        return default
    if "@" in image:
        return image.split("@", 1)[1]
    if ":" in image:
        return image.rsplit(":", 1)[1]
    return default


def infer_node_type(name: str, image: str, env: Dict[str, str]) -> Optional[str]:
    haystack = " ".join([name.lower(), image.lower(), " ".join(env.keys()).lower()])
    if any(token in haystack for token in ("soroban", "rpc")):
        return "SorobanRpc"
    if "horizon" in haystack:
        return "Horizon"
    if any(token in haystack for token in ("stellar-core", "validator", "core")):
        return "Validator"
    return None


def infer_replicas(service: Dict[str, Any]) -> int:
    deploy = service.get("deploy", {})
    if isinstance(deploy, dict):
        replicas = deploy.get("replicas")
        if isinstance(replicas, int) and replicas > 0:
            return replicas
    return 1


def coerce_cpu(value: Any) -> Optional[str]:
    if value in (None, ""):
        return None
    if isinstance(value, str) and value.endswith("m"):
        return value
    try:
        numeric = Decimal(str(value))
    except InvalidOperation:
        return str(value)
    if numeric < 1:
        return f"{int(numeric * 1000)}m"
    if numeric == numeric.to_integral():
        return str(int(numeric))
    return f"{int(numeric * 1000)}m"


def extract_resources(service: Dict[str, Any]) -> Dict[str, Dict[str, str]]:
    deploy = service.get("deploy", {})
    resources = deploy.get("resources", {}) if isinstance(deploy, dict) else {}
    limits = resources.get("limits", {}) if isinstance(resources, dict) else {}
    reservations = resources.get("reservations", {}) if isinstance(resources, dict) else {}

    requests_cpu = coerce_cpu(reservations.get("cpus") or reservations.get("cpu"))
    limits_cpu = coerce_cpu(limits.get("cpus") or limits.get("cpu"))
    requests_mem = reservations.get("memory")
    limits_mem = limits.get("memory")

    if not any((requests_cpu, limits_cpu, requests_mem, limits_mem)):
        return {}

    output: Dict[str, Dict[str, str]] = {"requests": {}, "limits": {}}
    if requests_cpu:
        output["requests"]["cpu"] = requests_cpu
    if requests_mem:
        output["requests"]["memory"] = str(requests_mem)
    if limits_cpu:
        output["limits"]["cpu"] = limits_cpu
    if limits_mem:
        output["limits"]["memory"] = str(limits_mem)

    if not output["requests"]:
        output.pop("requests")
    if not output["limits"]:
        output.pop("limits")
    return output


def normalize_volumes(volumes: Any) -> List[Dict[str, str]]:
    if not isinstance(volumes, list):
        return []
    result: List[Dict[str, str]] = []
    for volume in volumes:
        if isinstance(volume, str):
            parts = volume.split(":")
            if len(parts) >= 2:
                result.append({"source": parts[0], "target": parts[1]})
        elif isinstance(volume, dict):
            source = volume.get("source") or volume.get("src") or volume.get("type", "")
            target = volume.get("target") or volume.get("dst") or volume.get("destination")
            if target:
                result.append({"source": str(source), "target": str(target)})
    return result


def infer_storage(service: Dict[str, Any], storage_class: str) -> Optional[Dict[str, Any]]:
    mounts = normalize_volumes(service.get("volumes"))
    for mount in mounts:
        target = mount.get("target", "")
        if any(hint in target for hint in DATA_PATH_HINTS):
            return {
                "mode": "PersistentVolume",
                "storageClass": storage_class,
                "size": "100Gi",
                "retentionPolicy": "Retain",
            }
    return None


def split_csv(value: str) -> List[str]:
    return [item.strip() for item in value.split(",") if item.strip()]


def make_secret(name: str, namespace: str, string_data: Dict[str, str]) -> Dict[str, Any]:
    return {
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {"name": name, "namespace": namespace},
        "type": "Opaque",
        "stringData": string_data,
    }


def common_spec(service: Dict[str, Any], node_type: str, network: str, storage_class: str) -> Dict[str, Any]:
    image = str(service.get("image", ""))
    spec: Dict[str, Any] = {
        "nodeType": node_type,
        "network": network,
        "version": extract_version(image),
        "topologySpreadConstraints": [],
        "networkPolicy": {"enabled": True},
    }

    resources = extract_resources(service)
    if resources:
        spec["resources"] = resources

    storage = infer_storage(service, storage_class)
    if storage:
        spec["storage"] = storage

    replicas = infer_replicas(service)
    if replicas > 1:
        spec["replicas"] = replicas

    if node_type == "Validator":
        spec["maxUnavailable"] = 0
        spec["minAvailable"] = 1
        spec["podAntiAffinity"] = "Hard"
        spec["alerting"] = True
    else:
        spec["maxUnavailable"] = 1
        spec["minAvailable"] = 1 if replicas == 1 else max(1, replicas - 1)

    return spec


def convert_service(
    service_name: str,
    service: Dict[str, Any],
    services: Dict[str, Dict[str, Any]],
    namespace: str,
    network: str,
    storage_class: str,
) -> Tuple[List[Dict[str, Any]], Optional[Dict[str, Any]], List[str]]:
    env = normalize_env(service.get("environment"))
    image = str(service.get("image", ""))
    node_type = infer_node_type(service_name, image, env)
    notes: List[str] = []
    documents: List[Dict[str, Any]] = []

    if node_type is None:
        notes.append(
            f"Skipped Compose service '{service_name}' because it does not look like a Validator, Horizon, or Soroban RPC node."
        )
        return documents, None, notes

    manifest_name = sanitize_name(service_name)
    spec = common_spec(service, node_type, network, storage_class)

    if service.get("ports"):
        notes.append(
            f"Review service '{service_name}' port publishing manually; the converter does not infer Ingress or LoadBalancer policy from Compose ports."
        )

    if node_type == "Validator":
        validator_secret_name = f"{manifest_name}-seed"
        seed_value = (
            env.get("STELLAR_CORE_SEED")
            or env.get("NODE_SEED")
            or "CHANGE_ME"
        )
        documents.append(
            make_secret(
                validator_secret_name,
                namespace,
                {"STELLAR_CORE_SEED": seed_value},
            )
        )
        validator_config: Dict[str, Any] = {"seedSecretRef": validator_secret_name}
        history_urls = split_csv(
            env.get("HISTORY_ARCHIVE_URLS", "") or env.get("HISTORY_ARCHIVES", "")
        )
        if history_urls:
            validator_config["enableHistoryArchive"] = True
            validator_config["historyArchiveUrls"] = history_urls
        spec["validatorConfig"] = validator_config

    elif node_type == "Horizon":
        db_secret_name = f"{manifest_name}-database"
        db_url = env.get("DATABASE_URL", "")
        if not db_url:
            db_service = next(
                (name for name in services.keys() if any(token in name.lower() for token in ("postgres", "db"))),
                None,
            )
            if db_service:
                db_url = f"postgresql://stellar:CHANGE_ME@{sanitize_name(db_service)}:5432/horizon"
            else:
                db_url = "postgresql://stellar:CHANGE_ME@postgresql:5432/horizon"
                notes.append(
                    f"Compose service '{service_name}' does not define DATABASE_URL; a placeholder connection string was emitted."
                )
        documents.append(make_secret(db_secret_name, namespace, {"DATABASE_URL": db_url}))

        validator_service = next(
            (
                name
                for name, candidate in services.items()
                if infer_node_type(name, str(candidate.get("image", "")), normalize_env(candidate.get("environment")))
                == "Validator"
            ),
            None,
        )
        stellar_core_url = env.get("STELLAR_CORE_URL", "")
        if not stellar_core_url and validator_service:
            stellar_core_url = f"http://{sanitize_name(validator_service)}:11626"
        if not stellar_core_url:
            stellar_core_url = "http://validator:11626"
            notes.append(
                f"Compose service '{service_name}' does not define STELLAR_CORE_URL; a placeholder validator URL was emitted."
            )

        horizon_config: Dict[str, Any] = {
            "databaseSecretRef": db_secret_name,
            "stellarCoreUrl": stellar_core_url,
        }
        enable_ingest = env.get("ENABLE_INGEST") or env.get("HORIZON_ENABLE_INGEST")
        if enable_ingest:
            horizon_config["enableIngest"] = str(enable_ingest).lower() in {"1", "true", "yes"}
        workers = env.get("INGEST_WORKERS")
        if workers and workers.isdigit():
            horizon_config["ingestWorkers"] = int(workers)
        spec["horizonConfig"] = horizon_config
        spec["strategy"] = {"type": "rollingUpdate"}

    elif node_type == "SorobanRpc":
        validator_service = next(
            (
                name
                for name, candidate in services.items()
                if infer_node_type(name, str(candidate.get("image", "")), normalize_env(candidate.get("environment")))
                == "Validator"
            ),
            None,
        )
        stellar_core_url = env.get("STELLAR_CORE_URL", "")
        if not stellar_core_url and validator_service:
            stellar_core_url = f"http://{sanitize_name(validator_service)}:11626"
        if not stellar_core_url:
            stellar_core_url = "http://validator:11626"
            notes.append(
                f"Compose service '{service_name}' does not define STELLAR_CORE_URL; a placeholder validator URL was emitted."
            )

        soroban_config: Dict[str, Any] = {"stellarCoreUrl": stellar_core_url}
        max_events = env.get("MAX_EVENTS_PER_REQUEST")
        if max_events and max_events.isdigit():
            soroban_config["maxEventsPerRequest"] = int(max_events)
        spec["sorobanConfig"] = soroban_config

    node = {
        "apiVersion": "stellar.org/v1alpha1",
        "kind": "StellarNode",
        "metadata": {
            "name": manifest_name,
            "namespace": namespace,
            "annotations": {
                "migration.stellar.org/source-service": service_name,
                "migration.stellar.org/source-format": "docker-compose",
            },
        },
        "spec": spec,
    }
    return documents, node, notes


def build_namespace(name: str, network: str) -> Dict[str, Any]:
    return {
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": {
            "name": name,
            "labels": {"stellar.org/network": network},
        },
    }


def generate_documents(compose: Dict[str, Any], args: argparse.Namespace) -> Tuple[List[Dict[str, Any]], List[str]]:
    services = compose.get("services")
    if not isinstance(services, dict) or not services:
        raise ValueError("Compose file must contain a non-empty 'services' mapping")

    documents: List[Dict[str, Any]] = []
    notes: List[str] = []

    if args.emit_namespace:
        documents.append(build_namespace(args.namespace, args.network))

    for service_name, service in services.items():
        if not isinstance(service, dict):
            continue
        extras, node, service_notes = convert_service(
            service_name=service_name,
            service=service,
            services=services,
            namespace=args.namespace,
            network=args.network,
            storage_class=args.storage_class,
        )
        documents.extend(extras)
        if node is not None:
            documents.append(node)
        notes.extend(service_notes)

        if "depends_on" in service:
            notes.append(
                f"Compose service '{service_name}' uses depends_on; Kubernetes readiness and service discovery replace startup ordering and must be reviewed manually."
            )
        if service.get("network_mode") == "host":
            notes.append(
                f"Compose service '{service_name}' uses host networking, which is not converted automatically."
            )

    return documents, notes


def dump_yaml(documents: Iterable[Dict[str, Any]]) -> str:
    return yaml.safe_dump_all(
        list(documents),
        sort_keys=False,
        default_flow_style=False,
    )


def main() -> None:
    args = parse_args()
    input_path = Path(args.input)
    output_path = Path(args.output)

    if not input_path.exists():
        print(f"Error: compose file not found: {input_path}", file=sys.stderr)
        sys.exit(1)

    try:
        compose = load_yaml(input_path)
        documents, notes = generate_documents(compose, args)
    except Exception as exc:  # pragma: no cover - defensive CLI handling
        print(f"Error: {exc}", file=sys.stderr)
        sys.exit(1)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(dump_yaml(documents), encoding="utf-8")

    print(f"Generated {len(documents)} manifest(s) at {output_path}")
    if notes:
        print("\nManual review notes:", file=sys.stderr)
        for note in notes:
            print(f"- {note}", file=sys.stderr)


if __name__ == "__main__":
    main()
