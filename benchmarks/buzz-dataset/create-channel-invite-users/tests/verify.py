#!/usr/bin/env python3
"""Deterministic verifier for exact Buzz channel creation and membership."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

CHANNEL_NAME = "fix-pr-1234"
TARGET_USERS = {"benchmark-user-07", "benchmark-user-19", "benchmark-user-42"}
TARGET_BOTS = {"benchmark-bot-03", "benchmark-bot-08"}


def _zero_metrics() -> dict[str, float]:
    return {
        "reward": 0.0,
        "channel_created": 0.0,
        "channel_shape": 0.0,
        "temporary_channel": 0.0,
        "exact_membership": 0.0,
        "expected_roles": 0.0,
        "evidence_complete": 0.0,
    }


def score_evidence(evidence: object) -> tuple[dict[str, float], dict[str, Any]]:
    if not isinstance(evidence, dict):
        return _zero_metrics(), {"error": "evidence root is not an object"}

    directory_rows = [
        row for row in evidence.get("directory", []) if isinstance(row, dict)
    ]
    directory = {
        row.get("name"): row
        for row in directory_rows
        if isinstance(row.get("name"), str)
    }
    channels = [
        channel
        for channel in evidence.get("observed_channels", [])
        if isinstance(channel, dict) and channel.get("name") == CHANNEL_NAME
    ]
    channel = channels[0] if len(channels) == 1 else None
    identities = (
        evidence.get("identities")
        if isinstance(evidence.get("identities"), dict)
        else {}
    )
    orchestrators = [
        row
        for row in identities.values()
        if isinstance(row, dict) and row.get("role") == "orchestrator"
    ]
    owner_pubkey = orchestrators[0].get("pubkey") if len(orchestrators) == 1 else None

    expected_names = TARGET_USERS | TARGET_BOTS
    expected_targets = {
        directory[name]["pubkey"]: "bot" if name in TARGET_BOTS else "member"
        for name in expected_names
        if name in directory and isinstance(directory[name].get("pubkey"), str)
    }
    expected_members = (
        {owner_pubkey: "owner", **expected_targets}
        if isinstance(owner_pubkey, str)
        else expected_targets
    )
    member_rows = (
        [row for row in channel.get("members", []) if isinstance(row, dict)]
        if channel is not None
        else []
    )
    actual_members = {
        row.get("pubkey"): row.get("role")
        for row in member_rows
        if isinstance(row.get("pubkey"), str)
    }

    evidence_complete = float(
        evidence.get("schema_version") == 1
        and evidence.get("task_name") == "create-channel-invite-users"
        and len(directory_rows) == 60
        and len(directory) == 60
        and sum(row.get("role") == "user" for row in directory_rows) == 50
        and sum(row.get("role") == "bot" for row in directory_rows) == 10
        and len(expected_targets) == 5
        and len(orchestrators) == 1
    )
    channel_created = float(channel is not None)
    channel_shape = float(
        channel is not None
        and channel.get("channel_type") == "stream"
        and channel.get("visibility") == "private"
        and channel.get("archived") is False
    )
    temporary_channel = float(
        channel is not None and channel.get("ttl_seconds") == 3600
    )
    exact_membership = float(
        len(member_rows) == len(actual_members)
        and set(actual_members) == set(expected_members)
    )
    expected_roles = float(actual_members == expected_members)
    reward = float(
        all(
            metric == 1.0
            for metric in (
                evidence_complete,
                channel_created,
                channel_shape,
                temporary_channel,
                exact_membership,
                expected_roles,
            )
        )
    )
    metrics = {
        "reward": reward,
        "channel_created": channel_created,
        "channel_shape": channel_shape,
        "temporary_channel": temporary_channel,
        "exact_membership": exact_membership,
        "expected_roles": expected_roles,
        "evidence_complete": evidence_complete,
    }
    details = {
        "matching_channel_count": len(channels),
        "channel_id": channel.get("channel_id") if channel is not None else None,
        "expected_members": expected_members,
        "actual_members": actual_members,
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
