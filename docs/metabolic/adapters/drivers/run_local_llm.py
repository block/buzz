#!/usr/bin/env python3
"""Hermetic local-llm cortex sink — prompt on stdin, draft on stdout.

Default backend: Ollama HTTP API (127.0.0.1:11434). No tools, no Buzz posts.

Env:
  BUZZ_DRIVER_LOCAL_LLM_MODEL     default gemma3:4b (must exist in `ollama list`)
  BUZZ_DRIVER_LOCAL_LLM_HOST      default http://127.0.0.1:11434
  BUZZ_DRIVER_LOCAL_LLM_TIMEOUT   seconds (default 90)
  BUZZ_DRIVER_LOCAL_LLM_NUM_PREDICT  max tokens (default 180)

Exit:
  0  draft (or NO_REPLY) on stdout
  2  config / unreachable backend
  3  model error
"""
from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.request


def main() -> int:
    prompt = sys.stdin.read()
    if not prompt.strip():
        print("NO_REPLY")
        return 0

    model = os.environ.get("BUZZ_DRIVER_LOCAL_LLM_MODEL") or "gemma3:4b"
    host = (os.environ.get("BUZZ_DRIVER_LOCAL_LLM_HOST") or "http://127.0.0.1:11434").rstrip(
        "/"
    )
    timeout = float(os.environ.get("BUZZ_DRIVER_LOCAL_LLM_TIMEOUT") or "90")
    try:
        num_predict = int(os.environ.get("BUZZ_DRIVER_LOCAL_LLM_NUM_PREDICT") or "180")
    except ValueError:
        num_predict = 180

    # System-style prefix reinforces untrusted-context (prompt already says it).
    full = prompt.strip() + "\n\nYour reply (or NO_REPLY):\n"

    body = {
        "model": model,
        "prompt": full,
        "stream": False,
        "options": {
            "num_predict": num_predict,
            "temperature": 0.2,
        },
    }
    url = f"{host}/api/generate"
    req = urllib.request.Request(
        url,
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as exc:
        err = exc.read().decode("utf-8", errors="replace")[:300]
        print(f"local-llm HTTP {exc.code}: {err}", file=sys.stderr)
        return 3
    except urllib.error.URLError as exc:
        print(f"local-llm unreachable at {host}: {exc.reason}", file=sys.stderr)
        return 2
    except TimeoutError:
        print(f"local-llm timeout after {timeout}s", file=sys.stderr)
        return 2

    try:
        data = json.loads(raw)
    except json.JSONDecodeError:
        print("local-llm bad JSON response", file=sys.stderr)
        return 3

    if data.get("error"):
        print(f"local-llm error: {data.get('error')}", file=sys.stderr)
        return 3

    text = (data.get("response") or "").strip()
    if not text:
        print("NO_REPLY")
        return 0

    # Collapse whitespace; cap for phone-safe Buzz drafts
    flat = " ".join(text.split())
    if len(flat) > 800:
        flat = flat[:797] + "..."
    # Reject obvious tool-call / code-execution patterns from model
    lower = flat.lower()
    if any(
        bad in lower
        for bad in (
            "```bash",
            "rm -rf",
            "curl http",
            "export buzz_private",
            "tool_call",
        )
    ):
        print("NO_REPLY")
        return 0

    print(flat)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
