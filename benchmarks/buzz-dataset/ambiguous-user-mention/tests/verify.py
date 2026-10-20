#!/usr/bin/env python3
"""Deterministic verifier for a duplicate-display-name notification."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

TARGET_ID = "taylor-release-captain"
OTHER_ID = "taylor-observer"


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "target_notified_once": 0.0,
        "other_not_notified": 0.0,
        "message_correct": 0.0,
        "user_callback": 0.0,
        "reply_to_thread": 0.0,
        "evidence_complete": 0.0,
    }


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero(), {"error": "evidence root is not an object"}
    root = evidence.get("task_event_id")
    directory = {
        row.get("identity_id"): row
        for row in evidence.get("directory", [])
        if isinstance(row, dict)
    }
    target = directory.get(TARGET_ID, {})
    other = directory.get(OTHER_ID, {})
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    user = identities.get("Olivia Grace Park", {})
    agents = [
        row
        for row in identities.values()
        if isinstance(row, dict) and row.get("role") == "orchestrator"
    ]
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    messages = [row for row in evidence.get("messages", []) if isinstance(row, dict)]
    authored = [
        row for row in messages if agent_pubkey and row.get("pubkey") == agent_pubkey
    ]
    target_messages = [
        row
        for row in authored
        if target.get("pubkey") in row.get("mentioned_pubkeys", [])
    ]
    other_messages = [
        row
        for row in authored
        if other.get("pubkey") in row.get("mentioned_pubkeys", [])
    ]
    delivery = target_messages[0] if len(target_messages) == 1 else None
    callbacks = [
        row
        for row in authored
        if user.get("pubkey") in row.get("mentioned_pubkeys", [])
        and row not in target_messages
    ]
    callback = callbacks[-1] if callbacks else None
    target_notified_once = float(len(target_messages) == 1)
    other_not_notified = float(len(other_messages) == 0)
    message_correct = float(
        delivery is not None
        and "ORCHID-72 approved" in str(delivery.get("content", ""))
    )
    user_callback = float(
        callback is not None
        and any(
            word in str(callback.get("content", "")).casefold()
            for word in ("sent", "notified", "delivered")
        )
    )
    reply_to_thread = float(
        delivery is not None
        and callback is not None
        and delivery.get("reply_to_event_id") == root
        and callback.get("reply_to_event_id") == root
    )
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "ambiguous-user-mention"
        and evidence.get("truncated") is False
        and len(directory) == 2
        and isinstance(target.get("pubkey"), str)
        and isinstance(other.get("pubkey"), str)
        and target.get("pubkey") != other.get("pubkey")
        and len(agents) == 1
    )
    values = (
        target_notified_once,
        other_not_notified,
        message_correct,
        user_callback,
        reply_to_thread,
        evidence_complete,
    )
    metrics = {
        "reward": float(all(value == 1.0 for value in values)),
        "target_notified_once": target_notified_once,
        "other_not_notified": other_not_notified,
        "message_correct": message_correct,
        "user_callback": user_callback,
        "reply_to_thread": reply_to_thread,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "target_pubkey": target.get("pubkey"),
        "other_pubkey": other.get("pubkey"),
        "delivery_message_id": delivery.get("id") if delivery else None,
        "callback_message_id": callback.get("id") if callback else None,
        "target_notification_count": len(target_messages),
        "other_notification_count": len(other_messages),
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
