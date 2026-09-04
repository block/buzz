#!/usr/bin/env python3
"""Deterministic verifier for a human-facing callback mention."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

USER_DISPLAY_NAME = "John Vincent Doe"
EXPECTED_TOTAL = 5_328.0
NUMBER = re.compile(r"(?<![A-Za-z0-9_])-?\$?\d[\d,]*(?:\.\d+)?")


def _numbers(content: str) -> list[float]:
    values: list[float] = []
    for token in NUMBER.findall(content):
        try:
            values.append(float(token.replace("$", "").replace(",", "")))
        except ValueError:
            continue
    return values


def _has_p_tag(message: dict[str, Any], pubkey: object) -> bool:
    return isinstance(pubkey, str) and any(
        isinstance(tag, list) and len(tag) >= 2 and tag[0] == "p" and tag[1] == pubkey
        for tag in message.get("tags", [])
    )


def _zero_metrics() -> dict[str, float]:
    return {
        "reward": 0.0,
        "answer_correct": 0.0,
        "user_p_tagged": 0.0,
        "three_word_user": 0.0,
        "same_channel": 0.0,
        "expected_author": 0.0,
        "evidence_complete": 0.0,
    }


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
        row
        for row in identities.values()
        if isinstance(row, dict) and row.get("role") == "orchestrator"
    ]
    users = [
        (name, row)
        for name, row in identities.items()
        if isinstance(row, dict) and row.get("role") == "user"
    ]
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    user_name, user = users[0] if len(users) == 1 else (None, {})
    user_pubkey = user.get("pubkey")
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
    content = str(final.get("content", "")) if final is not None else ""
    values = _numbers(content)

    three_word_user = float(
        user_name == USER_DISPLAY_NAME and len(USER_DISPLAY_NAME.split()) == 3
    )
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "user-mention"
        and evidence.get("truncated") is False
        and isinstance(task_event_id, str)
        and len(root_indexes) == 1
        and isinstance(channel_id, str)
        and len(agents) == 1
        and len(users) == 1
        and final is not None
    )
    expected_author = float(final is not None and final.get("pubkey") == agent_pubkey)
    same_channel = float(
        final is not None
        and final.get("channel_id") == channel_id
        and ["h", channel_id] in final.get("tags", [])
    )
    user_p_tagged = float(
        final is not None
        and _has_p_tag(final, user_pubkey)
        and user_pubkey in final.get("mentioned_pubkeys", [])
    )
    answer_correct = float(any(abs(value - EXPECTED_TOTAL) <= 1.0 for value in values))
    reward = float(
        all(
            metric == 1.0
            for metric in (
                evidence_complete,
                expected_author,
                same_channel,
                three_word_user,
                user_p_tagged,
                answer_correct,
            )
        )
    )
    metrics = {
        "reward": reward,
        "answer_correct": answer_correct,
        "user_p_tagged": user_p_tagged,
        "three_word_user": three_word_user,
        "same_channel": same_channel,
        "expected_author": expected_author,
        "evidence_complete": evidence_complete,
    }
    details = {
        "task_event_id": task_event_id,
        "selected_message_id": final.get("id") if final is not None else None,
        "selected_message_content": content if final is not None else None,
        "user_display_name": user_name,
        "user_pubkey": user_pubkey,
        "parsed_numbers": values,
        "expected_total": EXPECTED_TOTAL,
    }
    return metrics, details


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
