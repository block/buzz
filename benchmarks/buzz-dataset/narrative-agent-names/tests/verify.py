#!/usr/bin/env python3
"""Deterministic verifier for non-notifying narrative agent names."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

BOT_NAMES = ("Aurora Audit Bot", "Beacon Deploy Bot")


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "status_correct": 0.0,
        "bots_not_mentioned": 0.0,
        "user_mentioned": 0.0,
        "reply_to_thread": 0.0,
        "evidence_complete": 0.0,
    }


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero(), {"error": "evidence root is not an object"}
    root = evidence.get("task_event_id")
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    user = identities.get("Maya Elise Chen", {})
    agents = [
        row
        for row in identities.values()
        if isinstance(row, dict) and row.get("role") == "orchestrator"
    ]
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    directory = {
        row.get("name"): row
        for row in evidence.get("directory", [])
        if isinstance(row, dict)
    }
    bot_pubkeys = {
        directory[name].get("pubkey") for name in BOT_NAMES if name in directory
    }
    messages = [row for row in evidence.get("messages", []) if isinstance(row, dict)]
    candidates = [
        row for row in messages if agent_pubkey and row.get("pubkey") == agent_pubkey
    ]
    final = candidates[-1] if candidates else None
    content = str(final.get("content", "")) if final else ""
    mentioned = set(final.get("mentioned_pubkeys", [])) if final else set()
    status_correct = float(
        bool(re.search(r"Aurora Audit Bot[^.\n]*completed", content, re.IGNORECASE))
        and bool(re.search(r"Beacon Deploy Bot[^.\n]*idle", content, re.IGNORECASE))
    )
    bots_not_mentioned = float(
        len(bot_pubkeys) == 2
        and all(
            set(row.get("mentioned_pubkeys", [])).isdisjoint(bot_pubkeys)
            and not any(f"@{name}" in str(row.get("content", "")) for name in BOT_NAMES)
            for row in candidates
        )
    )
    user_mentioned = float(user.get("pubkey") in mentioned)
    reply_to_thread = float(
        final is not None and final.get("reply_to_event_id") == root
    )
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "narrative-agent-names"
        and evidence.get("truncated") is False
        and len(agents) == 1
        and len(directory) == 2
        and final is not None
        and len(candidates) == 1
    )
    reward = float(
        all(
            value == 1.0
            for value in (
                status_correct,
                bots_not_mentioned,
                user_mentioned,
                reply_to_thread,
                evidence_complete,
            )
        )
    )
    metrics = {
        "reward": reward,
        "status_correct": status_correct,
        "bots_not_mentioned": bots_not_mentioned,
        "user_mentioned": user_mentioned,
        "reply_to_thread": reply_to_thread,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "selected_message_id": final.get("id") if final else None,
        "content": content,
        "mentioned_pubkeys": sorted(mentioned),
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
