#!/usr/bin/env python3
"""metabolic.v0.2 guardrails — pure deterministic, zero LLM.

Scales multi-agent rooms by bounding admission, not by smarter polling.

Runtime fold (2026-08-07): prefer skill copies —
  codex-buzz-skill-dev/scripts/metabolic_guardrails.py
  ~/.grok/skills/use-buzz/scripts/metabolic_guardrails.py
This mono file remains the design snapshot + unit-test target.
"""
from __future__ import annotations

import hashlib
import json
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Optional


class FailureReason(str, Enum):
    AUTH = "auth"
    TRANSPORT = "transport"
    CURSOR = "cursor"
    SCHEMA = "schema"
    ADMISSION_OVERFLOW = "admission_overflow"
    STALE_NERVE = "stale_nerve"


class Action(str, Enum):
    IGNORE = "ignore"
    DIAGNOSTIC = "diagnostic"
    SUPPRESS = "suppress"
    NOTIFY = "notify"
    ADMIT = "AdmitCortex"
    OVERFLOW = "overflow"


@dataclass
class WakeBudget:
    max_events_per_turn: int = 3
    max_context_bytes: int = 2048
    per_task_cooldown_secs: int = 30


@dataclass
class AdapterCaps:
    transport: str = "poll"  # push | poll
    hitl: bool = True
    fetch_by_id: bool = True
    max_context_bytes: int = 2048


@dataclass
class GuardState:
    admitted_ids: set[str] = field(default_factory=set)
    actions_by_correlation: set[str] = field(default_factory=set)
    last_admit_by_task: dict[str, float] = field(default_factory=dict)
    turn_events: int = 0
    turn_bytes: int = 0


REQUIRED_WAKE_FIELDS = ("schema", "event_id", "channel_id", "t", "urgency", "seat_id", "pubkey", "summary")


def validate_event(payload: dict[str, Any]) -> Optional[FailureReason]:
    """Return failure reason if invalid; None if OK."""
    if not payload.get("event_id") or not str(payload.get("summary") or "").strip():
        return FailureReason.SCHEMA
    schema = payload.get("schema") or ""
    if schema and not str(schema).startswith("team.v0") and schema != "metabolic.wake.v0":
        # unknown major schema
        if not str(schema).startswith("team.v") and not str(schema).startswith("metabolic.wake"):
            return FailureReason.SCHEMA
    if not payload.get("channel_id") and not payload.get("t"):
        return FailureReason.SCHEMA
    return None


def action_once_key(correlation_id: Optional[str], lease_id: Optional[str], action: str) -> str:
    raw = f"{correlation_id or ''}|{lease_id or ''}|{action}"
    return hashlib.sha256(raw.encode()).hexdigest()[:24]


def check_idempotency(state: GuardState, correlation_id: Optional[str], lease_id: Optional[str], action: str = "admit") -> bool:
    """True if this action may proceed (not yet taken)."""
    if not correlation_id and not lease_id:
        return True
    key = action_once_key(correlation_id, lease_id, action)
    if key in state.actions_by_correlation:
        return False
    return True


def mark_action(state: GuardState, correlation_id: Optional[str], lease_id: Optional[str], action: str = "admit") -> None:
    if not correlation_id and not lease_id:
        return
    state.actions_by_correlation.add(action_once_key(correlation_id, lease_id, action))


def check_cooldown(state: GuardState, task_id: Optional[str], budget: WakeBudget, now: Optional[float] = None) -> bool:
    """True if admit allowed (cooldown elapsed)."""
    if not task_id:
        return True
    now = now if now is not None else time.time()
    last = state.last_admit_by_task.get(task_id)
    if last is None:
        return True
    return (now - last) >= budget.per_task_cooldown_secs


def admit_wake(
    state: GuardState,
    wake: dict[str, Any],
    budget: WakeBudget,
    caps: AdapterCaps,
    now: Optional[float] = None,
) -> dict[str, Any]:
    """Apply v0.2 budgets + idempotency. Returns decision dict."""
    now = now if now is not None else time.time()
    eid = wake.get("event_id") or ""

    # schema / required
    missing = [f for f in REQUIRED_WAKE_FIELDS if not wake.get(f)]
    if missing:
        return {
            "action": Action.DIAGNOSTIC.value,
            "reason": FailureReason.SCHEMA.value,
            "detail": f"missing {missing}",
        }

    # unknown schema major
    schema = str(wake.get("schema") or "")
    if schema not in ("metabolic.wake.v0", "team.v0") and not schema.startswith("team.v0"):
        if schema and not schema.startswith("metabolic.wake"):
            return {
                "action": Action.DIAGNOSTIC.value,
                "reason": FailureReason.SCHEMA.value,
                "detail": f"unknown schema {schema}",
            }

    # replay
    if eid in state.admitted_ids:
        return {"action": Action.SUPPRESS.value, "reason": "replay", "event_id": eid}

    # idempotent action
    corr = wake.get("correlation_id")
    lease = wake.get("lease_id") or wake.get("owner_epoch")
    if not check_idempotency(state, corr, lease, "admit"):
        return {
            "action": Action.SUPPRESS.value,
            "reason": "idempotent",
            "correlation_id": corr,
            "lease_id": lease,
        }

    # cooldown
    task_id = wake.get("task_id")
    if not check_cooldown(state, task_id, budget, now=now):
        return {
            "action": Action.SUPPRESS.value,
            "reason": "cooldown",
            "task_id": task_id,
        }

    # context bytes (summary-default; content optional)
    summary = str(wake.get("summary") or "")
    body = summary
    if wake.get("content") is not None and caps.fetch_by_id is False:
        body = summary + json.dumps(wake.get("content"), separators=(",", ":"))
    max_bytes = min(budget.max_context_bytes, caps.max_context_bytes)
    body_bytes = len(body.encode("utf-8"))
    if body_bytes > max_bytes:
        return {
            "action": Action.DIAGNOSTIC.value,
            "reason": FailureReason.ADMISSION_OVERFLOW.value,
            "detail": f"context {body_bytes}>{max_bytes}",
        }

    # turn budgets
    if state.turn_events >= budget.max_events_per_turn:
        return {
            "action": Action.OVERFLOW.value,
            "reason": FailureReason.ADMISSION_OVERFLOW.value,
            "detail": f"max_events_per_turn={budget.max_events_per_turn}",
            "status": "overflow",
        }
    if state.turn_bytes + body_bytes > max_bytes * budget.max_events_per_turn:
        return {
            "action": Action.OVERFLOW.value,
            "reason": FailureReason.ADMISSION_OVERFLOW.value,
            "detail": "turn_byte_budget",
            "status": "overflow",
        }

    # admit
    state.admitted_ids.add(eid)
    mark_action(state, corr, lease, "admit")
    if task_id:
        state.last_admit_by_task[task_id] = now
    state.turn_events += 1
    state.turn_bytes += body_bytes

    # degrade payload for cortex
    cortex = {
        "schema": "metabolic.wake.v0",
        "event_id": eid,
        "t": wake.get("t"),
        "urgency": wake.get("urgency"),
        "task_id": task_id,
        "correlation_id": corr,
        "summary": summary[:500],
    }
    return {"action": Action.ADMIT.value, "wake": cortex, "status": "ok"}


def new_turn(state: GuardState) -> None:
    state.turn_events = 0
    state.turn_bytes = 0


def monitor_failure(reason: FailureReason, detail: str = "") -> dict[str, Any]:
    return {
        "t": "team.v0.monitor.failure",
        "reason": reason.value,
        "detail": detail,
        "summary": f"monitor.failure:{reason.value}",
    }
