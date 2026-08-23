"""Stable, verifier-facing evidence derived from a Buzz channel transcript."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from typing import Any

from .provisioning import TrialHandle

EVIDENCE_SCHEMA_VERSION = 1


def _tags(message: Mapping[str, Any]) -> list[list[str]]:
    """Return only well-formed string tags from a relay message."""
    raw = message.get("tags")
    if not isinstance(raw, list):
        return []
    return [
        list(tag)
        for tag in raw
        if isinstance(tag, list)
        and tag
        and all(isinstance(value, str) for value in tag)
    ]


def _tag_value(
    tags: Iterable[list[str]], name: str, marker: str | None = None
) -> str | None:
    for tag in tags:
        if len(tag) < 2 or tag[0] != name:
            continue
        if marker is not None and (len(tag) < 4 or tag[3] != marker):
            continue
        return tag[1]
    return None


def _normalize_message(
    message: Mapping[str, Any], identities: Mapping[str, Mapping[str, str]]
) -> dict[str, Any]:
    tags = _tags(message)
    pubkey = message.get("pubkey") if isinstance(message.get("pubkey"), str) else ""
    identity = identities.get(pubkey, {})
    return {
        "id": message.get("id") if isinstance(message.get("id"), str) else "",
        "kind": message.get("kind") if isinstance(message.get("kind"), int) else None,
        "created_at": (
            message.get("created_at")
            if isinstance(message.get("created_at"), int)
            else None
        ),
        "pubkey": pubkey,
        "author": identity.get("name", "unknown"),
        "author_role": identity.get("role", "unknown"),
        "content": (
            message.get("content") if isinstance(message.get("content"), str) else ""
        ),
        # Preserve the signed protocol evidence. Derived fields below make the
        # common checks convenient without replacing the source-of-truth tags.
        "tags": tags,
        "channel_id": _tag_value(tags, "h"),
        "reply_to_event_id": _tag_value(tags, "e", "reply"),
        "mentioned_pubkeys": [
            tag[1] for tag in tags if len(tag) >= 2 and tag[0] == "p"
        ],
    }


def build_buzz_evidence(
    *,
    trial: TrialHandle,
    messages: object,
    task_event_id: str | None,
    completion_message_id: str | None,
    transcript_limit: int,
    observed_channels: object = None,
    scripted_events: object = None,
) -> dict[str, Any]:
    """Normalize relay messages into a versioned contract for task verifiers.

    Private keys and auth tags are intentionally absent. The exported identities
    contain only public names, roles, and pubkeys already visible on the relay.
    """
    raw_messages = messages if isinstance(messages, list) else []
    identity_rows = (
        (trial.user.agent_id, "user", trial.user.nostr_pubkey),
        *(
            (credential.agent_id, credential.role, credential.nostr_pubkey)
            for credential in trial.credentials
        ),
    )
    identities_by_pubkey = {
        pubkey: {"name": name, "role": role} for name, role, pubkey in identity_rows
    }
    identities_by_pubkey.update(
        {
            identity.pubkey: {"name": identity.name, "role": identity.role}
            for identity in trial.directory
        }
    )
    identities = {
        name: {"role": role, "pubkey": pubkey} for name, role, pubkey in identity_rows
    }
    normalized = [
        _normalize_message(message, identities_by_pubkey)
        for message in raw_messages
        if isinstance(message, dict)
    ]
    normalized.sort(
        key=lambda message: (
            message["created_at"] is None,
            message["created_at"] or 0,
        )
    )
    return {
        "schema_version": EVIDENCE_SCHEMA_VERSION,
        "trial": {
            "run_id": trial.run_id,
            "trial_id": trial.trial_id,
            "channel_id": trial.channel_id,
        },
        "task_event_id": task_event_id,
        "completion_message_id": completion_message_id,
        "identities": identities,
        "directory": [
            {
                "identity_id": identity.identity_id or identity.name,
                "name": identity.name,
                "role": identity.role,
                "pubkey": identity.pubkey,
                "about": identity.about,
            }
            for identity in trial.directory
        ],
        "scripted_events": scripted_events if isinstance(scripted_events, list) else [],
        "task_name": trial.task_name,
        "observed_channels": (
            observed_channels if isinstance(observed_channels, list) else []
        ),
        "message_count": len(normalized),
        "truncated": len(raw_messages) >= transcript_limit,
        "messages": normalized,
    }
