#!/usr/bin/env python3
"""Deterministic verifier for aggregation of batched agent reports."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any

REPORTERS = ("Ledger Scout", "Risk Sentinel", "Ops Forecaster")
LABELS = {"ledger-report", "risk-report", "operations-report"}


def _zero() -> dict[str, float]:
    return {
        "reward": 0.0,
        "reports_delivered": 0.0,
        "inputs_complete": 0.0,
        "answer_correct": 0.0,
        "single_human_callback": 0.0,
        "reporters_not_rementioned": 0.0,
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
    user = identities.get("Nora Isabel Grant", {})
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
    reporter_pubkeys = {
        directory[name].get("pubkey") for name in REPORTERS if name in directory
    }
    scripts = [
        row for row in evidence.get("scripted_events", []) if isinstance(row, dict)
    ]
    script_ids = {row.get("event_id") for row in scripts if row.get("label") in LABELS}
    messages = [row for row in evidence.get("messages", []) if isinstance(row, dict)]
    delivered_ids = {row.get("id") for row in messages}
    reports_delivered = float(
        len(script_ids) == 3 and None not in script_ids and script_ids <= delivered_ids
    )
    candidates = [
        row for row in messages if agent_pubkey and row.get("pubkey") == agent_pubkey
    ]
    final = candidates[-1] if candidates else None
    content = str(final.get("content", "")) if final else ""
    mentioned = set(final.get("mentioned_pubkeys", [])) if final else set()
    inputs_complete = float(
        all(
            re.search(pattern, content, re.IGNORECASE)
            for pattern in (
                r"north\D+17",
                r"south\D+23",
                r"reserve\D+11",
                r"multiplier\D+3",
            )
        )
    )
    answer_correct = float(
        bool(
            re.search(
                r"(?:final|result|forecast|=)\D*87(?:\D|$)",
                content,
                re.IGNORECASE,
            )
        )
    )
    human_callbacks = [
        row
        for row in candidates
        if user.get("pubkey") in row.get("mentioned_pubkeys", [])
    ]
    single_human_callback = float(
        len(human_callbacks) == 1 and final in human_callbacks
    )
    reporters_not_rementioned = float(
        len(reporter_pubkeys) == 3
        and all(
            set(row.get("mentioned_pubkeys", [])).isdisjoint(reporter_pubkeys)
            and not any(f"@{name}" in str(row.get("content", "")) for name in REPORTERS)
            for row in candidates
        )
    )
    reply_to_thread = float(
        final is not None and final.get("reply_to_event_id") == root
    )
    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "interleaved-agent-reports"
        and evidence.get("truncated") is False
        and len(agents) == 1
        and len(directory) == 3
        and final is not None
        and len(candidates) == 1
    )
    values = (
        reports_delivered,
        inputs_complete,
        answer_correct,
        single_human_callback,
        reporters_not_rementioned,
        reply_to_thread,
        evidence_complete,
    )
    metrics = {
        "reward": float(all(value == 1.0 for value in values)),
        "reports_delivered": reports_delivered,
        "inputs_complete": inputs_complete,
        "answer_correct": answer_correct,
        "single_human_callback": single_human_callback,
        "reporters_not_rementioned": reporters_not_rementioned,
        "reply_to_thread": reply_to_thread,
        "evidence_complete": evidence_complete,
    }
    return metrics, {
        "scripted_event_ids": sorted(str(value) for value in script_ids),
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
