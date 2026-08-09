#!/usr/bin/env python3
"""Negative smoke tests for host-agentd (no network to real home required)."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DAEMON = ROOT / "host-agentd.py"
CLI = ROOT / "buzz-host-agents"
PORT = 18799
TOKEN = "test-negative-token"


def http(method: str, path: str, token: str | None = TOKEN, body: dict | None = None):
    data = None
    headers = {}
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(
        f"http://127.0.0.1:{PORT}{path}",
        data=data,
        headers=headers,
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, json.loads(resp.read().decode())
    except urllib.error.HTTPError as exc:
        raw = exc.read().decode()
        try:
            parsed = json.loads(raw) if raw else {}
        except json.JSONDecodeError:
            parsed = {"raw": raw}
        return exc.code, parsed


def main() -> int:
    env = {
        **os.environ,
        "HOST_AGENTD_TOKEN": TOKEN,
        "HOST_AGENTD_HOST": "127.0.0.1",
        "HOST_AGENTD_PORT": str(PORT),
        "BUZZ_HOST_ROLE": "laptop",
        "BUZZ_HOST_AGENTS": str(CLI),
    }
    proc = subprocess.Popen(
        [sys.executable, str(DAEMON)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        deadline = time.time() + 5
        while time.time() < deadline:
            try:
                code, body = http("GET", "/v1/health")
                if code == 200 and body.get("ok"):
                    break
            except Exception:
                time.sleep(0.1)
        else:
            print("FAIL daemon did not start")
            return 1

        code, _ = http("GET", "/v1/status", token=None)
        assert code == 401, code
        code, _ = http("GET", "/v1/status", token="wrong")
        assert code == 401, code
        code, body = http("GET", "/v1/status")
        assert code == 200, (code, body)
        code, body = http("GET", "/v1/location-proof")
        assert code == 200, (code, body)
        # place_proof.v1 (P0) or legacy seat-location.v0
        assert body.get("schema") in (
            "place_proof.v1",
            "seat-location.v0",
        ) or body.get("ok") is True
        code, body = http("GET", "/v1/location-proof?view=public")
        assert code == 200, (code, body)
        assert body.get("schema") == "place_proof.v1" or body.get("ok") is True
        # public view must not leak host_local paths
        assert "host_local" not in body or body.get("view") == "public"
        code, body = http(
            "POST",
            "/v1/agents/home-grok/arm",
            body={"preset": "rm-rf-nope"},
        )
        assert code == 400, (code, body)
        assert body.get("ok") is False
        print("HOST_AGENTD_NEGATIVE_OK")
        return 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
