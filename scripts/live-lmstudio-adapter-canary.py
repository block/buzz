#!/usr/bin/env python3
"""Drive the release Buzz LM Studio adapter over ACP against loopback."""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import threading
import urllib.request


INSTANCE_ID = "gemma4-26b-official"


def validate_catalog() -> None:
    with urllib.request.urlopen("http://127.0.0.1:1234/api/v1/models", timeout=30) as response:
        catalog = json.load(response)
    loaded = [
        instance
        for model in catalog["models"]
        for instance in model.get("loaded_instances", [])
    ]
    if (
        len(loaded) != 1
        or loaded[0].get("id") != INSTANCE_ID
        or loaded[0].get("config", {}).get("context_length") != 65536
        or loaded[0].get("config", {}).get("parallel") != 1
    ):
        raise RuntimeError("adapter canary requires exactly one admitted instance")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--image", required=True)
    parser.add_argument("--cwd", required=True)
    args = parser.parse_args()
    validate_catalog()

    env = os.environ.copy()
    env.update(
        {
            "BUZZ_AGENT_PROVIDER": "lmstudio-native",
            "BUZZ_AGENT_MAX_CONTEXT_TOKENS": "65536",
            "BUZZ_AGENT_MAX_OUTPUT_TOKENS": "8192",
            "BUZZ_AGENT_LLM_TIMEOUT_SECS": "900",
            "BUZZ_AGENT_MAX_SESSIONS": "8",
            "BUZZ_AGENT_NO_HINTS": "1",
            "LM_STUDIO_BASE_URL": "http://127.0.0.1:1234",
            "LM_STUDIO_MODEL": INSTANCE_ID,
            "LM_STUDIO_MCP_INTEGRATIONS": "[]",
            "LM_STUDIO_REASONING": "off",
        }
    )
    env.pop("LM_STUDIO_FALLBACK_PROVIDER", None)

    child = subprocess.Popen(
        [args.binary],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
        env=env,
    )
    assert child.stdin is not None
    assert child.stdout is not None

    next_id = 1

    def call(method: str, params: dict) -> tuple[dict, str, list[str]]:
        nonlocal next_id
        request_id = next_id
        next_id += 1
        child.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": method,
                    "params": params,
                }
            )
            + "\n"
        )
        child.stdin.flush()
        messages: list[str] = []
        thoughts: list[str] = []
        while True:
            line = child.stdout.readline()
            if not line:
                stderr = child.stderr.read() if child.stderr is not None else ""
                raise RuntimeError(f"adapter exited before response: {stderr[-1000:]}")
            value = json.loads(line)
            if value.get("id") == request_id:
                return value, "".join(messages), thoughts
            update = value.get("params", {}).get("update", {})
            content = update.get("content", {})
            if update.get("sessionUpdate") == "agent_message_chunk":
                messages.append(content.get("text", ""))
            elif update.get("sessionUpdate") == "agent_thought_chunk":
                thoughts.append(content.get("text", ""))

    def cancelled_prompt(session_id: str) -> dict:
        nonlocal next_id
        request_id = next_id
        next_id += 1
        child.stdin.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "session/prompt",
                    "params": {
                        "sessionId": session_id,
                        "prompt": [
                            {
                                "type": "text",
                                "text": "Write the integers from 1 through 4000, one per line.",
                            }
                        ],
                    },
                }
            )
            + "\n"
        )
        child.stdin.flush()

        def cancel() -> None:
            child.stdin.write(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "method": "session/cancel",
                        "params": {"sessionId": session_id},
                    }
                )
                + "\n"
            )
            child.stdin.flush()

        timer = threading.Timer(0.25, cancel)
        timer.start()
        try:
            while True:
                line = child.stdout.readline()
                if not line:
                    stderr = child.stderr.read() if child.stderr is not None else ""
                    raise RuntimeError(
                        f"adapter exited before cancellation response: {stderr[-1000:]}"
                    )
                value = json.loads(line)
                if value.get("id") == request_id:
                    return value
        finally:
            timer.cancel()

    initialize, _, _ = call(
        "initialize", {"protocolVersion": 2, "clientCapabilities": {}}
    )
    if "error" in initialize:
        raise RuntimeError(initialize["error"])

    def new_session() -> str:
        response, _, _ = call(
            "session/new",
            {
                "cwd": os.path.abspath(args.cwd),
                "mcpServers": [],
                "systemPrompt": "Follow the user instruction exactly.",
            },
        )
        if "error" in response:
            raise RuntimeError(response["error"])
        return response["result"]["sessionId"]

    text_session = new_session()
    text_response, text, text_thoughts = call(
        "session/prompt",
        {
            "sessionId": text_session,
            "prompt": [{"type": "text", "text": "Reply exactly BUZZ ADAPTER READY"}],
        },
    )
    if "error" in text_response or text.strip() != "BUZZ ADAPTER READY" or text_thoughts:
        raise RuntimeError(
            f"text canary failed: response={text_response!r} text={text!r} thoughts={len(text_thoughts)}"
        )

    json_session = new_session()
    json_response, json_text, json_thoughts = call(
        "session/prompt",
        {
            "sessionId": json_session,
            "prompt": [
                {
                    "type": "text",
                    "text": 'Return exactly {"status":"ready","slots":1} with no markdown or extra text.',
                }
            ],
        },
    )
    try:
        parsed_json = json.loads(json_text)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"strict JSON canary failed: {error}") from error
    if (
        "error" in json_response
        or parsed_json != {"status": "ready", "slots": 1}
        or json_thoughts
    ):
        raise RuntimeError(
            f"strict JSON canary failed: response={json_response!r} thoughts={len(json_thoughts)}"
        )

    continuation_session = new_session()
    stored_response, stored_text, stored_thoughts = call(
        "session/prompt",
        {
            "sessionId": continuation_session,
            "prompt": [
                {
                    "type": "text",
                    "text": "Remember the codeword ANCHOR-642. Reply exactly STORED",
                }
            ],
        },
    )
    recalled_response, recalled_text, recalled_thoughts = call(
        "session/prompt",
        {
            "sessionId": continuation_session,
            "prompt": [
                {
                    "type": "text",
                    "text": "Reply with only the codeword I asked you to remember.",
                }
            ],
        },
    )
    if (
        "error" in stored_response
        or stored_text.strip() != "STORED"
        or stored_thoughts
        or "error" in recalled_response
        or recalled_text.strip() != "ANCHOR-642"
        or recalled_thoughts
    ):
        raise RuntimeError("stateful continuation canary failed")

    with open(args.image, "rb") as image_file:
        image_data = base64.b64encode(image_file.read()).decode("ascii")
    image_session = new_session()
    image_response, image_text, image_thoughts = call(
        "session/prompt",
        {
            "sessionId": image_session,
            "prompt": [
                {
                    "type": "text",
                    "text": "Read the ship name on the badge. Reply exactly Supply",
                },
                {"type": "image", "data": image_data, "mimeType": "image/png"},
            ],
        },
    )
    if (
        "error" in image_response
        or image_text.strip() != "Supply"
        or image_thoughts
    ):
        raise RuntimeError(
            f"image canary failed: response={image_response!r} text={image_text!r} thoughts={len(image_thoughts)}"
        )

    cancellation_session = new_session()
    cancelled_response = cancelled_prompt(cancellation_session)
    if cancelled_response.get("result", {}).get("stopReason") != "cancelled":
        raise RuntimeError(f"cancellation canary failed: {cancelled_response!r}")
    recovery_response, recovery_text, recovery_thoughts = call(
        "session/prompt",
        {
            "sessionId": cancellation_session,
            "prompt": [{"type": "text", "text": "Reply exactly RECOVERED"}],
        },
    )
    if (
        "error" in recovery_response
        or recovery_text.strip() != "RECOVERED"
        or recovery_thoughts
    ):
        raise RuntimeError("post-cancellation recovery canary failed")

    child.stdin.close()
    child.wait(timeout=10)
    validate_catalog()
    print(
        json.dumps(
            {
                "adapter": "buzz-lmstudio-agent",
                "model": "google/gemma-4-26b-a4b",
                "instanceId": INSTANCE_ID,
                "text": "pass",
                "strictJson": "pass",
                "statefulContinuation": "pass",
                "nativeImage": "pass",
                "cancellationRecovery": "pass",
                "reasoning": "off",
                "contextLength": 65536,
                "generationCapacity": 1,
                "secondInstanceObserved": False,
                "result": "pass",
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"live adapter canary failed: {error}", file=sys.stderr)
        raise SystemExit(1)
