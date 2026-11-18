#!/usr/bin/env python3
"""Deterministic verifier for the Buzz reply-to-thread task."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

# Each amount must appear on a line that also names what it is. Scanning the
# whole reply for bare numbers passes a work-showing table whose month-6 rows
# are right but whose stated answer is wrong.
EXPECTED_ANSWERS = (
    ("revenue", 160_811.0),
    ("expense", 84_462.0),
    ("profit", 374_470.0),
)
EXPECTED_AMOUNTS = tuple(amount for _, amount in EXPECTED_ANSWERS)
NUMBER = re.compile(r"(?<![A-Za-z0-9_])-?\$?\d[\d,]*(?:\.\d+)?")


def _numbers(content: str) -> list[float]:
    values: list[float] = []
    for token in NUMBER.findall(content):
        try:
            values.append(float(token.replace("$", "").replace(",", "")))
        except ValueError:
            continue
    return values


def _contains_amount(values: list[float], expected: float) -> bool:
    return any(abs(value - expected) <= 1.0 for value in values)


def _labelled_amount(content: str, label: str, expected: float) -> bool:
    """Whether some line names ``label`` and carries ``expected`` on it."""
    return any(
        label in line.casefold() and _contains_amount(_numbers(line), expected)
        for line in content.splitlines()
    )


def _has_tag(message: dict[str, Any], expected: list[str]) -> bool:
    return any(tag == expected for tag in message.get("tags", []))


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero_metrics(), {"error": "evidence root is not an object"}

    task_event_id = evidence.get("task_event_id")
    trial = evidence.get("trial") if isinstance(evidence.get("trial"), dict) else {}
    channel_id = trial.get("channel_id")
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    agents = [
        identity
        for identity in identities.values()
        if isinstance(identity, dict) and identity.get("role") == "orchestrator"
    ]
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    messages = [
        message for message in evidence.get("messages", []) if isinstance(message, dict)
    ]
    root_indexes = [
        index
        for index, message in enumerate(messages)
        if message.get("id") == task_event_id
    ]
    root_index = root_indexes[0] if len(root_indexes) == 1 else -1
    candidates = [
        message
        for message in messages[root_index + 1 :]
        if agent_pubkey and message.get("pubkey") == agent_pubkey
    ]
    final = candidates[-1] if candidates else None

    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("truncated") is False
        and isinstance(task_event_id, str)
        and len(root_indexes) == 1
        and isinstance(channel_id, str)
        and len(agents) == 1
        and final is not None
    )
    expected_author = float(final is not None and final.get("pubkey") == agent_pubkey)
    same_channel = float(
        final is not None
        and final.get("channel_id") == channel_id
        and _has_tag(final, ["h", channel_id])
    )
    reply_to_thread = float(
        final is not None
        and final.get("reply_to_event_id") == task_event_id
        and _has_tag(final, ["e", task_event_id, "", "reply"])
    )
    content = str(final.get("content", "")) if final is not None else ""
    values = _numbers(content)
    answer_correct = float(
        all(
            _labelled_amount(content, label, expected)
            for label, expected in EXPECTED_ANSWERS
        )
    )
    reward = float(
        all(
            metric == 1.0
            for metric in (
                evidence_complete,
                expected_author,
                same_channel,
                reply_to_thread,
                answer_correct,
            )
        )
    )
    metrics = {
        "reward": reward,
        "answer_correct": answer_correct,
        "reply_to_thread": reply_to_thread,
        "same_channel": same_channel,
        "expected_author": expected_author,
        "evidence_complete": evidence_complete,
    }
    details = {
        "task_event_id": task_event_id,
        "selected_message_id": final.get("id") if final is not None else None,
        "selected_message_content": final.get("content") if final is not None else None,
        "parsed_numbers": values,
        "expected_amounts": list(EXPECTED_AMOUNTS),
    }
    return metrics, details


def _zero_metrics() -> dict[str, float]:
    return {
        "reward": 0.0,
        "answer_correct": 0.0,
        "reply_to_thread": 0.0,
        "same_channel": 0.0,
        "expected_author": 0.0,
        "evidence_complete": 0.0,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--reward", type=Path, required=True)
    parser.add_argument("--details", type=Path, required=True)
    args = parser.parse_args()

    try:
        evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
        metrics, details = score_evidence(evidence)
    except (OSError, json.JSONDecodeError) as error:
        metrics, details = _zero_metrics(), {"error": str(error)}

    args.reward.write_text(json.dumps(metrics, sort_keys=True) + "\n", encoding="utf-8")
    args.details.write_text(
        json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
