#!/usr/bin/env python3
"""Verifier for reading a user-named path outside the Buzz workspace."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

CHECK_ID = re.compile(r"^CHECK_ID:\s*(\S+)\s*$", re.MULTILINE)
ACTION = re.compile(r"^ACTION:\s*(\S.*\S|\S)\s*$", re.MULTILINE)


def _normalize(text: str) -> str:
    """Fold case, collapse whitespace, drop trailing punctuation.

    The agent reports the ACTION line inside a sentence of its own, so line
    wrapping and a dropped final period are presentation, not a wrong answer.
    """
    return re.sub(r"\s+", " ", text).strip().strip(".!").casefold()


def _zero_metrics() -> dict[str, float]:
    return {
        "reward": 0.0,
        "named_path_read": 0.0,
        "action_reported": 0.0,
        "same_channel": 0.0,
        "expected_author": 0.0,
        "evidence_complete": 0.0,
    }


def load_expectations(skill_file: Path) -> tuple[str, str]:
    content = skill_file.read_text(encoding="utf-8")
    check_id = CHECK_ID.search(content)
    action = ACTION.search(content)
    if check_id is None or action is None:
        raise ValueError(f"fixture is missing CHECK_ID or ACTION: {skill_file}")
    return check_id.group(1), action.group(1)


def score_evidence(
    evidence: object, *, expected_check_id: str, expected_action: str
) -> tuple[dict[str, float], dict[str, Any]]:
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
    content = str(final.get("content", "")) if final is not None else ""

    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "read-named-path-outside-workspace"
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
        and ["h", channel_id] in final.get("tags", [])
    )
    # CHECK_ID is generated at image build time, so quoting it is proof the
    # file was read — which is the whole question this task asks. Refusal
    # phrasing is deliberately not scored: a real refusal cannot produce this
    # marker or the ACTION line, so these two checks already catch it.
    named_path_read = float(expected_check_id in content)
    action_reported = float(_normalize(expected_action) in _normalize(content))
    reward = float(
        all(
            metric == 1.0
            for metric in (
                evidence_complete,
                expected_author,
                same_channel,
                named_path_read,
                action_reported,
            )
        )
    )
    metrics = {
        "reward": reward,
        "named_path_read": named_path_read,
        "action_reported": action_reported,
        "same_channel": same_channel,
        "expected_author": expected_author,
        "evidence_complete": evidence_complete,
    }
    details = {
        "task_event_id": task_event_id,
        "selected_message_id": final.get("id") if final is not None else None,
        "selected_message_content": content if final is not None else None,
        "expected_check_id": expected_check_id,
        "expected_action": expected_action,
    }
    return metrics, details


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--skill-file", type=Path, required=True)
    parser.add_argument("--reward", type=Path, required=True)
    parser.add_argument("--details", type=Path, required=True)
    args = parser.parse_args()

    try:
        evidence = json.loads(args.evidence.read_text(encoding="utf-8"))
        expected_check_id, expected_action = load_expectations(args.skill_file)
        metrics, details = score_evidence(
            evidence,
            expected_check_id=expected_check_id,
            expected_action=expected_action,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        metrics, details = _zero_metrics(), {"error": str(error)}

    args.reward.write_text(json.dumps(metrics, sort_keys=True) + "\n", encoding="utf-8")
    args.details.write_text(
        json.dumps(details, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
