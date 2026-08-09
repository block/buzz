#!/usr/bin/env python3
"""seat-location / place_proof bridge (P6 + entity-holon P0).

Delegates to place_proof.py (place_proof.v1). Keeps CLI --write / --print-board.
"""
from __future__ import annotations

import json
from typing import Any, Optional

from place_proof import (
    LEGACY_SCHEMA,
    PUBLIC_SCHEMA,
    build_location_bundle,
    build_location_proof,
    host_id,
    host_role,
    public_only,
    write_proof_file,
)

# re-export for importers
SCHEMA = LEGACY_SCHEMA
__all__ = [
    "SCHEMA",
    "PUBLIC_SCHEMA",
    "LEGACY_SCHEMA",
    "build_location_proof",
    "build_location_bundle",
    "write_proof_file",
    "phone_safe_board_line",
    "public_only",
    "host_id",
    "host_role",
    "main",
]


def phone_safe_board_line(proof: dict[str, Any]) -> str:
    bodies = proof.get("bodies") or proof.get("seats") or []
    bits = []
    for s in bodies:
        sid = s.get("seat_id") or s.get("legal_name") or "?"
        health = s.get("health") or "?"
        host = s.get("host_id") or ""
        birth = (s.get("birth_cert_id") or "")[:8]
        bits.append(f"{sid}={health}@{host}" + (f" dna={birth}" if birth else ""))
    return (
        f"## HOST location proof\n\n"
        f"`host={proof.get('host_id')} role={proof.get('host_role')} "
        f"bodies={', '.join(bits) or 'none'} ts={proof.get('ts')}`\n\n"
        f"`{proof.get('schema') or PUBLIC_SCHEMA} · heartbeat`"
    )


def main() -> None:
    import argparse
    from pathlib import Path

    from place_proof import inject_seat_self_location

    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--write", action="store_true", help="write location-proof.json")
    p.add_argument(
        "--print-board", action="store_true", help="print phone-safe board markdown"
    )
    p.add_argument(
        "--public",
        action="store_true",
        help="emit public-only place_proof.v1 (no surface_root/pid)",
    )
    p.add_argument(
        "--inject-seat",
        metavar="SEAT",
        help="R3: write self-location.env + PLACE_PROMPT.txt for seat",
    )
    p.add_argument(
        "--unit-dir",
        metavar="DIR",
        help="unit directory for --inject-seat (required with inject)",
    )
    args = p.parse_args()
    if args.inject_seat:
        if not args.unit_dir:
            raise SystemExit("--unit-dir required with --inject-seat")
        paths = inject_seat_self_location(args.inject_seat, Path(args.unit_dir))
        prompt = paths["prompt"].read_text()
        if "/home/" in prompt or "surface_root" in prompt.lower():
            print("R3_PROMPT_PUBLIC_FAIL path leak", flush=True)
            raise SystemExit(2)
        print(f"R3_PROMPT_PUBLIC_OK wrote {paths['env']} {paths['prompt']}", flush=True)
        return
    proof = build_location_bundle()
    if args.public:
        proof = public_only(proof)
    if args.write:
        path = write_proof_file(build_location_bundle())
        print(f"wrote {path}")
    if args.print_board:
        print(phone_safe_board_line(proof))
    if not args.write and not args.print_board and not args.inject_seat:
        print(json.dumps(proof, indent=2))


if __name__ == "__main__":
    main()
