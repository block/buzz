from __future__ import annotations

import json
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any


@dataclass
class FixtureState:
    fault: str = ""
    requests: list[dict[str, Any]] = field(default_factory=list)
    poison_liveness: bool = False
    concurrent: dict[str, threading.Event] = field(
        default_factory=lambda: {
            "REG17-CONCURRENT-A-ec11": threading.Event(),
            "REG17-CONCURRENT-B-52d8": threading.Event(),
        }
    )


class _Server(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address: tuple[str, int], state: FixtureState) -> None:
        super().__init__(address, _Handler)
        self.state = state


class _Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    @property
    def state(self) -> FixtureState:
        return self.server.state  # type: ignore[attr-defined,no-any-return]

    def log_message(self, format: str, *args: object) -> None:
        return

    def _json(self, status: int, body: dict[str, Any]) -> None:
        encoded = json.dumps(body, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def _sse(self, nonce: str, *, invalid: bool = False) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Connection", "close")
        self.end_headers()
        if invalid:
            self.wfile.write(b"data: {not-json}\n\n")
            self.wfile.flush()
            return
        midpoint = max(1, len(nonce) // 2)
        for content in (nonce[:midpoint], nonce[midpoint:]):
            frame = {"choices": [{"delta": {"content": content}}]}
            self.wfile.write(f"data: {json.dumps(frame)}\n\n".encode())
            self.wfile.flush()
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()

    @staticmethod
    def _text(payload: dict[str, Any]) -> str:
        return " ".join(
            str(message.get("content", "")) for message in payload.get("messages", [])
        )

    @staticmethod
    def _nonce(text: str) -> str:
        return next((word.strip() for word in text.split() if word.startswith("REG17-")), "")

    def do_POST(self) -> None:
        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length))
        self.state.requests.append(
            {"authorization": self.headers.get("Authorization", ""), "payload": payload}
        )
        text = self._text(payload)
        nonce = self._nonce(text)

        if self.state.fault == "timeout" and "REG17-BASIC" in text:
            time.sleep(0.2)
        if self.state.fault == "429" and "REG17-BASIC" in text:
            self._json(429, {"error": "rate limited"})
            return
        if self.state.fault == "5xx" and "REG17-BASIC" in text:
            self._json(503, {"error": "unavailable"})
            return
        if self.state.poison_liveness and "REG17-LIVE" in text:
            self._json(503, {"error": "cancel poisoned server"})
            return

        if payload.get("stream"):
            if "REG17-CANCEL" in text and self.state.fault == "cancellation":
                self.state.poison_liveness = True
            self._sse(nonce, invalid=self.state.fault == "invalid_sse" and "STREAM" in text)
            return

        if "REG17-CONCURRENT" in nonce:
            other = (
                "REG17-CONCURRENT-B-52d8"
                if nonce.endswith("A-ec11")
                else "REG17-CONCURRENT-A-ec11"
            )
            self.state.concurrent[nonce].set()
            self.state.concurrent[other].wait(timeout=1)
            if self.state.fault == "crossed_nonces":
                nonce = other

        if payload.get("tools"):
            message = {
                "role": "assistant",
                "content": None,
                "tool_calls": [
                    {
                        "id": "fixture-call-1",
                        "type": "function",
                        "function": {
                            "name": "echo_probe",
                            "arguments": json.dumps({"nonce": nonce}),
                        },
                    }
                ],
            }
        elif payload.get("reasoning_effort") and self.state.fault == "reasoning_unsupported":
            self._json(400, {"error": "unsupported parameter"})
            return
        else:
            message = {"role": "assistant", "content": nonce}
        self._json(200, {"choices": [{"message": message}]})


@contextmanager
def openai_fixture(*, fault: str = "") -> Iterator[tuple[str, FixtureState]]:
    state = FixtureState(fault=fault)
    server = _Server(("127.0.0.1", 0), state)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        address = server.server_address
        yield f"http://{address[0]}:{address[1]}/v1", state
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
