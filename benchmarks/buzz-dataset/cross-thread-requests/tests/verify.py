#!/usr/bin/env python3
"""Deterministic verifier for isolation of two top-level requests."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "alpha_correct": 0.0,
        "beta_correct": 0.0,
        "thread_isolation": 0.0,
        "user_mentioned_twice": 0.0,
        "evidence_complete": 0.0,
    }


def _matches(message: dict[str, Any], label: str, value: int) -> bool:
    content = str(message.get("content", ""))
    return bool(re.search(rf"{label}\D+{value}(?:\D|$)", content, re.IGNORECASE))


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero(), {"error": "evidence root is not an object"}
    alpha_root = evidence.get("task_event_id")
    scripts = [
        row
        for row in evidence.get("scripted_events", [])
        if isinstance(row, dict) and row.get("label") == "beta-request"
    ]
    beta_root = scripts[0].get("event_id") if len(scripts) == 1 else None
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    user = identities.get("Priya Simone Patel", {})
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
    alpha_replies = [
        row
        for row in candidates
        if row.get("reply_to_event_id") == alpha_root and _matches(row, "ALPHA", 346)
    ]
    beta_replies = [
        row
        for row in candidates
        if row.get("reply_to_event_id") == beta_root and _matches(row, "BETA", 41)
    ]
    alpha = alpha_replies[-1] if alpha_replies else None
    beta = beta_replies[-1] if beta_replies else None
    alpha_content = str(alpha.get("content", "")) if alpha else ""
    beta_content = str(beta.get("content", "")) if beta else ""
    alpha_correct = float(alpha is not None)
    beta_correct = float(beta is not None)
    thread_isolation = float(
        alpha is not None
        and beta is not None
        and len(candidates) == 2
        and alpha.get("id") != beta.get("id")
        and "BETA" not in alpha_content.upper()
        and "ALPHA" not in beta_content.upper()
    )
    user_mentioned_twice = float(
        alpha is not None
        and beta is not None
        and user.get("pubkey") in alpha.get("mentioned_pubkeys", [])
        and user.get("pubkey") in beta.get("mentioned_pubkeys", [])
    )
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "cross-thread-requests"
        and evidence.get("truncated") is False
        and isinstance(alpha_root, str)
        and isinstance(beta_root, str)
        and len(agents) == 1
    )
    values = (
        alpha_correct,
        beta_correct,
        thread_isolation,
        user_mentioned_twice,
        evidence_complete,
    )
    metrics = {
        "reward": float(all(value == 1.0 for value in values)),
        "alpha_correct": alpha_correct,
        "beta_correct": beta_correct,
        "thread_isolation": thread_isolation,
        "user_mentioned_twice": user_mentioned_twice,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "alpha_root": alpha_root,
        "beta_root": beta_root,
        "alpha_message_id": alpha.get("id") if alpha else None,
        "beta_message_id": beta.get("id") if beta else None,
        "alpha_content": alpha_content,
        "beta_content": beta_content,
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
