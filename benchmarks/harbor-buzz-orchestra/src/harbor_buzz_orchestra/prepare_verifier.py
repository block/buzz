"""Generate deterministic, offline-verifiable Harbor task copies."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from pathlib import Path

LOCK_ROOT = Path(__file__).resolve().parents[2] / "verifier-locks"
PREPARED_DIR = ".buzz-verifier"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tree_hash(root: Path) -> str:
    digest = hashlib.sha256()
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        relative = path.relative_to(root).as_posix().encode()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(bytes.fromhex(sha256(path)))
    return digest.hexdigest()


def _append_verifier_layer(dockerfile: Path, base_image: str) -> None:
    original = dockerfile.read_text()
    lines = original.splitlines()
    if not lines or not lines[0].startswith("FROM "):
        raise ValueError(
            f"unsupported Dockerfile (first line must be FROM): {dockerfile}"
        )
    lines[0] = f"FROM {base_image}"
    addition = r"""

# Generated additive layer: verifier dependencies are present before grading.
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 python3-venv \
 && rm -rf /var/lib/apt/lists/*
COPY .buzz-verifier/wheels /opt/buzz-verifier/wheels
COPY .buzz-verifier/requirements.lock /opt/buzz-verifier/requirements.lock
RUN python3 -m venv /opt/buzz-verifier/venv \
 && /opt/buzz-verifier/venv/bin/pip install \
      --no-index --find-links=/opt/buzz-verifier/wheels \
      --require-hashes -r /opt/buzz-verifier/requirements.lock
COPY .buzz-verifier/bin /opt/buzz-verifier/bin
RUN chmod 0555 /opt/buzz-verifier/bin/* \
 && mkdir -p /root/.local/bin \
 && printf '# verifier bootstrap already prepared\n' > /root/.local/bin/env
"""
    dockerfile.write_text("\n".join(lines) + addition)


def _write_shims(destination: Path) -> None:
    bin_dir = destination / "environment" / PREPARED_DIR / "bin"
    bin_dir.mkdir(parents=True)
    (bin_dir / "apt-get").write_text(
        '#!/bin/sh\ncase "$*" in update|"install -y curl") exit 0;; '
        '*) exec /usr/bin/apt-get "$@";; esac\n'
    )
    (bin_dir / "curl").write_text(
        '#!/bin/sh\ncase "$*" in *https://astral.sh/uv/0.9.7/install.sh*) '
        "printf '#!/bin/sh\\nexit 0\\n';; *) exec /usr/bin/curl \"$@\";; esac\n"
    )
    (bin_dir / "uvx").write_text(
        '#!/bin/sh\nwhile [ "$#" -gt 0 ]; do\n'
        '  case "$1" in --with) shift 2;; pytest) shift; break;; *) shift;; esac\n'
        'done\nexec /opt/buzz-verifier/venv/bin/pytest "$@"\n'
    )


def _set_verifier_env(task_toml: Path, network_enforcement: str) -> None:
    text = task_toml.read_text()
    marker = "[verifier]\n"
    if text.count(marker) != 1:
        raise ValueError(f"expected exactly one [verifier] table: {task_toml}")
    verifier_table = text[
        text.index(marker) : text.index("\n[", text.index(marker) + 1)
    ]
    if "network_mode" in verifier_table:
        raise ValueError("verifier network_mode already declared")
    if network_enforcement == "enforced":
        text = text.replace(marker, marker + 'network_mode = "no-network"\n', 1)
    elif network_enforcement != "advisory":
        raise ValueError(
            "verifier network enforcement must be 'enforced' or 'advisory'"
        )
    env_marker = "[verifier.env]\n"
    if text.count(env_marker) != 1:
        raise ValueError(f"expected exactly one [verifier.env] table: {task_toml}")
    text = text.replace(
        env_marker,
        env_marker
        + 'PATH = "/opt/buzz-verifier/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"\n'
        + 'UV_OFFLINE = "1"\n',
        1,
    )
    task_toml.write_text(text)


def prepare_task(
    source: Path,
    destination: Path,
    lock_dir: Path,
    *,
    network_enforcement: str = "enforced",
) -> dict[str, object]:
    source = source.resolve()
    if network_enforcement not in {"enforced", "advisory"}:
        raise ValueError(
            "verifier network enforcement must be 'enforced' or 'advisory'"
        )
    if destination.exists():
        raise FileExistsError(f"destination already exists: {destination}")
    lock = json.loads((lock_dir / "lock.json").read_text())
    required = ["task.toml", "environment/Dockerfile", "tests/test.sh"]
    for relative in required:
        if not (source / relative).is_file():
            raise FileNotFoundError(source / relative)
    for package in lock["python_packages"]:
        wheel = lock_dir / "wheels" / package["file"]
        actual = sha256(wheel)
        if actual != package["sha256"]:
            raise ValueError(f"wheel hash mismatch for {wheel.name}: {actual}")

    source_test_hashes = {
        path.relative_to(source).as_posix(): sha256(path)
        for path in sorted((source / "tests").rglob("*"))
        if path.is_file()
    }
    shutil.copytree(source, destination)
    prepared = destination / "environment" / PREPARED_DIR
    prepared.mkdir()
    shutil.copy2(lock_dir / "requirements.lock", prepared / "requirements.lock")
    shutil.copytree(lock_dir / "wheels", prepared / "wheels")
    _write_shims(destination)
    _append_verifier_layer(destination / "environment/Dockerfile", lock["base_image"])
    _set_verifier_env(destination / "task.toml", network_enforcement)

    output_test_hashes = {
        path.relative_to(destination).as_posix(): sha256(path)
        for path in sorted((destination / "tests").rglob("*"))
        if path.is_file()
    }
    if output_test_hashes != source_test_hashes:
        raise AssertionError("generator changed verifier test bytes")

    metadata: dict[str, object] = {
        "schema_version": 1,
        "task": lock["task"],
        "prepared_image": True,
        "source_task_content_sha256": tree_hash(source),
        "source_test_sha256": source_test_hashes,
        "dependency_lock_sha256": sha256(lock_dir / "requirements.lock"),
        "dependency_manifest_sha256": sha256(lock_dir / "lock.json"),
        "prepared_layer_content_sha256": tree_hash(prepared),
        "base_image": lock["base_image"],
        "verifier_network_mode": (
            "no-network" if network_enforcement == "enforced" else None
        ),
        "verifier_network_enforcement": network_enforcement,
        "offline_attestation_required": network_enforcement == "advisory",
    }
    (destination / "prepared-image.json").write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n"
    )
    return metadata


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    parser.add_argument("--lock-dir", type=Path, default=LOCK_ROOT / "hello-world")
    parser.add_argument(
        "--network-enforcement",
        choices=("enforced", "advisory"),
        default="enforced",
        help="advisory is a laptop-only divergence and requires direct offline attestation",
    )
    args = parser.parse_args()
    print(
        json.dumps(
            prepare_task(
                args.source,
                args.destination,
                args.lock_dir,
                network_enforcement=args.network_enforcement,
            ),
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
