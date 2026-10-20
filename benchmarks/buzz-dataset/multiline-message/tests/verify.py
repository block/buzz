#!/usr/bin/env python3
"""Deterministic verifier for multiline Buzz message delivery."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

EXPECTED = (
    "Release readiness\n\n"
    "- API: ready\n"
    "- Database: ready\n"
    "- Rollback: tested\n\n"
    "Owner: Platform Operations"
)


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "layout_preserved": 0.0,
        "real_newlines": 0.0,
        "reply_to_thread": 0.0,
        "user_mentioned": 0.0,
        "evidence_complete": 0.0,
    }


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero(), {"error": "evidence root is not an object"}
    root = evidence.get("task_event_id")
    trial = evidence.get("trial") if isinstance(evidence.get("trial"), dict) else {}
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    user = identities.get("Eleanor June Brooks", {})
    agents = [
        row
        for row in identities.values()
        if isinstance(row, dict) and row.get("role") == "orchestrator"
    ]
    agent_pubkey = agents[0].get("pubkey") if len(agents) == 1 else None
    messages = [row for row in evidence.get("messages", []) if isinstance(row, dict)]
    candidates = [
        row for row in messages if agent_pubkey and row.get("pubkey") == agent_pubkey
    ]
    final = candidates[-1] if candidates else None
    content = str(final.get("content", "")) if final else ""
    tags = final.get("tags", []) if final else []
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "multiline-message"
        and evidence.get("truncated") is False
        and isinstance(root, str)
        and isinstance(trial.get("channel_id"), str)
        and len(agents) == 1
        and isinstance(user.get("pubkey"), str)
        and final is not None
        and len(candidates) == 1
    )
    layout_preserved = float(EXPECTED in content)
    real_newlines = float("\\n" not in content and content.count("\n") >= 6)
    reply_to_thread = float(
        final is not None and final.get("reply_to_event_id") == root
    )
    user_mentioned = float(
        user.get("pubkey") in (final.get("mentioned_pubkeys", []) if final else [])
    )
    reward = float(
        all(
            value == 1.0
            for value in (
                evidence_complete,
                layout_preserved,
                real_newlines,
                reply_to_thread,
                user_mentioned,
            )
        )
    )
    metrics = {
        "reward": reward,
        "layout_preserved": layout_preserved,
        "real_newlines": real_newlines,
        "reply_to_thread": reply_to_thread,
        "user_mentioned": user_mentioned,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "selected_message_id": final.get("id") if final else None,
        "content": content,
        "tags": tags,
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
