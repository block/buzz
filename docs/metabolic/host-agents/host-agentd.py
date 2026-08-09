#!/usr/bin/env python3
"""host-agentd — thin HTTP control plane for headless host agents.

Wraps `buzz-host-agents` for the traveling laptop Remote Agents UI.
Bind to Tailscale IP or 127.0.0.1; never expose to the public internet.

Env:
  HOST_AGENTD_TOKEN     required shared secret (Authorization: Bearer …)
  HOST_AGENTD_HOST      default 127.0.0.1 (use Tailscale IP on home)
  HOST_AGENTD_PORT      default 8787
  BUZZ_HOST_AGENTS      path to buzz-host-agents script
  BUZZ_HOST_ROLE        home|laptop
  BUZZ_HOST_ID          hostname

Endpoints:
  GET  /v1/health
  GET  /v1/status
  GET  /v1/agents
  GET  /v1/location-proof[?view=public|full]
  POST /v1/agents               JSON { seat_id?, display_name?, model?, preset?, room?, notes?, arm? }
  POST /v1/agents/{seat}/arm     JSON { "preset", "room"?, "model"?, "force"? }
  POST /v1/agents/{seat}/disarm  JSON { "preset"? }
  GET  /v1/agents/{seat}/logs?tail=80

Entity holon P0 (LOCK r1c):
  arm is dual-body safe — returns 409 dual_body + public place_proof when DNA already live.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Optional
from urllib.parse import parse_qs, urlparse

# place_proof lives next to this daemon
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from place_proof import (  # type: ignore
        check_dual_body,
        grant_lease,
        load_registry,
        public_only,
        release_lease,
        resolve_birth_cert,
        build_location_bundle,
        write_proof_file,
    )
except ImportError:  # pragma: no cover
    check_dual_body = None  # type: ignore
    grant_lease = None  # type: ignore
    load_registry = None  # type: ignore
    public_only = None  # type: ignore
    release_lease = None  # type: ignore
    resolve_birth_cert = None  # type: ignore
    build_location_bundle = None  # type: ignore
    write_proof_file = None  # type: ignore


def env(name: str, default: str = "") -> str:
    return os.environ.get(name, default).strip()


TOKEN = env("HOST_AGENTD_TOKEN")
BIND_HOST = env("HOST_AGENTD_HOST", "127.0.0.1")
BIND_PORT = int(env("HOST_AGENTD_PORT", "8787") or "8787")
CLI = env("BUZZ_HOST_AGENTS") or str(
    Path(__file__).resolve().with_name("buzz-host-agents")
)


def run_cli(args: list[str], timeout: float = 120.0) -> tuple[int, str, str]:
    cli_path = Path(CLI)
    if not cli_path.is_file():
        return 127, "", f"buzz-host-agents not found: {CLI}"
    # Always invoke via bash so non-executable installs still work
    cmd = ["bash", str(cli_path), *args]
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
            env={**os.environ},
        )
        return proc.returncode, proc.stdout or "", proc.stderr or ""
    except FileNotFoundError:
        return 127, "", "bash not found"
    except subprocess.TimeoutExpired:
        return 124, "", "timeout"


def status_json() -> dict[str, Any]:
    code, out, err = run_cli(["status", "--json"])
    if code != 0:
        return {
            "ok": False,
            "error": err.strip() or out.strip() or f"exit {code}",
            "raw": out,
        }
    try:
        data = json.loads(out)
        data["ok"] = True
        return data
    except json.JSONDecodeError:
        # Older CLI may not support --json; parse human status lightly
        return {
            "ok": True,
            "schema": "host-agent.status.v0",
            "raw": out,
            "stderr": err,
            "host_id": env("BUZZ_HOST_ID") or None,
            "host_role": env("BUZZ_HOST_ROLE") or None,
        }


class Handler(BaseHTTPRequestHandler):
    server_version = "host-agentd/0.1"

    def log_message(self, fmt: str, *args: Any) -> None:
        sys.stderr.write("host-agentd: " + (fmt % args) + "\n")

    def _cors(self) -> None:
        # Laptop Desktop (tauri:// / http://localhost:1420) and browser dogfood
        # call host-agentd via loopback tunnel. Allow any origin on loopback
        # binds only — daemon must stay off the public internet.
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header(
            "Access-Control-Allow-Headers",
            "Authorization, Content-Type, X-Host-Agent-Token, Accept",
        )
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Max-Age", "600")

    def _unauthorized(self) -> None:
        self.send_response(401)
        self.send_header("Content-Type", "application/json")
        self._cors()
        self.end_headers()
        self.wfile.write(b'{"error":"unauthorized"}')

    def _json(self, code: int, body: dict[str, Any]) -> None:
        raw = json.dumps(body, indent=2).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self._cors()
        self.end_headers()
        self.wfile.write(raw)

    def _check_auth(self) -> bool:
        if not TOKEN:
            self._json(500, {"error": "HOST_AGENTD_TOKEN not configured"})
            return False
        auth = self.headers.get("Authorization") or ""
        if auth == f"Bearer {TOKEN}" or auth == TOKEN:
            return True
        # also allow X-Host-Agent-Token
        if (self.headers.get("X-Host-Agent-Token") or "") == TOKEN:
            return True
        self._unauthorized()
        return False

    def _read_json(self) -> dict[str, Any]:
        length = int(self.headers.get("Content-Length") or "0")
        if length <= 0:
            return {}
        raw = self.rfile.read(length)
        try:
            data = json.loads(raw.decode("utf-8"))
            return data if isinstance(data, dict) else {}
        except json.JSONDecodeError:
            return {}

    def do_OPTIONS(self) -> None:  # noqa: N802
        self.send_response(204)
        self._cors()
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802
        if not self._check_auth():
            return
        path = urlparse(self.path).path
        qs = parse_qs(urlparse(self.path).query)

        if path in ("/v1/health", "/health"):
            self._json(200, {"ok": True, "service": "host-agentd"})
            return

        if path in ("/v1/status", "/status"):
            self._json(200, status_json())
            return

        if path in ("/v1/location-proof", "/location-proof"):
            try:
                st = status_json()
                if build_location_bundle is None:
                    raise RuntimeError("place_proof module missing")
                bundle = build_location_bundle(st if st.get("ok") else None)
                if write_proof_file is not None:
                    write_proof_file(bundle)
                view = (qs.get("view") or ["full"])[0].lower()
                if view == "public" and public_only is not None:
                    self._json(200, public_only(bundle))
                else:
                    self._json(200, bundle)
            except Exception as exc:  # keep controller alive
                self._json(500, {"ok": False, "error": str(exc)[:200]})
            return

        if path in ("/v1/agents", "/agents"):
            st = status_json()
            agents = st.get("seats") if isinstance(st, dict) else []
            self._json(
                200,
                {
                    "ok": st.get("ok", False) if isinstance(st, dict) else False,
                    "host_id": st.get("host_id") if isinstance(st, dict) else None,
                    "host_role": st.get("host_role") if isinstance(st, dict) else None,
                    "agents": agents or [],
                    "status": st,
                },
            )
            return

        if path.startswith("/v1/agents/") and path.endswith("/logs"):
            # /v1/agents/{seat}/logs
            parts = path.strip("/").split("/")
            # v1 agents seat logs
            seat = parts[2] if len(parts) >= 4 else ""
            tail = int((qs.get("tail") or ["80"])[0])
            role = env("BUZZ_HOST_ROLE") or "home"
            unit_root = Path.home() / ".buzz-dev" / "hosts" / role / "units"
            logs: list[str] = []
            if unit_root.is_dir() and seat:
                for log in sorted(unit_root.glob(f"{seat}-*/watch.log")):
                    try:
                        lines = log.read_text(errors="replace").splitlines()
                        logs.append(f"--- {log.name} ---")
                        logs.extend(lines[-tail:])
                    except OSError as exc:
                        logs.append(f"error reading {log}: {exc}")
            self._json(200, {"ok": True, "seat": seat, "lines": logs})
            return

        self._json(404, {"error": "not_found", "path": path})

    def do_POST(self) -> None:  # noqa: N802
        if not self._check_auth():
            return
        path = urlparse(self.path).path
        body = self._read_json()
        parts = [p for p in path.strip("/").split("/") if p]

        # POST /v1/agents — create/register a remote seat (Desktop "+" card)
        if parts == ["v1", "agents"]:
            seat = str(body.get("seat_id") or body.get("seat") or "").strip()
            display = str(body.get("display_name") or body.get("name") or "").strip()
            model = str(body.get("model") or "").strip()
            notes = str(body.get("notes") or "").strip()
            room = str(body.get("room") or "").strip()
            preset = str(body.get("preset") or "co-lab-gemma").strip()
            arm_now = bool(body.get("arm", True))
            if not seat:
                # Derive slug from display name when seat omitted
                raw = display or "remote-agent"
                seat = "".join(
                    c if c.isalnum() or c in "._-" else "-" for c in raw.lower()
                ).strip("-")[:63] or "remote-agent"
            args = ["register", "--seat", seat]
            if model:
                args.extend(["--model", model])
            if notes:
                args.extend(["--notes", notes])
            if room:
                args.extend(["--room", room])
            if display:
                args.extend(["--display", display])
            code, out, err = run_cli(args, timeout=60.0)
            redacted_err = "\n".join(
                ln
                for ln in (err or "").splitlines()
                if "TOKEN" not in ln.upper() and "PRIVATE" not in ln.upper()
            )
            if code != 0:
                self._json(
                    500,
                    {
                        "ok": False,
                        "error": redacted_err or out or f"register exit {code}",
                        "exit": code,
                        "stdout": out[-2000:],
                        "stderr": redacted_err[-1000:],
                    },
                )
                return
            arm_result: dict[str, Any] = {}
            if arm_now and preset != "status-only":
                if check_dual_body is not None and load_registry is not None:
                    reg = load_registry()
                    seat_row = next(
                        (
                            s
                            for s in (reg.get("seats") or [])
                            if s.get("seat_id") == seat
                        ),
                        {"seat_id": seat, "display_name": display},
                    )
                    blocked, existing = check_dual_body(seat_row, seat)
                    if blocked:
                        self._json(
                            409,
                            {
                                "ok": False,
                                "error": "dual_body",
                                "message": (
                                    "birth cert already has a live body; "
                                    "registered seat but refuse second arm"
                                ),
                                "seat_id": seat,
                                "registered": True,
                                "place_proof": existing,
                            },
                        )
                        return
                arm_args = ["arm", "--preset", preset, "--seat", seat]
                if room:
                    arm_args.extend(["--room", room])
                if model:
                    arm_args.extend(["--model", model])
                acode, aout, aerr = run_cli(arm_args, timeout=180.0)
                aerr_r = "\n".join(
                    ln
                    for ln in (aerr or "").splitlines()
                    if "TOKEN" not in ln.upper() and "PRIVATE" not in ln.upper()
                )
                lease_info: dict[str, Any] = {}
                if acode == 0 and grant_lease is not None and load_registry is not None:
                    reg = load_registry()
                    seat_row = next(
                        (
                            s
                            for s in (reg.get("seats") or [])
                            if s.get("seat_id") == seat
                        ),
                        {"seat_id": seat},
                    )
                    lease_info = grant_lease(seat_row, seat)
                arm_result = {
                    "ok": acode == 0,
                    "exit": acode,
                    "stdout": aout[-2000:],
                    "stderr": aerr_r[-1000:],
                    "lease": lease_info or None,
                }
            self._json(
                200,
                {
                    "ok": True,
                    "seat_id": seat,
                    "display_name": display or None,
                    "model": model or None,
                    "preset": preset,
                    "armed": arm_result.get("ok") if arm_result else False,
                    "register_stdout": out[-2000:],
                    "arm": arm_result or None,
                },
            )
            return

        # /v1/agents/{seat}/arm|disarm
        # ["v1","agents",seat,"arm"]
        if len(parts) == 4 and parts[0] == "v1" and parts[1] == "agents":
            seat = parts[2]
            action = parts[3]
            preset = str(body.get("preset") or "co-lab-gemma")
            room = str(body.get("room") or "")
            model = str(body.get("model") or "")

            allowed = {
                "co-lab-gemma",
                "co-lab-watch",
                "push-nerve",
                "codex-home",
                "codex@home",
                "status-only",
            }
            if preset not in allowed:
                self._json(
                    400,
                    {
                        "ok": False,
                        "error": f"unknown preset {preset}",
                        "allowed": sorted(allowed),
                    },
                )
                return

            if action == "arm":
                force = bool(body.get("force"))
                # P0 dual-body refuse (LOCK r1c) — unless force (rare; transfer path later)
                if not force and check_dual_body is not None and load_registry is not None:
                    reg = load_registry()
                    seat_row = next(
                        (
                            s
                            for s in (reg.get("seats") or [])
                            if s.get("seat_id") == seat
                        ),
                        {"seat_id": seat},
                    )
                    blocked, existing = check_dual_body(seat_row, seat)
                    if blocked:
                        self._json(
                            409,
                            {
                                "ok": False,
                                "error": "dual_body",
                                "message": (
                                    "birth cert already has a live body on this host; "
                                    "adopt existing or fork new DNA — refuse silent dual"
                                ),
                                "action": "arm",
                                "seat": seat,
                                "place_proof": existing,
                            },
                        )
                        return

                args = ["arm", "--preset", preset, "--seat", seat]
                if room:
                    args.extend(["--room", room])
                if model:
                    args.extend(["--model", model])
                code, out, err = run_cli(args, timeout=180.0)
                # Never echo secrets if env leaked into stderr
                redacted_err = "\n".join(
                    ln
                    for ln in (err or "").splitlines()
                    if "TOKEN" not in ln.upper() and "PRIVATE" not in ln.upper()
                )
                lease_info: dict[str, Any] = {}
                if code == 0 and grant_lease is not None and load_registry is not None:
                    reg = load_registry()
                    seat_row = next(
                        (
                            s
                            for s in (reg.get("seats") or [])
                            if s.get("seat_id") == seat
                        ),
                        {"seat_id": seat},
                    )
                    # Prefer unit name as body_id when present in stdout / units
                    lease_info = grant_lease(seat_row, seat)
                self._json(
                    200 if code == 0 else 500,
                    {
                        "ok": code == 0,
                        "action": "arm",
                        "seat": seat,
                        "preset": preset,
                        "exit": code,
                        "stdout": out[-4000:],
                        "stderr": redacted_err[-2000:],
                        "lease": lease_info or None,
                    },
                )
                return

            if action == "disarm":
                args = ["disarm", "--preset", preset, "--seat", seat]
                code, out, err = run_cli(args, timeout=60.0)
                redacted_err = "\n".join(
                    ln
                    for ln in (err or "").splitlines()
                    if "TOKEN" not in ln.upper() and "PRIVATE" not in ln.upper()
                )
                if code == 0 and release_lease is not None and load_registry is not None:
                    reg = load_registry()
                    seat_row = next(
                        (
                            s
                            for s in (reg.get("seats") or [])
                            if s.get("seat_id") == seat
                        ),
                        {"seat_id": seat},
                    )
                    birth = ""
                    if resolve_birth_cert is not None:
                        birth = resolve_birth_cert(seat_row, seat)
                    release_lease(seat, birth)
                self._json(
                    200 if code == 0 else 500,
                    {
                        "ok": code == 0,
                        "action": "disarm",
                        "seat": seat,
                        "preset": preset,
                        "exit": code,
                        "stdout": out[-4000:],
                        "stderr": redacted_err[-2000:],
                    },
                )
                return

        self._json(404, {"error": "not_found", "path": path})


def main() -> int:
    if not TOKEN:
        print("error: set HOST_AGENTD_TOKEN", file=sys.stderr)
        return 2
    if not Path(CLI).exists():
        print(f"error: CLI missing: {CLI}", file=sys.stderr)
        return 2
    # ensure executable path works via bash
    os.environ.setdefault("BUZZ_HOST_ROLE", env("BUZZ_HOST_ROLE") or "home")
    httpd = ThreadingHTTPServer((BIND_HOST, BIND_PORT), Handler)
    print(
        f"host-agentd listen http://{BIND_HOST}:{BIND_PORT} cli={CLI}",
        flush=True,
    )
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        print("host-agentd stop", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
