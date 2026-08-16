#!/usr/bin/env python3
"""Prove three overlapping submissions execute serially on one LM Studio slot."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import threading
import time
import urllib.request


def request_json(url: str) -> dict:
    with urllib.request.urlopen(url, timeout=900) as response:
        return json.load(response)


def loaded_instances(catalog: dict) -> list[dict]:
    return [
        instance
        for model in catalog["models"]
        for instance in model.get("loaded_instances", [])
    ]


def validate_catalog(catalog: dict, instance_id: str) -> None:
    loaded = loaded_instances(catalog)
    if (
        len(loaded) != 1
        or loaded[0].get("id") != instance_id
        or loaded[0].get("config", {}).get("context_length") != 65536
        or loaded[0].get("config", {}).get("parallel") != 1
    ):
        raise RuntimeError("queue canary requires exactly one admitted parallel-one instance")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", default="http://127.0.0.1:1234")
    parser.add_argument("--instance", default="gemma4-26b-official")
    args = parser.parse_args()

    validate_catalog(request_json(f"{args.endpoint}/api/v1/models"), args.instance)
    first_output = threading.Event()

    def run(sequence: int, long_running: bool = False) -> dict:
        prompt = (
            "Write the integers 1 through 700, separated by spaces."
            if long_running
            else f"Reply exactly QUEUE-{sequence}"
        )
        body = {
            "model": args.instance,
            "input": prompt,
            "system_prompt": "Follow the user instruction exactly.",
            "stream": True,
            "reasoning": "off",
            "max_output_tokens": 2048 if long_running else 64,
            "context_length": 65536,
            "store": False,
        }
        request = urllib.request.Request(
            f"{args.endpoint}/api/v1/chat",
            data=json.dumps(body).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        submitted = time.monotonic()
        first_delta_at = None
        terminal = None
        with urllib.request.urlopen(request, timeout=900) as response:
            for raw_line in response:
                line = raw_line.decode("utf-8").strip()
                if not line.startswith("data: "):
                    continue
                event = json.loads(line[6:])
                if event.get("type") == "message.delta" and first_delta_at is None:
                    first_delta_at = time.monotonic()
                    if long_running:
                        first_output.set()
                if event.get("type") == "chat.end":
                    terminal = event["result"]
        finished = time.monotonic()
        if first_delta_at is None or terminal is None:
            raise RuntimeError(f"queue request {sequence} returned no terminal stream")
        message = "\n".join(
            item["content"]
            for item in terminal["output"]
            if item.get("type") == "message"
        )
        if (
            terminal.get("model_instance_id") != args.instance
            or terminal.get("stats", {}).get("reasoning_output_tokens") != 0
            or (not long_running and message != f"QUEUE-{sequence}")
        ):
            raise RuntimeError(f"queue request {sequence} failed")
        return {
            "request": sequence,
            "submittedAt": submitted,
            "firstOutputAt": first_delta_at,
            "finishedAt": finished,
        }

    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        first = executor.submit(run, 1, True)
        if not first_output.wait(timeout=120):
            raise RuntimeError("first queue request did not begin generation")
        second = executor.submit(run, 2)
        time.sleep(0.1)
        third = executor.submit(run, 3)
        results = [first.result(), second.result(), third.result()]

    ordered = sorted(results, key=lambda item: item["firstOutputAt"])
    generation_order = [item["request"] for item in ordered]
    if generation_order != [1, 2, 3]:
        raise RuntimeError(
            f"LM Studio did not preserve FIFO generation order: {generation_order}"
        )
    for previous, current in zip(ordered, ordered[1:]):
        if current["firstOutputAt"] < previous["finishedAt"]:
            raise RuntimeError("LM Studio generated two queue requests concurrently")

    validate_catalog(request_json(f"{args.endpoint}/api/v1/models"), args.instance)
    print(
        json.dumps(
            {
                "instanceId": args.instance,
                "submittedRequests": 3,
                "generationCapacity": 1,
                "generationOrder": generation_order,
                "nonOverlappingGeneration": True,
                "requests": [
                    {
                        "request": item["request"],
                        "queueWaitSeconds": round(
                            item["firstOutputAt"] - item["submittedAt"], 3
                        ),
                        "generationSeconds": round(
                            item["finishedAt"] - item["firstOutputAt"], 3
                        ),
                    }
                    for item in results
                ],
                "secondInstanceObserved": False,
                "result": "pass",
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
