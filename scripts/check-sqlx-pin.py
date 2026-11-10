#!/usr/bin/env python3
"""Keep the SQLx recovery patch selected until an intentional upgrade (Python 3.11+)."""

import json
import sys
import tomllib
from pathlib import Path


CRATES = [
    "sqlx",
    "sqlx-core",
    "sqlx-macros",
    "sqlx-macros-core",
    "sqlx-mysql",
    "sqlx-postgres",
    "sqlx-sqlite",
]
REVISION = "c3735abf034689bb1fab9d7bf9f040e492de28cf"
REPOSITORY = "https://github.com/launchbadge/sqlx"
VERSION = "0.9.0"


def check(root: Path) -> None:
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    dependency = manifest["workspace"]["dependencies"]["sqlx"]
    if dependency.get("version") != f"={VERSION}":
        raise ValueError(f"SQLx must require exactly ={VERSION}; newer releases may omit the patch")

    patches = manifest.get("patch", {}).get("crates-io", {})
    for name in CRATES:
        if patches.get(name) != {"git": REPOSITORY, "rev": REVISION}:
            raise ValueError(f"{name}: preserve the reviewed SQLx Git patch")

    lock = tomllib.loads((root / "Cargo.lock").read_text())
    source = f"git+{REPOSITORY}?rev={REVISION}#{REVISION}"
    for name in CRATES:
        packages = [p for p in lock["package"] if p["name"] == name]
        if (
            len(packages) != 1
            or packages[0].get("version") != VERSION
            or packages[0].get("source") != source
        ):
            raise ValueError(f"{name}: Cargo.lock must contain exactly one package from the reviewed revision")

    rules = json.loads((root / "renovate.json").read_text()).get("packageRules", [])
    # Renovate applies matching rules in order. The final rule must retain the
    # disable, including when an earlier rule enables automerge for all crates.
    required = {"matchManagers": ["cargo"], "matchDepNames": CRATES, "enabled": False}
    if not rules or {k: v for k, v in rules[-1].items() if k != "description"} != required:
        raise ValueError("The final Renovate rule must disable updates for all seven SQLx crates")


if __name__ == "__main__":
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1]
    try:
        check(root)
    except (OSError, ValueError, KeyError, TypeError) as error:
        sys.exit(f"SQLx pin check failed: {error}. Verify all required fixes before changing the pin policy.")
    print("SQLx manifest, lockfile, and Renovate policy preserve the reviewed revision")
