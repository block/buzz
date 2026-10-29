#!/usr/bin/env python3
"""Compute an exact, version-agnostic Cargo release cache key."""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DESKTOP_MANIFEST = pathlib.Path("desktop/src-tauri/Cargo.toml")
DESKTOP_LOCK = pathlib.Path("desktop/src-tauri/Cargo.lock")


def normalized(path: pathlib.Path, data: bytes) -> bytes:
    text = data.decode()
    if path == DESKTOP_MANIFEST:
        text, count = re.subn(
            r'(?ms)(^\[package\].*?^version\s*=\s*)"[^"]+"',
            r'\1"<desktop-version>"',
            text,
            count=1,
        )
        if count != 1:
            raise ValueError(f"could not normalize package version in {path}")
    elif path == DESKTOP_LOCK:
        text, count = re.subn(
            r'(?ms)(^name = "buzz-desktop"\nversion = )"[^"]+"',
            r'\1"<desktop-version>"',
            text,
            count=1,
        )
        if count != 1:
            raise ValueError(f"could not normalize package version in {path}")
    return text.encode()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--platform", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--features", default="default")
    parser.add_argument("--native-inputs", required=True)
    args = parser.parse_args()

    manifest_output = subprocess.check_output(
        ["git", "ls-files", "*Cargo.toml"], cwd=ROOT, text=True
    )
    paths = [ROOT / path for path in manifest_output.splitlines()]
    paths += [ROOT / "Cargo.lock", ROOT / DESKTOP_LOCK, ROOT / "rust-toolchain.toml"]
    cargo_config = ROOT / ".cargo/config.toml"
    if cargo_config.exists():
        paths.append(cargo_config)

    digest = hashlib.sha256()
    descriptors = {
        "schema": "desktop-rust-release-v1",
        "platform": args.platform,
        "target": args.target,
        "profile": "release",
        "features": args.features,
        "native-inputs": args.native_inputs,
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True).strip(),
    }
    for name, value in sorted(descriptors.items()):
        digest.update(f"{name}\0{value}\0".encode())

    for absolute in sorted(set(paths)):
        relative = absolute.relative_to(ROOT)
        digest.update(str(relative).encode() + b"\0")
        digest.update(normalized(relative, absolute.read_bytes()) + b"\0")

    print(f"desktop-rust-release-v1-{args.platform}-{args.target}-{digest.hexdigest()}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
