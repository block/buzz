#!/usr/bin/env python3
"""Build a checksum-addressed inventory of Command Adviser offline payloads."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import sys
from typing import Callable, NamedTuple, Sequence


SCHEMA_VERSION = 1
REQUIRED_ROLES = frozenset(
    {
        "app",
        "model",
        "embedding_model",
        "rag_snapshot",
        "memory_backup",
        "relay",
        "recovery",
    }
)
SAFE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


class ComponentInput(NamedTuple):
    role: str
    name: str
    path: Path


class ProtectedConfigInput(NamedTuple):
    name: str
    path: Path


class PathInventory(NamedTuple):
    kind: str
    size_bytes: int
    sha256: str


def _hash_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def inventory_path(path: Path) -> PathInventory:
    """Return a deterministic content inventory and reject symbolic links."""
    path = path.expanduser()
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"component path does not exist: {path}") from error
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"component path must not be a symlink: {path}")
    if stat.S_ISREG(metadata.st_mode):
        return PathInventory("file", metadata.st_size, _hash_file(path))
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError(f"component path must be a regular file or directory: {path}")

    entries: list[tuple[str, int, str]] = []
    total = 0
    for root, directories, files in os.walk(path, followlinks=False):
        root_path = Path(root)
        for name in directories:
            candidate = root_path / name
            if candidate.is_symlink():
                raise ValueError(f"component directory contains a symlink: {candidate}")
        for name in files:
            candidate = root_path / name
            candidate_metadata = candidate.lstat()
            if stat.S_ISLNK(candidate_metadata.st_mode):
                raise ValueError(f"component directory contains a symlink: {candidate}")
            if not stat.S_ISREG(candidate_metadata.st_mode):
                raise ValueError(
                    f"component directory contains a non-regular file: {candidate}"
                )
            relative = candidate.relative_to(path).as_posix()
            file_hash = _hash_file(candidate)
            entries.append((relative, candidate_metadata.st_size, file_hash))
            total += candidate_metadata.st_size

    digest = hashlib.sha256()
    for relative, size, file_hash in sorted(entries):
        digest.update(f"F\0{relative}\0{size}\0{file_hash}\n".encode("utf-8"))
    return PathInventory("directory", total, digest.hexdigest())


def _component_record(component: ComponentInput) -> dict:
    if component.role not in REQUIRED_ROLES:
        raise ValueError(f"unknown component role: {component.role}")
    if not SAFE_NAME.fullmatch(component.name):
        raise ValueError(f"invalid component name: {component.name}")
    inventory = inventory_path(component.path)
    return {
        "role": component.role,
        "name": component.name,
        "kind": inventory.kind,
        "source_path": str(component.path.expanduser().resolve()),
        "size_bytes": inventory.size_bytes,
        "sha256": inventory.sha256,
        "materialized": False,
    }


def _protected_config_record(config: ProtectedConfigInput) -> dict:
    if not SAFE_NAME.fullmatch(config.name):
        raise ValueError(f"invalid protected config name: {config.name}")
    path = config.path.expanduser()
    try:
        metadata = path.lstat()
    except FileNotFoundError as error:
        raise ValueError(f"protected config does not exist: {path}") from error
    if not stat.S_ISREG(metadata.st_mode) or stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"protected config must be a regular file: {path}")
    mode = stat.S_IMODE(metadata.st_mode)
    if mode != 0o600:
        raise ValueError(f"protected config must have mode 0600: {path}")
    return {
        "name": config.name,
        "source_path": str(path.resolve()),
        "mode": "0600",
        "content_in_manifest": False,
        "materialized": False,
    }


def _identity_payload(components: Sequence[dict], protected_configs: Sequence[dict]) -> dict:
    return {
        "schema_version": SCHEMA_VERSION,
        "components": [
            {
                key: item[key]
                for key in ("role", "name", "kind", "size_bytes", "sha256")
            }
            for item in components
        ],
        "protected_configs": [
            {"name": item["name"], "mode": item["mode"]}
            for item in protected_configs
        ],
    }


def build_manifest(
    components: Sequence[ComponentInput],
    protected_configs: Sequence[ProtectedConfigInput],
) -> dict:
    """Build a canonical manifest without copying any payload."""
    names = [component.name for component in components]
    if len(names) != len(set(names)):
        raise ValueError("duplicate component name")
    protected_names = [config.name for config in protected_configs]
    if len(protected_names) != len(set(protected_names)):
        raise ValueError("duplicate protected config name")
    if set(names).intersection(protected_names):
        raise ValueError("component and protected config names must be distinct")

    present_roles = {component.role for component in components}
    missing = sorted(REQUIRED_ROLES - present_roles)
    if missing:
        raise ValueError(f"missing required component roles: {', '.join(missing)}")

    records = sorted(
        (_component_record(component) for component in components),
        key=lambda item: (item["role"], item["name"]),
    )
    protected_records = sorted(
        (_protected_config_record(config) for config in protected_configs),
        key=lambda item: item["name"],
    )
    identity = _identity_payload(records, protected_records)
    canonical = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "bundle_id": hashlib.sha256(canonical).hexdigest(),
        "created_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "architecture": platform.machine(),
        "payload_bytes": sum(item["size_bytes"] for item in records),
        "components": records,
        "protected_configs": protected_records,
    }


def verify_manifest(manifest: dict) -> None:
    """Recompute every in-place payload identity without reading protected config."""
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ValueError("unsupported sea-going manifest schema")
    components = manifest.get("components")
    protected = manifest.get("protected_configs")
    if not isinstance(components, list) or not isinstance(protected, list):
        raise ValueError("sea-going manifest component lists are invalid")
    roles = {item.get("role") for item in components if isinstance(item, dict)}
    missing = sorted(REQUIRED_ROLES - roles)
    if missing:
        raise ValueError(f"missing required component roles: {', '.join(missing)}")
    for item in components:
        if not isinstance(item, dict):
            raise ValueError("sea-going manifest component is invalid")
        source = Path(str(item.get("source_path", "")))
        inventory = inventory_path(source)
        if (
            inventory.kind != item.get("kind")
            or inventory.size_bytes != item.get("size_bytes")
            or inventory.sha256 != item.get("sha256")
        ):
            raise ValueError(f"component identity changed: {item.get('name', 'unknown')}")
    for item in protected:
        if not isinstance(item, dict):
            raise ValueError("protected config record is invalid")
        record = _protected_config_record(
            ProtectedConfigInput(
                name=str(item.get("name", "")),
                path=Path(str(item.get("source_path", ""))),
            )
        )
        if record["mode"] != item.get("mode"):
            raise ValueError(f"protected config identity changed: {record['name']}")
    identity = _identity_payload(components, protected)
    canonical = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode(
        "utf-8"
    )
    expected = hashlib.sha256(canonical).hexdigest()
    if manifest.get("bundle_id") != expected:
        raise ValueError("sea-going manifest bundle ID is invalid")


def ensure_materialization_capacity(
    destination: Path,
    payload_bytes: int,
    *,
    disk_usage: Callable[[Path], os.statvfs_result | shutil._ntuple_diskusage] = shutil.disk_usage,
) -> None:
    """Require payload capacity while retaining 20 percent filesystem free."""
    destination = destination.expanduser()
    probe = destination
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    usage = disk_usage(probe)
    reserve = int(usage.total * 0.20)
    if usage.free - payload_bytes < reserve:
        raise ValueError(
            "insufficient free space for materialisation plus 20% recovery reserve"
        )


def materialize(
    manifest: dict,
    components: Sequence[ComponentInput],
    destination: Path,
    *,
    disk_usage: Callable[[Path], os.statvfs_result | shutil._ntuple_diskusage] = shutil.disk_usage,
) -> dict:
    """Copy only declared payloads to an explicit portable destination."""
    destination = destination.expanduser()
    if destination.exists() and any(destination.iterdir()):
        raise ValueError(f"materialisation destination is not empty: {destination}")
    ensure_materialization_capacity(
        destination, int(manifest["payload_bytes"]), disk_usage=disk_usage
    )
    destination.mkdir(parents=True, exist_ok=True)
    payload_root = destination / "payload"
    payload_root.mkdir()
    by_name = {component.name: component for component in components}
    materialized_records = []
    for record in manifest["components"]:
        component = by_name[record["name"]]
        target_root = payload_root / component.name
        source = component.path.expanduser()
        if source.is_dir():
            shutil.copytree(source, target_root / source.name)
        else:
            target_root.mkdir()
            shutil.copy2(source, target_root / source.name)
        copied = dict(record)
        copied["materialized"] = True
        copied["materialized_path"] = str(
            (Path("payload") / component.name / source.name).as_posix()
        )
        materialized_records.append(copied)
    output = dict(manifest)
    output["components"] = materialized_records
    output["materialized_at"] = datetime.now(timezone.utc).isoformat().replace(
        "+00:00", "Z"
    )
    output_path = destination / "manifest.json"
    output_path.write_text(
        json.dumps(output, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return output


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--verify-manifest", type=Path)
    parser.add_argument(
        "--component",
        nargs=3,
        action="append",
        metavar=("ROLE", "NAME", "PATH"),
        default=[],
    )
    parser.add_argument(
        "--protected-config",
        nargs=2,
        action="append",
        metavar=("NAME", "PATH"),
        default=[],
    )
    parser.add_argument("--materialize", type=Path)
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv if argv is not None else sys.argv[1:])
    if args.verify_manifest is not None:
        if args.output is not None or args.component or args.protected_config or args.materialize:
            print("sea-going manifest failed: --verify-manifest cannot build a bundle", file=sys.stderr)
            return 2
        try:
            manifest = json.loads(args.verify_manifest.read_text(encoding="utf-8"))
            verify_manifest(manifest)
        except (OSError, ValueError, json.JSONDecodeError) as error:
            print(f"sea-going manifest failed: {error}", file=sys.stderr)
            return 1
        print(json.dumps({"bundle_id": manifest["bundle_id"], "result": "pass"}))
        return 0
    if args.output is None:
        print("sea-going manifest failed: --output is required when building", file=sys.stderr)
        return 2
    components = [
        ComponentInput(role=role, name=name, path=Path(path))
        for role, name, path in args.component
    ]
    protected = [
        ProtectedConfigInput(name=name, path=Path(path))
        for name, path in args.protected_config
    ]
    try:
        manifest = build_manifest(components, protected)
        if args.materialize is not None:
            manifest = materialize(manifest, components, args.materialize)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    except (OSError, ValueError) as error:
        print(f"sea-going manifest failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps({"bundle_id": manifest["bundle_id"], "result": "pass"}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
