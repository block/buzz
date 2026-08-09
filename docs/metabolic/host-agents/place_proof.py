#!/usr/bin/env python3
"""place_proof.v1 — birth cert · body · public vs host-local proofs.

LOCK (agent-entity-holon r1c):
  birth_cert_id = Nostr pubkey (v0)
  body_id = one runtime instance
  lease_epoch = fence for live body ownership
  Public proofs never carry nsec, tokens, full surface_root, or pid.

Host-local registry may still store surface_root for tool binding.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import time
import uuid
from pathlib import Path
from typing import Any, Optional

PUBLIC_SCHEMA = "place_proof.v1"
HOST_LOCAL_SCHEMA = "place_proof.host_local.v1"
LEGACY_SCHEMA = "seat-location.v0"

SURFACE_KINDS = frozenset(
    {"desktop-local", "cli-seat", "host-unit", "remote-view"}
)

TTL_SECS_DEFAULT = 90


def host_role() -> str:
    return os.environ.get("BUZZ_HOST_ROLE") or "home"


def host_id() -> str:
    return os.environ.get("BUZZ_HOST_ID") or os.uname().nodename


def host_root() -> Path:
    override = os.environ.get("BUZZ_HOST_ROOT")
    if override:
        return Path(override)
    return Path.home() / ".buzz-dev" / "hosts" / host_role()


def registry_path() -> Path:
    override = os.environ.get("BUZZ_HOST_REGISTRY")
    if override:
        return Path(override)
    return host_root() / "registry.json"


def units_dir() -> Path:
    return host_root() / "units"


def leases_path() -> Path:
    return host_root() / "leases.json"


def load_registry() -> dict[str, Any]:
    path = registry_path()
    if not path.is_file():
        return {"seats": [], "host_id": host_id(), "host_role": host_role()}
    try:
        return json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return {"seats": [], "host_id": host_id(), "host_role": host_role()}


def live_units() -> list[dict[str, Any]]:
    root = units_dir()
    out: list[dict[str, Any]] = []
    if not root.is_dir():
        return out
    for pidf in root.glob("*/watch.pid"):
        unit = pidf.parent.name
        pid: Optional[int] = None
        alive = False
        try:
            pid = int(pidf.read_text().strip())
            os.kill(pid, 0)
            alive = True
        except (ValueError, OSError, ProcessLookupError):
            alive = False
        out.append({"unit_name": unit, "unit_pid": pid, "alive": alive})
    return out


def _read_pubkey_file(path: Path) -> str:
    if not path.is_file():
        return ""
    try:
        text = path.read_text(errors="replace").strip()
    except OSError:
        return ""
    # PUBLIC.txt may be "pubkey_hex: …" or bare hex
    for line in text.splitlines():
        line = line.strip()
        m = re.search(r"\b([0-9a-fA-F]{64})\b", line)
        if m:
            return m.group(1).lower()
    m = re.search(r"\b([0-9a-fA-F]{64})\b", text)
    return m.group(1).lower() if m else ""


def _read_pubkey_env(path: Path) -> str:
    if not path.is_file():
        return ""
    try:
        for line in path.read_text(errors="replace").splitlines():
            if line.startswith("BUZZ_PUBLIC_KEY=") or line.startswith(
                "BUZZ_PUBKEY="
            ):
                val = line.split("=", 1)[1].strip().strip('"').strip("'")
                if re.fullmatch(r"[0-9a-fA-F]{64}", val):
                    return val.lower()
    except OSError:
        return ""
    return ""


def resolve_birth_cert(seat: dict[str, Any], seat_id: str) -> str:
    """Immutable DNA = Nostr pubkey when known."""
    for key in ("pubkey", "pubkey_hint", "birth_cert_id"):
        val = (seat.get(key) or "").strip()
        if re.fullmatch(r"[0-9a-fA-F]{64}", val):
            return val.lower()

    agents_root = Path.home() / ".buzz-dev" / "agents"
    candidates = [
        agents_root / seat_id / "PUBLIC.txt",
        agents_root / seat_id / "agent.env",
        agents_root / seat_id.replace("_", "-") / "PUBLIC.txt",
        agents_root / seat_id.replace("_", "-") / "agent.env",
    ]
    # common aliases
    if seat_id in ("home-grok", "Buzz-home-grok"):
        candidates.extend(
            [
                agents_root / "home-grok" / "PUBLIC.txt",
                agents_root / "home-grok" / "agent.env",
            ]
        )
    for path in candidates:
        if path.name == "PUBLIC.txt":
            pk = _read_pubkey_file(path)
        else:
            pk = _read_pubkey_env(path)
        if pk:
            return pk
    return ""


def surface_kind_for(seat: dict[str, Any], unit_alive: bool) -> str:
    raw = (seat.get("surface_kind") or "").strip()
    if raw in SURFACE_KINDS:
        return raw
    # Heuristic: unit process → host-unit; else cli-seat if expected, else remote-view
    if unit_alive or seat.get("runtimes"):
        return "host-unit"
    if seat.get("expected_online"):
        return "cli-seat"
    return "host-unit"


def surface_id_for(seat_id: str, surface_root: str) -> str:
    """Stable non-path bind id for public proof (no personal FS path)."""
    if not surface_root:
        return f"seat:{seat_id}"
    digest = hashlib.sha256(surface_root.encode("utf-8")).hexdigest()[:16]
    return f"bind:{seat_id}:{digest}"


def load_leases() -> dict[str, Any]:
    path = leases_path()
    if not path.is_file():
        return {"schema": "host-agent.leases.v0", "leases": {}}
    try:
        data = json.loads(path.read_text())
        if not isinstance(data.get("leases"), dict):
            data["leases"] = {}
        return data
    except (json.JSONDecodeError, OSError):
        return {"schema": "host-agent.leases.v0", "leases": {}}


def save_leases(data: dict[str, Any]) -> None:
    path = leases_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, indent=2) + "\n")
    tmp.replace(path)


def new_body_id(seat_id: str) -> str:
    return f"{seat_id}-{uuid.uuid4().hex[:12]}"


def public_place_proof_for_seat(
    seat: dict[str, Any],
    *,
    unit: Optional[dict[str, Any]] = None,
    lease: Optional[dict[str, Any]] = None,
    now: Optional[int] = None,
    ttl_secs: int = TTL_SECS_DEFAULT,
) -> dict[str, Any]:
    now = int(now if now is not None else time.time())
    sid = seat.get("seat_id") or ""
    birth = resolve_birth_cert(seat, sid)
    unit_alive = bool(unit and unit.get("alive"))
    surface_root = (
        seat.get("surface_root")
        or os.environ.get(f"BUZZ_SURFACE_ROOT_{sid.upper().replace('-', '_')}")
        or os.environ.get("BUZZ_SURFACE_ROOT")
        or ""
    )
    health = "down"
    if unit_alive:
        health = "ok"
    elif seat.get("expected_online"):
        health = "stale"
    elif lease and lease.get("expires_at", 0) > now:
        health = "stale"

    body_id = (lease or {}).get("body_id") or (
        (unit or {}).get("unit_name") if unit_alive else ""
    )
    epoch = int((lease or {}).get("lease_epoch") or 0)
    issued = int((lease or {}).get("updated_at") or now)
    expires = int((lease or {}).get("expires_at") or (now + ttl_secs))

    return {
        "schema": PUBLIC_SCHEMA,
        "birth_cert_id": birth,
        "legal_name": seat.get("display_name") or sid,
        "seat_id": sid,
        "body_id": body_id or None,
        "host_id": host_id(),
        "host_role": host_role(),
        "surface_kind": surface_kind_for(seat, unit_alive),
        "surface_id": surface_id_for(sid, surface_root),
        "health": health,
        "lease_epoch": epoch,
        "issued_at": issued,
        "expires_at": expires,
        "attestation": "host-local-v0",
        # public-safe runtime labels only
        "runtime": ",".join(seat.get("runtimes") or []) or None,
        "model": seat.get("model") or None,
    }


def host_local_place_proof_for_seat(
    seat: dict[str, Any],
    *,
    unit: Optional[dict[str, Any]] = None,
    lease: Optional[dict[str, Any]] = None,
    now: Optional[int] = None,
) -> dict[str, Any]:
    """Privileged local view — never post to rooms."""
    pub = public_place_proof_for_seat(seat, unit=unit, lease=lease, now=now)
    sid = seat.get("seat_id") or ""
    surface_root = (
        seat.get("surface_root")
        or os.environ.get(f"BUZZ_SURFACE_ROOT_{sid.upper().replace('-', '_')}")
        or os.environ.get("BUZZ_SURFACE_ROOT")
        or ""
    )
    return {
        **pub,
        "schema": HOST_LOCAL_SCHEMA,
        "surface_root": surface_root,
        "unit_name": (unit or {}).get("unit_name") or "",
        "unit_pid": (unit or {}).get("unit_pid"),
        "channels": seat.get("channels") or [],
        "project_ids": seat.get("project_ids") or [],
        "git_head": seat.get("git_head") or "",
    }


def build_location_bundle(
    status: Optional[dict[str, Any]] = None,
) -> dict[str, Any]:
    """Full bundle: public place_proofs + host-local seats (legacy compatible)."""
    reg = load_registry()
    units = live_units()
    leases = load_leases().get("leases") or {}
    now = int(time.time())
    public_bodies: list[dict[str, Any]] = []
    host_local_seats: list[dict[str, Any]] = []
    legacy_seats: list[dict[str, Any]] = []

    for seat in reg.get("seats") or []:
        sid = seat.get("seat_id") or ""
        unit_match = next(
            (
                u
                for u in units
                if u["unit_name"].startswith(sid + "-") or u["unit_name"] == sid
            ),
            None,
        )
        birth = resolve_birth_cert(seat, sid)
        lease = None
        if birth and birth in leases:
            lease = leases[birth]
        elif sid in leases:
            lease = leases[sid]

        pub = public_place_proof_for_seat(
            seat, unit=unit_match, lease=lease, now=now
        )
        # ensure birth filled when we resolved it
        if birth:
            pub["birth_cert_id"] = birth
        local = host_local_place_proof_for_seat(
            seat, unit=unit_match, lease=lease, now=now
        )
        if birth:
            local["birth_cert_id"] = birth
        public_bodies.append(pub)
        host_local_seats.append(local)

        # legacy seat-location.v0 row (includes paths/pids — host-local only)
        legacy_seats.append(
            {
                "seat_id": sid,
                "pubkey": birth or seat.get("pubkey") or seat.get("pubkey_hint") or "",
                "birth_cert_id": birth,
                "host_id": reg.get("host_id") or host_id(),
                "host_role": reg.get("host_role") or host_role(),
                "surface_root": local.get("surface_root") or "",
                "surface_kind": pub.get("surface_kind") or "",
                "surface_id": pub.get("surface_id"),
                "body_id": pub.get("body_id"),
                "lease_epoch": pub.get("lease_epoch"),
                "git_head": seat.get("git_head") or "",
                "runtime": pub.get("runtime") or "",
                "model": seat.get("model") or "",
                "health": {
                    "ok": "online",
                    "stale": "stale",
                    "down": "stopped",
                    "degraded": "stale",
                }.get(pub.get("health") or "down", "stopped"),
                "channels": seat.get("channels") or [],
                "project_ids": seat.get("project_ids") or [],
                "unit_name": local.get("unit_name") or "",
                "unit_pid": local.get("unit_pid"),
                "updated_at": now,
            }
        )

    return {
        "ok": True,
        "schema": PUBLIC_SCHEMA,
        "legacy_schema": LEGACY_SCHEMA,
        "host_id": reg.get("host_id") or host_id(),
        "host_role": reg.get("host_role") or host_role(),
        "ts": now,
        "bodies": public_bodies,
        "host_local": {
            "schema": HOST_LOCAL_SCHEMA,
            "seats": host_local_seats,
        },
        # backward compat for existing Desktop / tests
        "seats": legacy_seats,
        "status_excerpt": {
            "relay_ok": (status or {}).get("relay", {}).get("ok") if status else None,
            "ollama_ok": (status or {}).get("ollama", {}).get("ok") if status else None,
        },
    }


def public_only(bundle: dict[str, Any]) -> dict[str, Any]:
    """Strip host-local privileged fields for mesh/room exposure."""
    return {
        "ok": bundle.get("ok", True),
        "schema": PUBLIC_SCHEMA,
        "host_id": bundle.get("host_id"),
        "host_role": bundle.get("host_role"),
        "ts": bundle.get("ts"),
        "bodies": bundle.get("bodies") or [],
        "status_excerpt": bundle.get("status_excerpt"),
    }


def find_live_lease_for_birth(
    birth_cert_id: str,
    *,
    units: Optional[list[dict[str, Any]]] = None,
) -> Optional[dict[str, Any]]:
    """Return active lease if body still appears live (unit pid) or unexpired."""
    if not birth_cert_id:
        return None
    leases = load_leases().get("leases") or {}
    lease = leases.get(birth_cert_id)
    if not lease:
        return None
    now = int(time.time())
    units = units if units is not None else live_units()
    body_id = lease.get("body_id") or ""
    unit_match = next(
        (u for u in units if u.get("alive") and u.get("unit_name") == body_id),
        None,
    )
    if unit_match:
        return lease
    # Also match seat-prefix units when body_id is seat-based
    seat_id = lease.get("seat_id") or ""
    if seat_id:
        unit_match = next(
            (
                u
                for u in units
                if u.get("alive")
                and (
                    u["unit_name"].startswith(seat_id + "-")
                    or u["unit_name"] == seat_id
                )
            ),
            None,
        )
        if unit_match:
            return lease
    if int(lease.get("expires_at") or 0) > now and lease.get("force_live"):
        return lease
    return None


def check_dual_body(
    seat: dict[str, Any],
    seat_id: str,
) -> tuple[bool, Optional[dict[str, Any]]]:
    """
    Returns (blocked, public_place_proof_if_blocked).
    blocked=True means arm must 409 dual_body.
    """
    birth = resolve_birth_cert(seat, seat_id)
    units = live_units()
    # Live unit for this seat even without lease file
    unit_match = next(
        (
            u
            for u in units
            if u.get("alive")
            and (u["unit_name"].startswith(seat_id + "-") or u["unit_name"] == seat_id)
        ),
        None,
    )
    lease = find_live_lease_for_birth(birth, units=units) if birth else None
    if not unit_match and not lease:
        return False, None
    pub = public_place_proof_for_seat(
        seat, unit=unit_match, lease=lease or {}
    )
    if birth:
        pub["birth_cert_id"] = birth
    return True, pub


def grant_lease(
    seat: dict[str, Any],
    seat_id: str,
    *,
    body_id: Optional[str] = None,
    ttl_secs: int = TTL_SECS_DEFAULT,
) -> dict[str, Any]:
    """Atomic-ish lease grant after successful arm (file replace)."""
    birth = resolve_birth_cert(seat, seat_id) or f"seat:{seat_id}"
    data = load_leases()
    leases = data.setdefault("leases", {})
    prev = leases.get(birth) or {}
    epoch = int(prev.get("lease_epoch") or 0) + 1
    now = int(time.time())
    bid = body_id or new_body_id(seat_id)
    lease = {
        "birth_cert_id": birth if not birth.startswith("seat:") else "",
        "seat_id": seat_id,
        "body_id": bid,
        "lease_epoch": epoch,
        "host_id": host_id(),
        "host_role": host_role(),
        "updated_at": now,
        "expires_at": now + ttl_secs,
    }
    leases[birth] = lease
    # also index by seat_id for empty-pubkey transition
    leases[seat_id] = lease
    save_leases(data)
    return lease


def release_lease(seat_id: str, birth_cert_id: str = "") -> None:
    data = load_leases()
    leases = data.setdefault("leases", {})
    if birth_cert_id and birth_cert_id in leases:
        del leases[birth_cert_id]
    if seat_id in leases:
        del leases[seat_id]
    save_leases(data)


def write_proof_file(bundle: dict[str, Any]) -> Path:
    path = host_root() / "location-proof.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    # Store host-local full bundle on disk; public view is derived on GET
    path.write_text(json.dumps(bundle, indent=2) + "\n")
    public_path = host_root() / "location-proof.public.json"
    public_path.write_text(json.dumps(public_only(bundle), indent=2) + "\n")
    return path


PLACE_MARKER = "## Self-location (this body only)"


def self_location_prompt_block(
    *,
    legal_name: str,
    birth_cert_id: str,
    body_id: str,
    host: str,
    role: str,
    surface_kind: str,
    surface_id: str,
) -> str:
    """Public-safe prompt block — never include surface_root or /home paths."""
    return (
        f"{PLACE_MARKER}\n"
        f"- legal_name: {legal_name}\n"
        f"- birth_cert (DNA): {birth_cert_id}\n"
        f"- body_id: {body_id}\n"
        f"- host_id: {host}\n"
        f"- host_role: {role}\n"
        f"- surface_kind: {surface_kind}\n"
        f"- surface_id: {surface_id}\n"
        "\n"
        "You are **this body on this host only**. Do not claim another machine's "
        "workspace, files, or uptime. A second process with the same DNA elsewhere is "
        "a different body — refuse to act as if you were that place.\n"
        "(Public place only — full disk paths are not required for self-knowledge.)\n"
    )


def inject_seat_self_location(
    seat_id: str,
    unit_dir: Path,
    *,
    seat: Optional[dict[str, Any]] = None,
) -> dict[str, Path]:
    """
    Entity holon R3: write self-location.env + PLACE_PROMPT.txt under unit_dir.
    Place env wins when sourced after seat agent.env (arm path).
    """
    unit_dir = Path(unit_dir)
    unit_dir.mkdir(parents=True, exist_ok=True)
    reg = load_registry()
    if seat is None:
        seat = next(
            (s for s in (reg.get("seats") or []) if s.get("seat_id") == seat_id),
            {"seat_id": seat_id},
        )
    birth = resolve_birth_cert(seat, seat_id)
    body_id = unit_dir.name
    skind = surface_kind_for(seat, unit_alive=True)
    sroot = (
        seat.get("surface_root")
        or os.environ.get("BUZZ_SURFACE_ROOT")
        or ""
    )
    sid = surface_id_for(seat_id, sroot)
    legal = seat.get("display_name") or seat_id
    host = host_id()
    role = host_role()
    relay = os.environ.get("BUZZ_RELAY_URL") or ""

    env_lines = [
        f"export BUZZ_HOST_ID={_shell_quote(host)}",
        f"export BUZZ_HOST_ROLE={_shell_quote(role)}",
        f"export BUZZ_SURFACE_KIND={_shell_quote(skind)}",
        f"export BUZZ_SURFACE_ID={_shell_quote(sid)}",
        f"export BUZZ_BIRTH_CERT_ID={_shell_quote(birth)}",
        f"export BUZZ_BODY_ID={_shell_quote(body_id)}",
        f"export BUZZ_SEAT_ID={_shell_quote(seat_id)}",
    ]
    if relay:
        env_lines.append(f"export BUZZ_RELAY_URL={_shell_quote(relay)}")
    # host-local only — not in PLACE_PROMPT
    if sroot:
        env_lines.append(f"export BUZZ_SURFACE_ROOT={_shell_quote(sroot)}")

    prompt = self_location_prompt_block(
        legal_name=str(legal),
        birth_cert_id=birth or f"seat:{seat_id}",
        body_id=body_id,
        host=host,
        role=role,
        surface_kind=skind,
        surface_id=sid,
    )
    env_path = unit_dir / "self-location.env"
    prompt_path = unit_dir / "PLACE_PROMPT.txt"
    env_path.write_text("\n".join(env_lines) + "\n")
    prompt_path.write_text(prompt)
    # Convenience for CLI seats
    seat_loc = Path.home() / ".buzz-dev" / "agents" / seat_id / "self-location"
    try:
        seat_loc.mkdir(parents=True, exist_ok=True)
        (seat_loc / "self-location.env").write_text(env_path.read_text())
        (seat_loc / "PLACE_PROMPT.txt").write_text(prompt)
    except OSError:
        pass
    return {"env": env_path, "prompt": prompt_path}


def _shell_quote(value: str) -> str:
    return "'" + value.replace("'", "'\"'\"'") + "'"


# --- backward-compatible names used by host-agentd / location_proof ---

def build_location_proof(status: Optional[dict[str, Any]] = None) -> dict[str, Any]:
    return build_location_bundle(status)
