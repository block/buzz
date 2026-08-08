#!/usr/bin/env python3
"""
Antigravity ACP Harness for block/buzz
======================================
Bridges the buzz-acp relay bridge to Google's Gemini API.
Implements the Agent Client Protocol (ACP) via JSON-RPC 2.0 over stdio.

Protocol flow:
  buzz-acp  --[stdin JSON-RPC]-->  this harness  --[HTTPS]-->  Gemini API
  buzz-acp  <--[stdout JSON-RPC]-- this harness  <--[HTTPS]--  Gemini API
"""

import sys
import os
import json
import threading
import logging
from typing import Any

# Configure logging to stderr only (stdout is the ACP transport)
logging.basicConfig(
    stream=sys.stderr,
    level=logging.INFO,
    format="%(asctime)s [antigravity-acp] %(levelname)s %(message)s",
)
log = logging.getLogger("antigravity-acp")

# ---------------------------------------------------------------------------
# Gemini client (lazy-loaded so initialize works even without key)
# ---------------------------------------------------------------------------
_gemini_model = None

def get_gemini_model():
    global _gemini_model
    if _gemini_model is not None:
        return _gemini_model

    api_key = os.environ.get("GEMINI_API_KEY")
    if not api_key:
        raise RuntimeError(
            "GEMINI_API_KEY environment variable is not set. "
            "Get your key at https://aistudio.google.com/apikey"
        )

    try:
        import google.generativeai as genai
        genai.configure(api_key=api_key)
        _gemini_model = genai.GenerativeModel(
            model_name=os.environ.get("GEMINI_MODEL", "gemini-2.0-flash"),
            system_instruction=(
                "You are Antigravity, a powerful AI coding assistant by Google DeepMind, "
                "operating as a team member inside the Buzz workspace. "
                "You help with code, architecture decisions, debugging, pull request reviews, "
                "and technical discussions. Be concise, precise, and actionable. "
                "When you see @antigravity mentions, respond helpfully to the request. "
                "Format code with markdown code blocks."
            ),
        )
        log.info("Gemini model initialized: %s", os.environ.get("GEMINI_MODEL", "gemini-2.0-flash"))
        return _gemini_model
    except ImportError:
        raise RuntimeError(
            "google-generativeai package not installed. Run: pip install google-generativeai"
        )


# ---------------------------------------------------------------------------
# Session state — maintains conversation history per sessionId
# ---------------------------------------------------------------------------
class SessionStore:
    def __init__(self):
        self._lock = threading.Lock()
        self._sessions: dict[str, list[dict]] = {}
        self._cancel_flags: dict[str, bool] = {}

    def get_history(self, session_id: str) -> list[dict]:
        with self._lock:
            return list(self._sessions.get(session_id, []))

    def append(self, session_id: str, role: str, text: str):
        with self._lock:
            if session_id not in self._sessions:
                self._sessions[session_id] = []
            self._sessions[session_id].append({"role": role, "parts": [text]})

    def new_session(self, session_id: str):
        with self._lock:
            self._sessions[session_id] = []
            self._cancel_flags[session_id] = False

    def cancel(self, session_id: str):
        with self._lock:
            self._cancel_flags[session_id] = True

    def is_cancelled(self, session_id: str) -> bool:
        with self._lock:
            return self._cancel_flags.get(session_id, False)


sessions = SessionStore()

# ---------------------------------------------------------------------------
# ACP transport helpers
# ---------------------------------------------------------------------------
_stdout_lock = threading.Lock()


def send(obj: dict):
    """Write a JSON-RPC object to stdout (thread-safe)."""
    line = json.dumps(obj, ensure_ascii=False) + "\n"
    with _stdout_lock:
        sys.stdout.write(line)
        sys.stdout.flush()


def send_response(msg_id: Any, result: dict):
    send({"jsonrpc": "2.0", "id": msg_id, "result": result})


def send_error(msg_id: Any, code: int, message: str, data: Any = None):
    error: dict = {"code": code, "message": message}
    if data is not None:
        error["data"] = data
    send({"jsonrpc": "2.0", "id": msg_id, "error": error})


def send_update(session_id: str, update_type: str, content: str):
    """Send a session/update notification (agent → harness → relay)."""
    send({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "type": update_type,
                "role": "agent",
                "content": content,
            },
        },
    })


# ---------------------------------------------------------------------------
# JSON-RPC method handlers
# ---------------------------------------------------------------------------

def handle_initialize(msg_id: Any, params: dict) -> None:
    log.info("ACP initialize")
    send_response(msg_id, {
        "protocolVersion": "0.1",
        "capabilities": {
            "streaming": False,
            "sessionHistory": True,
            "cancellation": True,
        },
        "agentInfo": {
            "name": "Antigravity",
            "version": "0.1.0",
            "vendor": "Google DeepMind",
            "model": os.environ.get("GEMINI_MODEL", "gemini-2.0-flash"),
        },
    })


def handle_session_new(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    sessions.new_session(session_id)
    log.info("New session: %s", session_id)
    send_response(msg_id, {"sessionId": session_id})


def handle_session_prompt(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    message = params.get("message", {})
    parts = message.get("parts", [])

    # Extract text from parts
    user_text = " ".join(
        p.get("content", "") for p in parts
        if p.get("content_type", "text/plain") == "text/plain"
    ).strip()

    if not user_text:
        send_error(msg_id, -32602, "Empty prompt received")
        return

    log.info("Prompt [%s]: %s", session_id, user_text[:80])

    # Check for cancellation early
    if sessions.is_cancelled(session_id):
        send_error(msg_id, -32800, "Session was cancelled")
        return

    # --- Test stub mode: bypass real Gemini if HARNESS_STUB_RESPONSE is set ---
    stub_response = os.environ.get("HARNESS_STUB_RESPONSE")
    if stub_response:
        sessions.append(session_id, "user", user_text)
        sessions.append(session_id, "model", stub_response)
        send_update(session_id, "message", stub_response)
        send_response(msg_id, {"done": True})
        return
    # -------------------------------------------------------------------------

    # Validate Gemini is available
    try:
        model = get_gemini_model()
    except RuntimeError as e:
        send_error(msg_id, -32001, str(e))
        return

    # Append user message to history
    sessions.append(session_id, "user", user_text)

    # Build conversation history for Gemini
    history = sessions.get_history(session_id)
    # Remove the last user message (will be sent as the current prompt)
    chat_history = history[:-1]

    try:
        # Signal we're working
        send_update(session_id, "status", "🤔 Thinking...")

        # Call Gemini with conversation context
        chat = model.start_chat(history=chat_history)
        response = chat.send_message(user_text)
        answer = response.text

        if sessions.is_cancelled(session_id):
            send_error(msg_id, -32800, "Session was cancelled")
            return

        # Append agent response to history
        sessions.append(session_id, "model", answer)

        # Emit the answer as a session/update
        send_update(session_id, "message", answer)

        log.info("Response [%s]: %d chars", session_id, len(answer))
        send_response(msg_id, {"done": True})

    except Exception as e:
        log.error("Gemini error [%s]: %s", session_id, e)
        send_error(msg_id, -32603, f"Gemini API error: {e}")


def handle_session_cancel(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    sessions.cancel(session_id)
    log.info("Cancelled session: %s", session_id)
    send_response(msg_id, {})


# ---------------------------------------------------------------------------
# Main event loop
# ---------------------------------------------------------------------------
HANDLERS = {
    "initialize": handle_initialize,
    "session/new": handle_session_new,
    "session/prompt": handle_session_prompt,
    "session/cancel": handle_session_cancel,
}


def main():
    log.info("Antigravity ACP Harness starting (relay: %s)",
             os.environ.get("BUZZ_RELAY_URL", "not set"))

    for raw_line in sys.stdin:
        raw_line = raw_line.strip()
        if not raw_line:
            continue

        # Parse JSON-RPC
        try:
            msg = json.loads(raw_line)
        except json.JSONDecodeError as e:
            send_error(None, -32700, f"Parse error: {e}")
            continue

        msg_id = msg.get("id")
        method = msg.get("method", "")
        params = msg.get("params", {})

        handler = HANDLERS.get(method)
        if handler is None:
            send_error(msg_id, -32601, f"Method not found: {method}")
            continue

        try:
            handler(msg_id, params)
        except Exception as e:
            log.exception("Unhandled error in %s", method)
            send_error(msg_id, -32603, f"Internal error: {e}")


if __name__ == "__main__":
    main()
