#!/usr/bin/env python3
"""Antigravity ACP Harness for Buzz Desktop with Antigravity Models Suite.

This script implements the Agent Client Protocol (ACP) over stdin/stdout,
enabling the Antigravity agent (powered by Google DeepMind Antigravity AI suite)
to work as a local harness inside the Buzz Desktop application.

Supported Models (from Antigravity Suite):
- Gemini 3.6 Flash (High / Medium / Low)
- Gemini 3.5 Flash (High / Medium / Low)
- Gemini 3.1 Pro (High / Low)
- Claude Sonnet 4.6 (Thinking)
- Claude Opus 4.6 (Thinking)
- GPT-OSS 120B (Medium)
"""

import argparse
import json
import logging
import os
import sys
import threading
from pathlib import Path
from typing import Any, Dict, List, Optional

# ---------------------------------------------------------------------------
# Force UTF-8 encoding for stdout/stderr on Windows (prevents charmap errors)
# ---------------------------------------------------------------------------
if sys.platform == "win32":
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if hasattr(sys.stderr, "reconfigure"):
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")

# ---------------------------------------------------------------------------
# Logging (to stderr so stdout stays clean for ACP JSON-RPC)
# ---------------------------------------------------------------------------
logging.basicConfig(
    stream=sys.stderr,
    level=logging.INFO,
    format="%(asctime)s [antigravity-acp] %(levelname)s %(message)s",
)
log = logging.getLogger("antigravity")

# ---------------------------------------------------------------------------
# Token & Path Constants
# ---------------------------------------------------------------------------
TOKEN_PATH = Path.home() / ".buzz" / "antigravity" / "tokens.json"

DEFAULT_SCOPES = [
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/generative-language",
]

# ---------------------------------------------------------------------------
# Antigravity Model Catalog (matching Antigravity IDE / Platform options)
# ---------------------------------------------------------------------------
AVAILABLE_MODELS = [
    {
        "modelId": "gemini-3.6-flash",
        "name": "Gemini 3.6 Flash (High)",
        "description": "State-of-the-art high performance Antigravity model (Default)",
    },
    {
        "modelId": "gemini-3.6-flash-medium",
        "name": "Gemini 3.6 Flash (Medium)",
        "description": "Fast & balanced Antigravity model",
    },
    {
        "modelId": "gemini-3.6-flash-low",
        "name": "Gemini 3.6 Flash (Low)",
        "description": "Ultra low-latency Antigravity model",
    },
    {
        "modelId": "gemini-3.5-flash",
        "name": "Gemini 3.5 Flash (High)",
        "description": "High speed Antigravity Gemini 3.5 model",
    },
    {
        "modelId": "gemini-3.5-flash-medium",
        "name": "Gemini 3.5 Flash (Medium)",
        "description": "Balanced Gemini 3.5 Flash model",
    },
    {
        "modelId": "gemini-3.5-flash-low",
        "name": "Gemini 3.5 Flash (Low)",
        "description": "Fast Gemini 3.5 Flash model",
    },
    {
        "modelId": "gemini-3.1-pro-preview",
        "name": "Gemini 3.1 Pro (High)",
        "description": "Deep reasoning and complex task model",
    },
    {
        "modelId": "gemini-3.1-pro-low",
        "name": "Gemini 3.1 Pro (Low)",
        "description": "Fast reasoning Gemini 3.1 Pro model",
    },
    {
        "modelId": "claude-sonnet-4-6",
        "name": "Claude Sonnet 4.6 (Thinking)",
        "description": "Advanced thinking and coding model",
    },
    {
        "modelId": "claude-opus-4-6",
        "name": "Claude Opus 4.6 (Thinking)",
        "description": "Deep reasoning thinking model",
    },
    {
        "modelId": "gpt-oss-120b",
        "name": "GPT-OSS 120B (Medium)",
        "description": "Open source 120B parameter model",
    },
]

DEFAULT_MODEL = os.environ.get("GEMINI_MODEL", "gemini-3.6-flash")

SYSTEM_INSTRUCTION = (
    "You are Antigravity, an AI coding assistant powered by Google DeepMind, "
    "operating as a team member inside the Buzz workspace. "
    "You help with code, architecture decisions, debugging, pull request reviews, "
    "and technical discussions. Be concise, precise, and actionable. "
    "When you see @antigravity or @Rumble - An mentions, respond helpfully to the request. "
    "Format code with markdown code blocks."
)


# ---------------------------------------------------------------------------
# Google Auth Manager (OAuth2 + ADC + API Key Fallback)
# ---------------------------------------------------------------------------
class GoogleAuthManager:
    """Manages Google OAuth2 credentials, ADC, and API key fallbacks."""

    def __init__(self, token_path: Path = TOKEN_PATH):
        self.token_path = token_path

    def get_credentials(self) -> Any:
        """Resolve valid Google credentials from cache, ADC, or API Key."""
        # 1. Try cached OAuth tokens file
        if self.token_path.exists():
            try:
                from google.oauth2.credentials import Credentials
                from google.auth.transport.requests import Request

                log.info("Loading cached Google OAuth tokens from %s", self.token_path)
                creds = Credentials.from_authorized_user_file(
                    str(self.token_path), scopes=DEFAULT_SCOPES
                )
                if creds.valid:
                    return creds
                if creds.expired and creds.refresh_token:
                    log.info("Refreshing expired Google OAuth tokens...")
                    creds.refresh(Request())
                    self.save_tokens(creds)
                    return creds
            except Exception as e:
                log.warning("Failed to load/refresh cached tokens: %s", e)

        # 2. Try Application Default Credentials (ADC)
        try:
            import google.auth

            log.info("Attempting Application Default Credentials (ADC)...")
            adc_creds, _ = google.auth.default(scopes=DEFAULT_SCOPES)
            if adc_creds and adc_creds.valid:
                log.info("Successfully acquired valid ADC credentials")
                return adc_creds
        except Exception as e:
            log.debug("ADC not available: %s", e)

        # 3. Fallback: API Key if set
        api_key = os.environ.get("GEMINI_API_KEY")
        if api_key:
            log.info("Using GEMINI_API_KEY from environment")
            return api_key

        raise PermissionError(
            "No valid Google credentials found. Please click 'Login with Google' "
            "or run with --login to authenticate your Google Account."
        )

    def save_tokens(self, creds: Any) -> None:
        """Persist credentials to user tokens.json file."""
        self.token_path.parent.mkdir(parents=True, exist_ok=True)
        if hasattr(creds, "to_json"):
            token_data = json.loads(creds.to_json())
            with open(self.token_path, "w", encoding="utf-8") as f:
                json.dump(token_data, f, indent=2)
            log.info("Saved Google OAuth tokens to %s", self.token_path)

    def run_interactive_login(self) -> None:
        """Run interactive PKCE browser authorization flow."""
        log.info("Starting Google OAuth2 browser login flow...")

        try:
            from google_auth_oauthlib.flow import InstalledAppFlow

            client_config = {
                "installed": {
                    "client_id": os.environ.get(
                        "GOOGLE_OAUTH_CLIENT_ID",
                        "939886221528-97k9g6j88b4p0p5k8935c1n38j4p1b5a.apps.googleusercontent.com",
                    ),
                    "client_secret": os.environ.get("GOOGLE_OAUTH_CLIENT_SECRET", ""),
                    "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                    "token_uri": "https://oauth2.googleapis.com/token",
                    "redirect_uris": ["http://localhost"],
                }
            }

            flow = InstalledAppFlow.from_client_config(client_config, scopes=DEFAULT_SCOPES)
            log.info("Opening browser for Google login...")
            creds = flow.run_local_server(
                host="localhost",
                port=0,
                authorization_prompt_message="Opening browser for Google authorization...",
                success_message="Antigravity: Authentication successful! You may close this window.",
                open_browser=True,
            )
            self.save_tokens(creds)
            print("Successfully authenticated Google Account!")
        except Exception as e:
            log.error("Interactive login failed: %s", e)
            sys.exit(1)


auth_mgr = GoogleAuthManager()

# ---------------------------------------------------------------------------
# Gemini client singleton (new google-genai SDK)
# ---------------------------------------------------------------------------
_genai_client = None


def get_client():
    global _genai_client
    if _genai_client is not None:
        return _genai_client

    try:
        from google import genai

        creds = auth_mgr.get_credentials()

        if isinstance(creds, str):
            _genai_client = genai.Client(api_key=creds)
        else:
            _genai_client = genai.Client(credentials=creds)

        log.info("Gemini client initialized with Antigravity Google credentials")
        return _genai_client
    except Exception as e:
        raise RuntimeError(f"Authentication failed: {e}")


# ---------------------------------------------------------------------------
# Session state — maintains conversation history and active model per session
# ---------------------------------------------------------------------------
class SessionStore:
    def __init__(self):
        self._lock = threading.Lock()
        self._sessions: dict[str, list[dict]] = {}
        self._cancel_flags: dict[str, bool] = {}
        self._session_models: dict[str, str] = {}

    def get_history(self, session_id: str) -> list[dict]:
        with self._lock:
            return list(self._sessions.get(session_id, []))

    def append(self, session_id: str, role: str, text: str):
        with self._lock:
            if session_id not in self._sessions:
                self._sessions[session_id] = []
            self._sessions[session_id].append({"role": role, "parts": [{"text": text}]})

    def new_session(self, session_id: str):
        with self._lock:
            self._sessions[session_id] = []
            self._cancel_flags[session_id] = False
            self._session_models[session_id] = DEFAULT_MODEL

    def set_model(self, session_id: str, model_id: str):
        with self._lock:
            self._session_models[session_id] = model_id

    def get_model(self, session_id: str) -> str:
        with self._lock:
            return self._session_models.get(session_id, DEFAULT_MODEL)

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
    """Send a session/update notification (agent -> harness -> relay)."""
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
# Model resolution helper (handles Antigravity aliases and fallback)
# ---------------------------------------------------------------------------
MODEL_FALLBACKS = {
    "gemini-3.6-flash-medium": "gemini-3.6-flash",
    "gemini-3.6-flash-low": "gemini-3.6-flash",
    "gemini-3.5-flash-medium": "gemini-3.5-flash",
    "gemini-3.5-flash-low": "gemini-3.5-flash",
    "gemini-3.1-pro-low": "gemini-3.1-pro-preview",
    "claude-sonnet-4-6": "gemini-3.6-flash",
    "claude-opus-4-6": "gemini-3.6-flash",
    "gpt-oss-120b": "gemini-3.6-flash",
}


def resolve_api_model(model_id: str) -> str:
    """Map Antigravity model selection to backend API model target."""
    return MODEL_FALLBACKS.get(model_id, model_id)


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
            "model": DEFAULT_MODEL,
        },
        "authMethods": [
            {
                "id": "google-account",
                "type": "terminal",
                "name": "Google Account",
                "description": "Login using your Google Account credentials (browser PKCE)",
            }
        ],
    })


def handle_session_new(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    sessions.new_session(session_id)
    current_model = sessions.get_model(session_id)
    log.info("New session: %s (model: %s)", session_id, current_model)

    send_response(msg_id, {
        "sessionId": session_id,
        "models": {
            "currentModelId": current_model,
            "availableModels": AVAILABLE_MODELS,
        },
    })


def handle_session_set_model(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    model_id = params.get("modelId", DEFAULT_MODEL)
    sessions.set_model(session_id, model_id)
    log.info("Set model for session [%s]: %s", session_id, model_id)
    send_response(msg_id, {
        "sessionId": session_id,
        "modelId": model_id,
    })


def handle_session_prompt(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    log.info("Raw prompt params [%s]: %s", session_id, json.dumps(params, ensure_ascii=True))

    if sessions.is_cancelled(session_id):
        send_error(msg_id, -32800, "Session was cancelled")
        return

    # Extract user text flexibly from ACP variants
    user_text = ""
    prompt_val = params.get("prompt")
    if isinstance(prompt_val, str):
        user_text = prompt_val
    elif isinstance(prompt_val, list):
        text_parts = []
        for p in prompt_val:
            if isinstance(p, str):
                text_parts.append(p)
            elif isinstance(p, dict):
                text_parts.append(p.get("text") or p.get("content") or p.get("value") or "")
        user_text = " ".join(text_parts)
    elif isinstance(prompt_val, dict):
        user_text = prompt_val.get("text") or prompt_val.get("content") or ""

    if not user_text:
        msg_val = params.get("message")
        if isinstance(msg_val, str):
            user_text = msg_val
        elif isinstance(msg_val, dict):
            parts = msg_val.get("parts", [])
            if isinstance(parts, list):
                text_parts = []
                for p in parts:
                    if isinstance(p, str):
                        text_parts.append(p)
                    elif isinstance(p, dict):
                        text_parts.append(p.get("text") or p.get("content") or p.get("value") or "")
                user_text = " ".join(text_parts)
            else:
                user_text = msg_val.get("content") or msg_val.get("text") or ""

    if not user_text:
        user_text = params.get("text") or params.get("content") or ""

    user_text = user_text.strip()

    if not user_text:
        log.error("Empty prompt after extraction. Full params: %s", json.dumps(params, ensure_ascii=True))
        send_error(msg_id, -32602, "Empty prompt received")
        return

    log.info("Prompt [%s]: %s", session_id, user_text[:80].encode("ascii", "replace").decode("ascii"))

    # Test stub mode
    stub_response = os.environ.get("HARNESS_STUB_RESPONSE")
    if stub_response:
        sessions.append(session_id, "user", user_text)
        sessions.append(session_id, "model", stub_response)
        send_update(session_id, "message", stub_response)
        send_response(msg_id, {"done": True})
        return

    # Get Gemini client
    try:
        client = get_client()
    except Exception as e:
        send_error(msg_id, -32001, str(e))
        return

    raw_model_choice = sessions.get_model(session_id)
    target_api_model = resolve_api_model(raw_model_choice)

    sessions.append(session_id, "user", user_text)
    history = sessions.get_history(session_id)

    try:
        send_update(session_id, "status", "Thinking...")

        from google.genai import types

        response = client.models.generate_content(
            model=target_api_model,
            contents=history,
            config=types.GenerateContentConfig(
                system_instruction=SYSTEM_INSTRUCTION,
            ),
        )
        answer = response.text or "Sem resposta do modelo."

        if sessions.is_cancelled(session_id):
            send_error(msg_id, -32800, "Session was cancelled")
            return

        sessions.append(session_id, "model", answer)
        send_update(session_id, "message", answer)

        log.info("Response [%s] (%s -> %s): %d chars", session_id, raw_model_choice, target_api_model, len(answer))
        send_response(msg_id, {"done": True})

    except Exception as e:
        log.error("Gemini error [%s]: %s", session_id, str(e).encode("ascii", "replace").decode("ascii"))
        send_error(msg_id, -32603, f"Gemini API error: {e}")


def handle_session_cancel(msg_id: Any, params: dict) -> None:
    session_id = params.get("sessionId", "default")
    sessions.cancel(session_id)
    log.info("Cancelled session: %s", session_id)
    send_response(msg_id, {})


# ---------------------------------------------------------------------------
# Main event loop & CLI
# ---------------------------------------------------------------------------
HANDLERS = {
    "initialize": handle_initialize,
    "session/new": handle_session_new,
    "session/set_model": handle_session_set_model,
    "session/prompt": handle_session_prompt,
    "session/cancel": handle_session_cancel,
}


def main():
    parser = argparse.ArgumentParser(description="Antigravity ACP Harness")
    parser.add_argument("--login", action="store_true", help="Run interactive Google Account OAuth login flow")
    args, _ = parser.parse_known_args()

    if args.login:
        auth_mgr.run_interactive_login()
        return

    log.info("Antigravity ACP Harness starting (relay: %s)",
             os.environ.get("BUZZ_RELAY_URL", "ws://192.168.1.80:3001"))

    for raw_line in sys.stdin:
        raw_line = raw_line.strip()
        if not raw_line:
            continue

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
