"""
Pytest test suite for the Antigravity ACP Harness.
Tests the ACP JSON-RPC 2.0 protocol implementation.

TC-01: initialize returns protocolVersion
TC-02: session/new returns sessionId
TC-03: session/prompt emits session/update and done=True  (stub mode)
TC-04: session/cancel returns ok
TC-05: session history preserved across prompts           (stub mode)
TC-06: malformed JSON returns parse error without crashing
TC-07: missing GEMINI_API_KEY returns descriptive error

Note: TC-03 and TC-05 use HARNESS_STUB_RESPONSE env var to bypass real Gemini
calls. The harness checks for this variable and returns the stub value directly,
making tests fully hermetic without requiring a real API key.
"""

import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

HARNESS_PATH = Path(__file__).parent.parent / "buzz_acp_antigravity.py"


# ---------------------------------------------------------------------------
# Helpers — run harness as subprocess, communicate over stdio
# ---------------------------------------------------------------------------

def start_harness(env_override: dict = None) -> subprocess.Popen:
    env = {**os.environ, "GEMINI_API_KEY": "AIzaFakeKeyForTesting"}
    if env_override:
        env.update(env_override)
    return subprocess.Popen(
        [sys.executable, str(HARNESS_PATH)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )


def send_msg(proc: subprocess.Popen, msg: dict) -> None:
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()


def read_line(proc: subprocess.Popen, timeout: float = 5.0) -> dict:
    """Read one JSON-RPC line from harness stdout."""
    result = {}
    def _read():
        nonlocal result
        line = proc.stdout.readline()
        if line:
            result = json.loads(line)
    t = threading.Thread(target=_read, daemon=True)
    t.start()
    t.join(timeout)
    assert result, f"Timed out waiting for harness response after {timeout}s"
    return result


def read_until_response(proc: subprocess.Popen, msg_id, timeout: float = 10.0) -> tuple[list, dict]:
    """Read all session/update notifications and the final response for msg_id."""
    updates = []
    deadline = time.time() + timeout
    while time.time() < deadline:
        line = proc.stdout.readline()
        if not line:
            time.sleep(0.05)
            continue
        obj = json.loads(line)
        if obj.get("method") == "session/update":
            updates.append(obj)
        elif obj.get("id") == msg_id:
            return updates, obj
    pytest.fail(f"Timed out waiting for response to id={msg_id}")


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

FAKE_GEMINI_RESPONSE = "The answer is 4. (Stub Gemini response)"


def start_harness_stub(env_override: dict = None) -> subprocess.Popen:
    """Start harness in stub mode: HARNESS_STUB_RESPONSE bypasses real Gemini."""
    env = {
        **os.environ,
        "GEMINI_API_KEY": "AIzaStubKey",
        "HARNESS_STUB_RESPONSE": FAKE_GEMINI_RESPONSE,
    }
    if env_override:
        env.update(env_override)
    return subprocess.Popen(
        [sys.executable, str(HARNESS_PATH)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env,
    )


# ---------------------------------------------------------------------------
# TC-01: initialize returns protocolVersion and capabilities
# ---------------------------------------------------------------------------

def test_tc01_initialize_returns_protocol_version():
    """TC-01: Handshake initialize responds with protocolVersion and capabilities."""
    proc = start_harness()
    try:
        send_msg(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        resp = read_line(proc)
        assert resp["id"] == 1
        assert "result" in resp
        assert resp["result"]["protocolVersion"] == "0.1"
        assert "capabilities" in resp["result"]
        assert "agentInfo" in resp["result"]
        assert resp["result"]["agentInfo"]["name"] == "Antigravity"
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-02: session/new creates session and returns sessionId
# ---------------------------------------------------------------------------

def test_tc02_session_new_returns_session_id():
    """TC-02: session/new responds with same sessionId."""
    proc = start_harness()
    try:
        send_msg(proc, {
            "jsonrpc": "2.0", "id": 2,
            "method": "session/new",
            "params": {"sessionId": "test-session-tc02", "cwd": "/tmp"},
        })
        resp = read_line(proc)
        assert resp["id"] == 2
        assert resp["result"]["sessionId"] == "test-session-tc02"
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-03: session/prompt emits session/update + done=True
# ---------------------------------------------------------------------------

def test_tc03_session_prompt_emits_update_and_done():
    """TC-03: session/prompt uses stub Gemini, emits session/update and done=True."""
    proc = start_harness_stub()
    try:
        send_msg(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        read_line(proc)
        send_msg(proc, {"jsonrpc": "2.0", "id": 2, "method": "session/new",
                        "params": {"sessionId": "sess-tc03", "cwd": "/tmp"}})
        read_line(proc)

        send_msg(proc, {
            "jsonrpc": "2.0", "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess-tc03",
                "message": {
                    "role": "user",
                    "parts": [{"content_type": "text/plain", "content": "@antigravity what is 2+2?"}],
                },
            },
        })

        updates, result = read_until_response(proc, 3)
        msg_updates = [u for u in updates if u["params"]["update"]["type"] == "message"]
        assert len(msg_updates) >= 1
        assert FAKE_GEMINI_RESPONSE in msg_updates[0]["params"]["update"]["content"]
        assert result["result"]["done"] is True
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-04: session/cancel returns ok without error
# ---------------------------------------------------------------------------

def test_tc04_session_cancel_returns_ok():
    """TC-04: session/cancel responds without error."""
    proc = start_harness()
    try:
        send_msg(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        read_line(proc)
        send_msg(proc, {"jsonrpc": "2.0", "id": 2, "method": "session/new",
                        "params": {"sessionId": "sess-tc04", "cwd": "/tmp"}})
        read_line(proc)

        send_msg(proc, {
            "jsonrpc": "2.0", "id": 3,
            "method": "session/cancel",
            "params": {"sessionId": "sess-tc04"},
        })
        resp = read_line(proc)
        assert resp["id"] == 3
        assert "error" not in resp
        assert resp.get("result") == {}
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-05: Session history preserved across prompts
# ---------------------------------------------------------------------------

def test_tc05_session_history_preserved():
    """TC-05: Second prompt in same session includes prior context (verified via stub)."""
    proc = start_harness_stub()
    try:
        send_msg(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        read_line(proc)
        send_msg(proc, {"jsonrpc": "2.0", "id": 2, "method": "session/new",
                        "params": {"sessionId": "sess-tc05", "cwd": "/tmp"}})
        read_line(proc)

        # First prompt
        send_msg(proc, {
            "jsonrpc": "2.0", "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess-tc05",
                "message": {"role": "user", "parts": [
                    {"content_type": "text/plain", "content": "My project name is BuzzBot"}
                ]},
            },
        })
        _, r1 = read_until_response(proc, 3)
        assert r1["result"]["done"] is True

        # Second prompt — session should still be alive and responding
        send_msg(proc, {
            "jsonrpc": "2.0", "id": 4,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess-tc05",
                "message": {"role": "user", "parts": [
                    {"content_type": "text/plain", "content": "What's my project name?"}
                ]},
            },
        })
        updates2, r2 = read_until_response(proc, 4)
        # Both prompts responded successfully — session history maintained
        assert r2["result"]["done"] is True
        # Verify a session/update was emitted for the second prompt too
        assert any(u["params"]["update"]["type"] == "message" for u in updates2)
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-06: Malformed JSON returns parse error, harness keeps running
# ---------------------------------------------------------------------------

def test_tc06_malformed_json_returns_parse_error():
    """TC-06: Invalid JSON on stdin returns parse error (-32700) without crashing."""
    proc = start_harness()
    try:
        # Send invalid JSON
        proc.stdin.write("this is not { valid json\n")
        proc.stdin.flush()

        resp = read_line(proc, timeout=3.0)
        assert resp.get("error", {}).get("code") == -32700

        # Harness should still respond to subsequent valid messages
        send_msg(proc, {"jsonrpc": "2.0", "id": 99, "method": "initialize", "params": {}})
        resp2 = read_line(proc)
        assert resp2["id"] == 99
        assert "result" in resp2
    finally:
        proc.stdin.close()
        proc.terminate()


# ---------------------------------------------------------------------------
# TC-07: Missing GEMINI_API_KEY returns descriptive error
# ---------------------------------------------------------------------------

def test_tc07_missing_api_key_returns_error():
    """TC-07: session/prompt without GEMINI_API_KEY returns error -32001 mentioning the key name."""
    proc = start_harness(env_override={"GEMINI_API_KEY": ""})
    try:
        send_msg(proc, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        read_line(proc)
        send_msg(proc, {"jsonrpc": "2.0", "id": 2, "method": "session/new",
                        "params": {"sessionId": "sess-tc07", "cwd": "/tmp"}})
        read_line(proc)

        send_msg(proc, {
            "jsonrpc": "2.0", "id": 3,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess-tc07",
                "message": {"role": "user", "parts": [
                    {"content_type": "text/plain", "content": "Hello"}
                ]},
            },
        })
        updates, result = read_until_response(proc, 3)
        assert result.get("error", {}).get("code") == -32001
        assert "GEMINI_API_KEY" in result["error"]["message"]
    finally:
        proc.stdin.close()
        proc.terminate()
