#!/usr/bin/env python3
"""Verify the admitted LM Studio instance emits a structured tool call."""

from __future__ import annotations

import argparse
import json
import urllib.request


def request_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=900) as response:
        return json.load(response)


def validate_catalog(catalog: dict, instance_id: str) -> None:
    loaded = [
        instance
        for model in catalog["models"]
        if model.get("type") == "llm"
        for instance in model.get("loaded_instances", [])
    ]
    if (
        len(loaded) != 1
        or loaded[0].get("id") != instance_id
        or loaded[0].get("config", {}).get("context_length") != 65536
        or loaded[0].get("config", {}).get("parallel") != 1
    ):
        raise RuntimeError("tool canary requires exactly one admitted LLM instance")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", default="http://127.0.0.1:1234")
    parser.add_argument("--instance", default="gemma4-26b-official")
    args = parser.parse_args()

    validate_catalog(request_json(f"{args.endpoint}/api/v1/models"), args.instance)

    body = {
        "model": args.instance,
        "messages": [
            {
                "role": "user",
                "content": "Use the readiness tool for command-adviser. Do not answer directly.",
            }
        ],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "lookup_readiness",
                    "description": "Look up readiness for a named system.",
                    "parameters": {
                        "type": "object",
                        "properties": {"system": {"type": "string"}},
                        "required": ["system"],
                        "additionalProperties": False,
                    },
                },
            }
        ],
        "tool_choice": "required",
        "reasoning_effort": "none",
        "temperature": 0,
    }
    request = urllib.request.Request(
        f"{args.endpoint}/v1/chat/completions",
        data=json.dumps(body).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=900) as response:
        result = json.load(response)

    choice = result["choices"][0]
    calls = choice["message"].get("tool_calls", [])
    if choice.get("finish_reason") != "tool_calls" or len(calls) != 1:
        raise RuntimeError("model did not emit exactly one structured tool call")
    function = calls[0].get("function", {})
    arguments = json.loads(function.get("arguments", "{}"))
    reasoning_tokens = (
        result.get("usage", {})
        .get("completion_tokens_details", {})
        .get("reasoning_tokens")
    )
    if (
        result.get("model") != args.instance
        or reasoning_tokens != 0
        or function.get("name") != "lookup_readiness"
        or arguments != {"system": "command-adviser"}
    ):
        raise RuntimeError("structured tool call did not match the requested contract")
    validate_catalog(request_json(f"{args.endpoint}/api/v1/models"), args.instance)

    print(
        json.dumps(
            {
                "instanceId": args.instance,
                "finishReason": choice["finish_reason"],
                "function": function["name"],
                "arguments": arguments,
                "reasoningTokens": reasoning_tokens,
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
    raise SystemExit(main())
