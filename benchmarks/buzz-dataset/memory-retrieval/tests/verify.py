#!/usr/bin/env python3
"""Deterministic verifier for pre-seeded cold-memory retrieval."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

EXPECTED_CUSTOMERS = 352_345
ALLOWED_CONTEXT_NUMBERS = frozenset({2024})
# Numbers that appear only in the distractor memories. Mentioning any of them
# means the answer pulled from the wrong memory (or dumped several), so it does
# not demonstrate that the correct value was selected.
DISTRACTOR_NUMBERS = frozenset(
    {
        361_250,  # total-customers-per-month: monthly average
        351_340,  # customer-value-metric: last month's customers
        2_400,  # customer-value-metric: revenue per customer
        325_401,  # customers-metrics-spring-24: March total
        3_710,  # customers-metrics-spring-24: April active customers named John
        21_604,  # new-customers-april-2024: April new customers
    }
)
NUMBER = re.compile(r"(?<![A-Za-z0-9_])-?\d[\d,]*(?:\.\d+)?")


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "answer_correct": 0.0,
        "threaded_reply": 0.0,
        "evidence_complete": 0.0,
    }


def _numbers(content: str) -> list[float]:
    values: list[float] = []
    for token in NUMBER.findall(content):
        try:
            values.append(float(token.replace(",", "")))
        except ValueError:
            continue
    return values


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero(), {"error": "evidence root is not an object"}

    identities = evidence.get("identities", {})
    agents = (
        [
            row
            for row in identities.values()
            if isinstance(row, dict) and row.get("role") == "orchestrator"
        ]
        if isinstance(identities, dict)
        else []
    )
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    question_id = evidence.get("task_event_id")
    trial = evidence.get("trial", {})
    question_channel = trial.get("channel_id") if isinstance(trial, dict) else None

    messages = [row for row in evidence.get("messages", []) if isinstance(row, dict)]
    replies = [
        row
        for row in messages
        if agent_pubkey
        and row.get("pubkey") == agent_pubkey
        and row.get("channel_id") == question_channel
        and row.get("reply_to_event_id") == question_id
    ]
    answer = replies[-1] if replies else None
    content = str(answer.get("content", "")) if answer else ""
    values = _numbers(content)
    mentions_expected = any(value == EXPECTED_CUSTOMERS for value in values)
    mentions_distractor = any(value in DISTRACTOR_NUMBERS for value in values)
    noise_numbers = [
        value
        for value in values
        if value != EXPECTED_CUSTOMERS and value not in ALLOWED_CONTEXT_NUMBERS
    ]
    answer_correct = float(mentions_expected and not noise_numbers)
    threaded_reply = float(answer is not None)
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "memory-retrieval"
        and evidence.get("truncated") is False
        and len(agents) == 1
        and isinstance(question_id, str)
        and isinstance(question_channel, str)
    )

    structural_score = float(threaded_reply == 1.0 and evidence_complete == 1.0)
    metrics = {
        "reward": answer_correct * structural_score,
        "answer_correct": answer_correct,
        "threaded_reply": threaded_reply,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "question_event_id": question_id,
        "question_channel_id": question_channel,
        "answer_message_id": answer.get("id") if answer else None,
        "answer_content": content,
        "parsed_numbers": values,
        "expected_customers": EXPECTED_CUSTOMERS,
        "mentions_expected": mentions_expected,
        "mentions_distractor": mentions_distractor,
        "noise_numbers": noise_numbers,
        "allowed_context_numbers": sorted(ALLOWED_CONTEXT_NUMBERS),
        "distractor_numbers": sorted(DISTRACTOR_NUMBERS),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--reward", type=Path, required=True)
    parser.add_argument("--details", type=Path, required=True)
    args = parser.parse_args()
    try:
        metrics, details = score_evidence(
            json.loads(args.evidence.read_text(encoding="utf-8"))
        )
    except (OSError, json.JSONDecodeError) as error:
        metrics, details = _zero(), {"error": str(error)}
    args.reward.write_text(json.dumps(metrics, sort_keys=True) + "\n", encoding="utf-8")
    args.details.write_text(
        json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
