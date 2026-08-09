#!/usr/bin/env python3
"""Third-runtime adapter stub — W1.1 contract, zero LLM.

Proves a non-Grok / non-Codex process can arm, admit, and report status using
the same metabolic.wake.v0 payload + v0.2 guardrails as the skill L2 path.

Default runtime id: ``local-llm`` (generic process). ``antigravity`` is an
alias that shares the same process contract until product hooks exist.

Not a product turn injector. Stdout lines are the adapter surface.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Optional, TextIO

# Parent docs/metabolic for guardrails_v02
_METABOLIC = Path(__file__).resolve().parent.parent
if str(_METABOLIC) not in sys.path:
    sys.path.insert(0, str(_METABOLIC))

from guardrails_v02 import (  # noqa: E402
    Action,
    AdapterCaps,
    FailureReason,
    GuardState,
    WakeBudget,
    admit_wake,
    monitor_failure,
    new_turn,
)

SCHEMA = "metabolic.wake.v0"
DEFAULT_RUNTIME = "local-llm"
# Product aliases → same stub process until real drivers land
RUNTIME_ALIASES = {
    "antigravity": "local-llm",
    "local": "local-llm",
    "llm": "local-llm",
}

def state_root() -> Path:
    override = os.environ.get("BUZZ_ADAPTER_STATE_DIR")
    if override:
        return Path(override)
    return Path.home() / ".buzz-dev" / "adapters"


def resolve_runtime(runtime: str) -> str:
    r = (runtime or DEFAULT_RUNTIME).strip().lower()
    return RUNTIME_ALIASES.get(r, r)


def seat_dir(runtime: str, seat: str) -> Path:
    return state_root() / resolve_runtime(runtime) / seat


@dataclass
class AdapterSession:
    runtime: str
    seat: str
    room: str
    room_name: str = ""
    transport: str = "poll"  # push | poll
    # Product sink after AdmitCortex: none | notify | local-llm | antigravity
    driver: str = "local-llm"
    armed_at: float = 0.0
    last_health_at: float = 0.0
    # Transport resume watermark (unix secs). CLI --since uses max(0, since-1).
    since: int = 0
    self_pubkey: str = ""
    pending: list[dict[str, Any]] = field(default_factory=list)
    transport_seen: list[str] = field(default_factory=list)
    admitted_log: list[dict[str, Any]] = field(default_factory=list)
    driver_log: list[dict[str, Any]] = field(default_factory=list)
    last_failure: Optional[dict[str, Any]] = None


def session_path(runtime: str, seat: str) -> Path:
    return seat_dir(runtime, seat) / "session.json"


def guard_path(runtime: str, seat: str) -> Path:
    return seat_dir(runtime, seat) / "guard-state.json"


def load_session(runtime: str, seat: str) -> Optional[AdapterSession]:
    path = session_path(runtime, seat)
    if not path.exists():
        return None
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None
    return AdapterSession(
        runtime=data.get("runtime") or resolve_runtime(runtime),
        seat=data.get("seat") or seat,
        room=data.get("room") or "",
        room_name=data.get("room_name") or "",
        transport=data.get("transport") or "poll",
        driver=data.get("driver") or "local-llm",
        armed_at=float(data.get("armed_at") or 0),
        last_health_at=float(data.get("last_health_at") or 0),
        since=int(data.get("since") or 0),
        self_pubkey=data.get("self_pubkey") or "",
        pending=list(data.get("pending") or []),
        transport_seen=list(data.get("transport_seen") or []),
        admitted_log=list(data.get("admitted_log") or []),
        driver_log=list(data.get("driver_log") or []),
        last_failure=data.get("last_failure"),
    )


def save_session(session: AdapterSession) -> None:
    path = session_path(session.runtime, session.seat)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "runtime": session.runtime,
        "seat": session.seat,
        "room": session.room,
        "room_name": session.room_name,
        "transport": session.transport,
        "driver": session.driver or "local-llm",
        "armed_at": session.armed_at,
        "last_health_at": session.last_health_at,
        "since": int(session.since or 0),
        "self_pubkey": session.self_pubkey or "",
        "pending": session.pending[-100:],
        "transport_seen": session.transport_seen[-200:],
        "admitted_log": session.admitted_log[-50:],
        "driver_log": session.driver_log[-50:],
        "last_failure": session.last_failure,
    }
    path.write_text(json.dumps(payload, indent=2) + "\n")


def load_guard(runtime: str, seat: str) -> GuardState:
    path = guard_path(runtime, seat)
    if not path.exists():
        return GuardState()
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return GuardState()
    return GuardState(
        admitted_ids=set(data.get("admitted_ids") or []),
        actions_by_correlation=set(data.get("actions_by_correlation") or []),
        last_admit_by_task={
            str(k): float(v) for k, v in (data.get("last_admit_by_task") or {}).items()
        },
        turn_events=int(data.get("turn_events") or 0),
        turn_bytes=int(data.get("turn_bytes") or 0),
    )


def save_guard(runtime: str, seat: str, state: GuardState) -> None:
    path = guard_path(runtime, seat)
    path.parent.mkdir(parents=True, exist_ok=True)
    # Persist turn counters so a multi-wake inject in one process (or
    # sequential CLI injects in one turn) still hits max_events_per_turn.
    path.write_text(
        json.dumps(
            {
                "admitted_ids": list(state.admitted_ids)[-500:],
                "actions_by_correlation": list(state.actions_by_correlation)[-500:],
                "last_admit_by_task": dict(state.last_admit_by_task),
                "turn_events": state.turn_events,
                "turn_bytes": state.turn_bytes,
            },
            indent=2,
        )
        + "\n"
    )


def budget_from_env() -> WakeBudget:
    def _int(name: str, default: int) -> int:
        raw = os.environ.get(name)
        if raw is None or raw == "":
            return default
        try:
            return int(raw)
        except ValueError:
            return default

    return WakeBudget(
        max_events_per_turn=_int("BUZZ_ADMIT_MAX_EVENTS", 3),
        max_context_bytes=_int("BUZZ_ADMIT_MAX_CONTEXT_BYTES", 2048),
        per_task_cooldown_secs=_int("BUZZ_ADMIT_COOLDOWN_SECS", 30),
    )


def caps_from_session(session: AdapterSession) -> AdapterCaps:
    max_bytes = 2048
    raw = os.environ.get("BUZZ_ADMIT_MAX_CONTEXT_BYTES")
    if raw:
        try:
            max_bytes = int(raw)
        except ValueError:
            pass
    return AdapterCaps(
        transport=session.transport if session.transport in ("push", "poll") else "poll",
        hitl=True,
        fetch_by_id=True,
        max_context_bytes=max_bytes,
    )


def normalize_wake(raw: dict[str, Any], session: AdapterSession) -> dict[str, Any]:
    """Map free-form / skill row / W1.1 payload → admit_wake shape."""
    content = raw.get("content")
    summary = raw.get("summary") or raw.get("preview") or ""
    if not summary and isinstance(content, str):
        summary = " ".join(content.split())[:200]
    if not summary and content is not None:
        summary = json.dumps(content, separators=(",", ":"))[:200]
    if not summary:
        summary = "(empty)"
    wake = {
        "schema": raw.get("schema") or SCHEMA,
        "event_id": raw.get("event_id") or raw.get("id") or "",
        "channel_id": raw.get("channel_id") or session.room or "",
        "t": raw.get("t") or "team.v0.room.message",
        "urgency": raw.get("urgency") or "P2",
        "seat_id": raw.get("seat_id") or raw.get("seat") or session.seat,
        "pubkey": raw.get("pubkey") or raw.get("from") or "",
        "summary": summary,
        "received_at": int(raw.get("received_at") or time.time()),
    }
    for key in (
        "channel_name",
        "runtime",
        "lane_id",
        "task_id",
        "correlation_id",
        "target_seat",
        "target_pubkey",
        "lease_id",
        "content",
    ):
        if raw.get(key) is not None:
            wake[key] = raw[key]
    if "runtime" not in wake:
        wake["runtime"] = session.runtime
    return wake


def cortex_short(decision_wake: dict[str, Any], source: dict[str, Any]) -> dict[str, Any]:
    """W1.1 AdmitCortex default: summary + ids only."""
    return {
        "schema": SCHEMA,
        "event_id": decision_wake.get("event_id") or source.get("event_id"),
        "t": decision_wake.get("t") or source.get("t"),
        "urgency": decision_wake.get("urgency") or source.get("urgency"),
        "task_id": decision_wake.get("task_id") or source.get("task_id"),
        "correlation_id": decision_wake.get("correlation_id") or source.get("correlation_id"),
        "summary": (decision_wake.get("summary") or source.get("summary") or "")[:500],
    }


# --- W1.1 surface -----------------------------------------------------------


def arm(
    runtime: str,
    seat: str,
    room: str,
    room_name: str = "",
    transport: str = "poll",
    self_pubkey: str = "",
    since: Optional[int] = None,
    driver: str = "",
) -> AdapterSession:
    from drivers import resolve_driver_name  # local package next to stub_runtime

    runtime = resolve_runtime(runtime)
    now = time.time()
    pk = self_pubkey or os.environ.get("BUZZ_PUBLIC_KEY") or ""
    # Default seed: now — do not re-admit full channel history on first watch.
    seed = int(since) if since is not None else int(now)
    drv = resolve_driver_name(driver or None)
    session = AdapterSession(
        runtime=runtime,
        seat=seat,
        room=room,
        room_name=room_name,
        transport=transport if transport in ("push", "poll") else "poll",
        driver=drv,
        armed_at=now,
        last_health_at=now,
        since=seed,
        self_pubkey=pk,
    )
    # preserve cursors if re-arm same seat+room
    prev = load_session(runtime, seat)
    if prev and prev.room == room:
        session.pending = prev.pending
        session.transport_seen = prev.transport_seen
        session.admitted_log = prev.admitted_log
        session.driver_log = prev.driver_log
        if since is None and prev.since:
            session.since = prev.since
        if prev.self_pubkey and not pk:
            session.self_pubkey = prev.self_pubkey
        if not driver and prev.driver:
            session.driver = prev.driver
    save_session(session)
    # fresh turn counters on arm
    guard = load_guard(runtime, seat)
    new_turn(guard)
    save_guard(runtime, seat, guard)
    print(
        f"BUZZ_ADAPTER armed runtime={runtime} seat={seat} room={room} "
        f"transport={session.transport} driver={session.driver} since={session.since}",
        flush=True,
    )
    return session


def disarm(runtime: str, seat: str) -> None:
    runtime = resolve_runtime(runtime)
    path = session_path(runtime, seat)
    if path.exists():
        path.unlink()
    print(f"BUZZ_ADAPTER disarmed runtime={runtime} seat={seat}", flush=True)


def status(runtime: str, seat: str) -> dict[str, Any]:
    runtime = resolve_runtime(runtime)
    session = load_session(runtime, seat)
    if not session:
        line = f"BUZZ_MONITOR runtime={runtime} seat={seat} nerve=stopped pending=0"
        print(line, flush=True)
        return {"nerve": "stopped", "pending": 0}
    pending = len(session.pending)
    line = (
        f"BUZZ_MONITOR runtime={session.runtime} seat={session.seat} "
        f"room={session.room_name or session.room} nerve=attached "
        f"pending={pending} transport={session.transport} driver={session.driver}"
    )
    print(line, flush=True)
    return {
        "nerve": "attached",
        "pending": pending,
        "runtime": session.runtime,
        "seat": session.seat,
        "room": session.room,
        "transport": session.transport,
        "driver": session.driver,
    }


def health(runtime: str, seat: str, stale_after_secs: float = 120.0) -> dict[str, Any]:
    runtime = resolve_runtime(runtime)
    session = load_session(runtime, seat)
    if not session:
        print(
            f"BUZZ_ADAPTER health=stale runtime={runtime} seat={seat} reason=not_armed",
            flush=True,
        )
        return monitor_failure(FailureReason.STALE_NERVE, "not_armed")
    age = time.time() - float(session.last_health_at or session.armed_at or 0)
    if age > stale_after_secs:
        session.last_failure = monitor_failure(
            FailureReason.STALE_NERVE, f"age_secs={int(age)}"
        )
        save_session(session)
        print(
            f"BUZZ_ADAPTER health=stale runtime={runtime} seat={seat} age_secs={int(age)}",
            flush=True,
        )
        return session.last_failure
    session.last_health_at = time.time()
    save_session(session)
    print(
        f"BUZZ_ADAPTER health={session.transport} runtime={runtime} seat={seat}",
        flush=True,
    )
    return {"health": session.transport, "age_secs": age}


def on_wake(
    session: AdapterSession,
    raw: dict[str, Any],
    *,
    budget: Optional[WakeBudget] = None,
    start_turn: bool = False,
    now: Optional[float] = None,
    guard: Optional[GuardState] = None,
) -> dict[str, Any]:
    """Apply dual-cursor transport + v0.2 admission. Returns decision dict.

    Pass a shared ``guard`` across a batch so turn budgets apply (do not
    reload a zeroed turn counter between wakes).
    """
    budget = budget or budget_from_env()
    now = now if now is not None else time.time()
    wake = normalize_wake(raw, session)
    eid = wake.get("event_id") or ""

    # transport cursor (≠ admission)
    if eid and eid in set(session.transport_seen):
        decision = {
            "action": Action.IGNORE.value,
            "reason": "transport_replay",
            "event_id": eid,
        }
        print(
            f"BUZZ_ADAPTER on_wake action=ignore id={eid[:12]} reason=transport_replay",
            flush=True,
        )
        return decision
    if eid:
        session.transport_seen.append(eid)
        session.transport_seen = session.transport_seen[-200:]

    own_guard = guard is None
    if guard is None:
        guard = load_guard(session.runtime, session.seat)
    if start_turn:
        new_turn(guard)

    caps = caps_from_session(session)
    decision = admit_wake(guard, wake, budget, caps, now=now)
    action = decision.get("action")

    if action == Action.ADMIT.value:
        short = cortex_short(decision.get("wake") or {}, wake)
        session.admitted_log.append({"at": now, "wake": short})
        session.admitted_log = session.admitted_log[-50:]
        # remove from pending if present
        session.pending = [p for p in session.pending if (p.get("event_id") or p.get("id")) != eid]
        print(
            f"BUZZ_ADAPTER on_wake action=AdmitCortex id={eid[:12]} "
            f"urgency={short.get('urgency')} summary={short.get('summary', '')[:80]}",
            flush=True,
        )
        print(
            f"BUZZ_ADAPTER cortex {json.dumps(short, ensure_ascii=False)}",
            flush=True,
        )
        # Product driver hook (sink only — never owns transport/admit)
        driver_result = None
        try:
            from drivers import invoke_driver, print_driver_result

            driver_result = invoke_driver(session.driver, short, session)
            if driver_result is not None:
                print_driver_result(driver_result)
                session.driver_log.append(
                    {"at": now, "event_id": eid, **driver_result.as_dict()}
                )
                session.driver_log = session.driver_log[-50:]
        except Exception as exc:  # driver faults must not kill L0
            print(
                f"BUZZ_DRIVER status=error driver={session.driver} "
                f"detail={str(exc)[:120]}",
                flush=True,
            )
            session.last_failure = {
                "t": "team.v0.monitor.failure",
                "reason": "schema",
                "detail": f"driver:{str(exc)[:80]}",
                "summary": "monitor.failure:driver",
            }
        decision = {
            **decision,
            "cortex": short,
            "action": Action.ADMIT.value,
            "driver": driver_result.as_dict() if driver_result else None,
        }
    elif action == Action.OVERFLOW.value:
        # leave as pending for a later turn
        if eid and not any((p.get("event_id") or p.get("id")) == eid for p in session.pending):
            session.pending.append(wake)
        print(
            f"BUZZ_ADMIT overflow reason=admission_overflow "
            f"max_events_per_turn={budget.max_events_per_turn}",
            flush=True,
        )
        print(
            f"BUZZ_ADAPTER on_wake action=overflow id={eid[:12]} "
            f"reason={decision.get('reason')}",
            flush=True,
        )
    elif action == Action.SUPPRESS.value:
        reason = decision.get("reason")
        if reason == "cooldown" and eid:
            if not any((p.get("event_id") or p.get("id")) == eid for p in session.pending):
                session.pending.append(wake)
        print(
            f"BUZZ_ADAPTER on_wake action=suppress id={eid[:12]} reason={reason}",
            flush=True,
        )
    elif action == Action.DIAGNOSTIC.value:
        session.last_failure = {
            "t": "team.v0.monitor.failure",
            "reason": decision.get("reason") or FailureReason.SCHEMA.value,
            "detail": decision.get("detail") or "",
            "summary": f"monitor.failure:{decision.get('reason')}",
        }
        print(
            f"BUZZ_ADAPTER on_wake action=diagnostic id={eid[:12]} "
            f"reason={decision.get('reason')} detail={decision.get('detail')}",
            flush=True,
        )
    else:
        print(
            f"BUZZ_ADAPTER on_wake action={action} id={eid[:12]}",
            flush=True,
        )

    if own_guard:
        save_guard(session.runtime, session.seat, guard)
    session.last_health_at = now
    save_session(session)
    return decision


def inject_many(
    runtime: str,
    seat: str,
    wakes: list[dict[str, Any]],
    *,
    single_turn: bool = True,
) -> list[dict[str, Any]]:
    runtime = resolve_runtime(runtime)
    session = load_session(runtime, seat)
    if not session:
        raise SystemExit(f"not armed: runtime={runtime} seat={seat} — run arm first")
    budget = budget_from_env()
    guard = load_guard(runtime, seat)
    if single_turn:
        new_turn(guard)
    results = []
    for i, raw in enumerate(wakes):
        results.append(
            on_wake(
                session,
                raw,
                budget=budget,
                start_turn=(not single_turn),
                guard=guard,
            )
        )
        # session is mutated in place; keep same object for transport_seen
    save_guard(runtime, seat, guard)
    save_session(session)
    return results


def make_demo_wake(i: int, session: AdapterSession, **kw: Any) -> dict[str, Any]:
    eid = f"{i:02d}" + ("a" * 62)
    w = {
        "schema": SCHEMA,
        "event_id": eid,
        "channel_id": session.room or "00000000-0000-0000-0000-000000000000",
        "t": "team.v0.room.message",
        "urgency": kw.pop("urgency", "P1" if i == 0 else "P2"),
        "seat_id": session.seat,
        "pubkey": "ce" + ("0" * 62),
        "summary": kw.pop("summary", f"demo wake {i} third-runtime stub"),
        "received_at": int(time.time()) + i,
    }
    w.update(kw)
    return w


# --- messages watch bridge (CLI owns WS; adapter owns on_wake) ---------------


def find_buzz_cli() -> Optional[str]:
    """Prefer watch-capable binary (BUZZ_CLI, workspace builds, then PATH)."""
    candidates: list[str] = []
    env = os.environ.get("BUZZ_CLI")
    if env:
        candidates.append(env)
    home = Path.home()
    candidates.extend(
        [
            str(home / "PROJECTS" / " buzz" / "target" / "release" / "buzz"),
            str(home / "PROJECTS" / " buzz" / "target" / "debug" / "buzz"),
            str(home / "PROJECTS" / "buzz" / "target" / "release" / "buzz"),
            str(home / ".local" / "bin" / "buzz-watch-f4"),
            str(home / ".local" / "bin" / "buzz-watch-f3"),
            str(home / ".local" / "bin" / "buzz"),
        ]
    )
    which = shutil.which("buzz")
    if which:
        candidates.append(which)
    seen: set[str] = set()
    for path in candidates:
        if not path or path in seen:
            continue
        seen.add(path)
        if Path(path).is_file() and os.access(path, os.X_OK):
            return path
    return None


def cli_supports_watch(cli: str) -> bool:
    try:
        result = subprocess.run(
            [cli, "messages", "--help"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        return False
    text = (result.stdout or "") + (result.stderr or "")
    return bool(re.search(r"\bwatch\b", text, re.I))


def classify_urgency(content: str, tags: Any, seat: str) -> str:
    body = (content or "").lower()
    seat_l = seat.lower()
    if seat_l and (seat_l in body or seat.replace("-", " ").lower() in body):
        return "P0"
    has_p = any(isinstance(t, list) and len(t) >= 2 and t[0] == "p" for t in (tags or []))
    if has_p and any(w in body for w in ("hot-", "challenge", "ready", "ack", "complete")):
        return "P0"
    if any(m in body for m in ("hot-challenge", "hot-ws-", "⚡", "team.v0.task.completed")):
        return "P0"
    if any(m in body for m in ("ready", "ack", "complete", "team.v0.")):
        return "P1"
    return "P2"


def parse_team_fields(content: str) -> dict[str, Any]:
    """Best-effort extract of team.v0 markers from room text (not authority)."""
    out: dict[str, Any] = {}
    if not content:
        return out
    # t=team.v0.* or bare team.v0.*
    m = re.search(r"\b(team\.v0\.[a-z0-9_.]+)\b", content, re.I)
    if m:
        out["t"] = m.group(1)
    for key in ("task_id", "correlation_id", "lease_id"):
        m = re.search(rf"\b{key}\s*[=:]\s*[\"']?([A-Za-z0-9_.:-]+)", content, re.I)
        if m:
            out[key] = m.group(1)
    if "team.v0.task.completed" in content.lower() or "unblocked:" in content.lower():
        out.setdefault("t", "team.v0.task.completed")
    if "team.v0.agent.blocked" in content.lower():
        out.setdefault("t", "team.v0.agent.blocked")
    return out


def jsonl_fact_to_wake(fact: dict[str, Any], session: AdapterSession) -> Optional[dict[str, Any]]:
    """Map CLI `messages watch` JSONL fact → W1.1-ish wake dict.

    Returns None when the fact should be transport-advanced but not admitted
    (self, empty, wrong channel).
    """
    if not isinstance(fact, dict):
        return None
    mid = fact.get("id") or fact.get("event_id") or ""
    channel = fact.get("channel_id") or ""
    if session.room and channel and channel != session.room:
        print(
            f"BUZZ_WATCH ignore channel={channel[:12]} want={session.room[:12]}",
            flush=True,
        )
        return None
    pk = fact.get("pubkey") or ""
    if session.self_pubkey and pk and pk == session.self_pubkey:
        print(f"BUZZ_WATCH suppress self id={mid[:12]}", flush=True)
        return None
    content = fact.get("content")
    if content is None:
        content = ""
    if not isinstance(content, str):
        content = json.dumps(content, separators=(",", ":"))
    content = content.strip()
    if not content or content.lower() == "undefined":
        print(f"BUZZ_WATCH suppress empty id={mid[:12]}", flush=True)
        return None
    ts = int(fact.get("created_at") or 0)
    tags = fact.get("tags") or []
    team = parse_team_fields(content)
    summary = re.sub(r"\s+", " ", content)[:160]
    wake: dict[str, Any] = {
        "schema": SCHEMA,
        "event_id": mid,
        "channel_id": channel or session.room,
        "channel_name": session.room_name or None,
        "t": team.get("t") or "team.v0.room.message",
        "urgency": classify_urgency(content, tags, session.seat),
        "seat_id": session.seat,
        "pubkey": pk,
        "summary": summary,
        "content": content,
        "received_at": int(time.time()),
        "runtime": session.runtime,
        "created_at": ts,
        "id": mid,
        "from": pk,
        "preview": summary,
        "tags": tags,
    }
    for key in ("task_id", "correlation_id", "lease_id"):
        if team.get(key):
            wake[key] = team[key]
    return wake


def advance_watermark(session: AdapterSession, fact: dict[str, Any]) -> None:
    ts = int(fact.get("created_at") or 0)
    if ts > int(session.since or 0):
        session.since = ts


def process_jsonl_fact(
    session: AdapterSession,
    fact: dict[str, Any],
    *,
    guard: Optional[GuardState] = None,
    budget: Optional[WakeBudget] = None,
    start_turn: bool = True,
) -> Optional[dict[str, Any]]:
    """One CLI fact → optional on_wake. Always advances transport watermark."""
    advance_watermark(session, fact)
    mid = fact.get("id") or ""
    # Track transport seen even for suppressed self/empty so reconnect is quiet
    if mid and mid not in session.transport_seen:
        # on_wake also tracks admitted transport_seen for candidates;
        # mark pure suppresses here so they do not reappear as pending.
        pass
    wake = jsonl_fact_to_wake(fact, session)
    if wake is None:
        # still record id as transport-seen without admit
        if mid and mid not in session.transport_seen:
            session.transport_seen.append(mid)
            session.transport_seen = session.transport_seen[-200:]
        session.last_health_at = time.time()
        save_session(session)
        return {"action": Action.IGNORE.value, "reason": "filtered"}
    return on_wake(
        session,
        wake,
        budget=budget,
        start_turn=start_turn,
        guard=guard,
    )


def process_jsonl_stream(
    session: AdapterSession,
    lines: Iterable[str],
    *,
    shared_turn: bool = False,
    max_facts: int = 0,
) -> list[dict[str, Any]]:
    """Consume JSONL lines (one message object each) into on_wake."""
    budget = budget_from_env()
    guard = load_guard(session.runtime, session.seat)
    if shared_turn:
        new_turn(guard)
    results: list[dict[str, Any]] = []
    count = 0
    for line in lines:
        line = (line or "").strip()
        if not line:
            continue
        # CLI may print diagnostics on stdout in older builds; only parse JSON objects
        if not line.startswith("{"):
            print(f"BUZZ_WATCH skip non-json: {line[:80]}", flush=True)
            continue
        try:
            fact = json.loads(line)
        except json.JSONDecodeError:
            print(f"BUZZ_WATCH skip bad-json: {line[:80]}", flush=True)
            continue
        if not isinstance(fact, dict):
            continue
        dec = process_jsonl_fact(
            session,
            fact,
            guard=guard if shared_turn else None,
            budget=budget,
            start_turn=not shared_turn,
        )
        if dec is not None:
            results.append(dec)
        count += 1
        if max_facts > 0 and count >= max_facts:
            break
    if shared_turn:
        save_guard(session.runtime, session.seat, guard)
    save_session(session)
    return results


def run_watch_push(
    session: AdapterSession,
    *,
    cli: str,
    timeout: Optional[float] = None,
    limit: Optional[int] = None,
    max_facts: int = 0,
    shared_turn: bool = False,
) -> int:
    """Spawn `buzz messages watch` and pipe JSONL into process_jsonl_stream."""
    relay = os.environ.get("BUZZ_RELAY_URL") or ""
    cmd = [cli]
    if relay:
        cmd.extend(["--relay", relay])
    cmd.extend(["messages", "watch", "--channel", session.room, "--format", "jsonl"])
    since = int(session.since or 0)
    if since > 0:
        # Overlap one second; id-primary transport_seen makes this safe.
        cmd.extend(["--since", str(max(0, since - 1))])
    if timeout is not None and timeout > 0:
        cmd.extend(["--timeout", str(int(timeout) if timeout >= 1 else 1)])
    if limit is not None and limit > 0:
        cmd.extend(["--limit", str(int(limit))])

    session.transport = "push"
    save_session(session)
    print(
        f"BUZZ_WATCH armed runtime={session.runtime} seat={session.seat} "
        f"cli={cli} channel={session.room} since={since} mode=push",
        flush=True,
    )
    try:
        proc = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
    except OSError as exc:
        print(f"BUZZ_WATCH fail spawn: {exc}", flush=True)
        session.last_failure = monitor_failure(FailureReason.TRANSPORT, str(exc)[:120])
        save_session(session)
        return 2

    assert proc.stdout is not None
    assert proc.stderr is not None

    # Drain stderr diagnostics without secrets
    def _stderr_pump(stream: TextIO) -> None:
        for line in stream:
            line = line.rstrip("\n")
            if not line:
                continue
            # Never echo private keys if a buggy CLI ever leaked them
            if "PRIVATE" in line.upper() or "nsec1" in line:
                print("BUZZ_WATCH stderr=<redacted>", flush=True)
                continue
            print(f"BUZZ_WATCH {line}", flush=True)

    import threading

    err_thread = threading.Thread(target=_stderr_pump, args=(proc.stderr,), daemon=True)
    err_thread.start()

    try:
        process_jsonl_stream(
            session,
            proc.stdout,
            shared_turn=shared_turn,
            max_facts=max_facts,
        )
    except KeyboardInterrupt:
        proc.terminate()
    finally:
        try:
            proc.terminate()
        except OSError:
            pass
        try:
            rc = proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            rc = proc.wait()
        err_thread.join(timeout=1)

    print(f"BUZZ_WATCH stopped status={rc}", flush=True)
    if rc not in (0, None) and rc != 0:
        # timeout exits may be non-zero depending on CLI
        if timeout and rc != 0:
            return 0
    return int(rc or 0)


def run_watch_poll(
    session: AdapterSession,
    *,
    cli: str,
    tick_secs: float = 15.0,
    max_ticks: int = 0,
    max_facts: int = 0,
    shared_turn: bool = False,
) -> int:
    """Poll fallback: messages get --since loop when watch is unavailable."""
    relay = os.environ.get("BUZZ_RELAY_URL") or ""
    session.transport = "poll"
    save_session(session)
    print(
        f"BUZZ_WATCH armed runtime={session.runtime} seat={session.seat} "
        f"cli={cli} channel={session.room} since={session.since} mode=poll tick={tick_secs}s",
        flush=True,
    )
    ticks = 0
    facts_total = 0
    budget = budget_from_env()
    while True:
        ticks += 1
        cmd = [cli]
        if relay:
            cmd.extend(["--relay", relay])
        cmd.extend(
            [
                "messages",
                "get",
                "--channel",
                session.room,
                "--limit",
                "20",
            ]
        )
        if session.since:
            cmd.extend(["--since", str(max(0, int(session.since) - 1))])
        try:
            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=60, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            print(f"BUZZ_WATCH poll error: {exc}", flush=True)
            session.last_failure = monitor_failure(FailureReason.TRANSPORT, str(exc)[:120])
            save_session(session)
            time.sleep(tick_secs)
            continue
        if result.returncode != 0:
            err = (result.stderr or result.stdout or "").strip().splitlines()
            detail = err[-1] if err else f"exit={result.returncode}"
            print(f"BUZZ_WATCH poll fail: {detail[:160]}", flush=True)
            time.sleep(tick_secs)
            continue
        raw = (result.stdout or "").strip()
        try:
            data = json.loads(raw or "[]")
        except json.JSONDecodeError:
            data = []
        if isinstance(data, dict):
            data = data.get("messages") or data.get("events") or []
        if not isinstance(data, list):
            data = []
        # Emit as JSONL through the same path
        lines = [json.dumps(m) for m in data if isinstance(m, dict)]
        before = facts_total
        # For poll, use shared_turn=False per message by default
        decs = process_jsonl_stream(
            session,
            lines,
            shared_turn=shared_turn,
            max_facts=(max_facts - facts_total) if max_facts > 0 else 0,
        )
        facts_total += len(lines)
        if max_facts > 0 and facts_total >= max_facts:
            print(f"BUZZ_WATCH poll max_facts={max_facts}", flush=True)
            return 0
        if max_ticks > 0 and ticks >= max_ticks:
            print(f"BUZZ_WATCH poll max_ticks={max_ticks}", flush=True)
            return 0
        _ = decs, before, budget
        session.last_health_at = time.time()
        save_session(session)
        time.sleep(tick_secs)


def cmd_watch(args: argparse.Namespace) -> int:
    """Wire CLI messages watch (or poll fallback) → on_wake."""
    runtime = resolve_runtime(args.runtime)
    session = load_session(runtime, args.seat)
    if not session:
        if not args.room:
            print("error: not armed — pass --room or run arm first", file=sys.stderr)
            return 2
        session = arm(
            runtime,
            args.seat,
            room=args.room,
            room_name=args.room_name or "",
            transport="push",
            since=args.since,
        )
    elif args.room and args.room != session.room:
        session = arm(
            runtime,
            args.seat,
            room=args.room,
            room_name=args.room_name or session.room_name,
            transport="push",
            since=args.since,
        )
    if args.since is not None:
        session.since = int(args.since)
        save_session(session)
    if args.self_pubkey:
        session.self_pubkey = args.self_pubkey
        save_session(session)
    elif not session.self_pubkey and os.environ.get("BUZZ_PUBLIC_KEY"):
        session.self_pubkey = os.environ["BUZZ_PUBLIC_KEY"]
        save_session(session)

    # stdin mode: no CLI required (tests + offline inject of recorded JSONL)
    if args.from_stdin:
        session.transport = "push"
        save_session(session)
        print(
            f"BUZZ_WATCH armed runtime={session.runtime} seat={session.seat} "
            f"mode=stdin channel={session.room}",
            flush=True,
        )
        process_jsonl_stream(
            session,
            sys.stdin,
            shared_turn=args.shared_turn,
            max_facts=args.max_facts,
        )
        status(runtime, args.seat)
        return 0

    if args.file:
        session.transport = "push"
        save_session(session)
        text = Path(args.file).read_text().splitlines()
        process_jsonl_stream(
            session,
            text,
            shared_turn=args.shared_turn,
            max_facts=args.max_facts,
        )
        status(runtime, args.seat)
        return 0

    cli = args.cli or find_buzz_cli()
    if not cli:
        print("error: buzz CLI not found — set BUZZ_CLI", file=sys.stderr)
        return 2

    mode = args.mode  # auto | push | poll
    use_push = False
    if mode == "push":
        if not cli_supports_watch(cli):
            print(f"error: CLI lacks messages watch: {cli}", file=sys.stderr)
            return 2
        use_push = True
    elif mode == "poll":
        use_push = False
    else:  # auto
        use_push = cli_supports_watch(cli)
        print(
            f"BUZZ_WATCH push-detect cli={cli} watch={'yes' if use_push else 'no'}",
            flush=True,
        )

    if use_push:
        return run_watch_push(
            session,
            cli=cli,
            timeout=args.timeout,
            limit=args.limit,
            max_facts=args.max_facts,
            shared_turn=args.shared_turn,
        )
    return run_watch_poll(
        session,
        cli=cli,
        tick_secs=args.tick,
        max_ticks=args.max_ticks,
        max_facts=args.max_facts,
        shared_turn=args.shared_turn,
    )


# --- CLI --------------------------------------------------------------------


def cmd_arm(args: argparse.Namespace) -> int:
    arm(
        args.runtime,
        args.seat,
        args.room,
        room_name=args.room_name or "",
        transport=args.transport,
        self_pubkey=getattr(args, "self_pubkey", "") or "",
        since=getattr(args, "since", None),
        driver=getattr(args, "driver", "") or "",
    )
    status(args.runtime, args.seat)
    return 0


def cmd_drivers(args: argparse.Namespace) -> int:
    from drivers import list_drivers, resolve_driver_name

    current = resolve_driver_name(getattr(args, "driver", None))
    print(f"BUZZ_DRIVER list={','.join(list_drivers())} default={current}", flush=True)
    for name in list_drivers():
        print(f"BUZZ_DRIVER available name={name}", flush=True)
    return 0


def cmd_disarm(args: argparse.Namespace) -> int:
    disarm(args.runtime, args.seat)
    return 0


def cmd_status(args: argparse.Namespace) -> int:
    status(args.runtime, args.seat)
    return 0


def cmd_health(args: argparse.Namespace) -> int:
    health(args.runtime, args.seat, stale_after_secs=args.stale_after)
    return 0


def cmd_inject(args: argparse.Namespace) -> int:
    wakes: list[dict[str, Any]] = []
    if args.json:
        wakes.append(json.loads(args.json))
    if args.file:
        text = Path(args.file).read_text()
        text = text.strip()
        if text.startswith("["):
            wakes.extend(json.loads(text))
        else:
            for line in text.splitlines():
                line = line.strip()
                if not line:
                    continue
                wakes.append(json.loads(line))
    if not wakes:
        print("error: provide --json or --file", file=sys.stderr)
        return 2
    inject_many(args.runtime, args.seat, wakes, single_turn=not args.new_turn_each)
    status(args.runtime, args.seat)
    return 0


def cmd_demo_overflow(args: argparse.Namespace) -> int:
    """Four wakes in one turn → 3 AdmitCortex + loud overflow (v0.2 dogfood)."""
    runtime = resolve_runtime(args.runtime)
    session = load_session(runtime, args.seat)
    if not session:
        # auto-arm synthetic room for local proof
        session = arm(
            runtime,
            args.seat,
            room=args.room or "00000000-0000-0000-0000-000000000099",
            room_name="stub-demo",
            transport="poll",
        )
    # force clean turn budgets for demo
    os.environ.setdefault("BUZZ_ADMIT_COOLDOWN_SECS", "0")
    wakes = [make_demo_wake(i, session) for i in range(4)]
    # reset guard for clean overflow demo if requested
    if args.reset_guard:
        save_guard(runtime, args.seat, GuardState())
        session.transport_seen = []
        save_session(session)
    results = inject_many(runtime, args.seat, wakes, single_turn=True)
    admits = sum(1 for r in results if r.get("action") == Action.ADMIT.value)
    overflows = sum(1 for r in results if r.get("action") == Action.OVERFLOW.value)
    print(
        f"BUZZ_ADAPTER demo-overflow admits={admits} overflows={overflows} "
        f"expected_admits=3 expected_overflow=1",
        flush=True,
    )
    status(runtime, args.seat)
    return 0 if admits == 3 and overflows == 1 else 1


def cmd_notify(args: argparse.Namespace) -> int:
    """Human-only path: never AdmitCortex."""
    runtime = resolve_runtime(args.runtime)
    session = load_session(runtime, args.seat)
    if not session:
        print("error: not armed", file=sys.stderr)
        return 2
    wake = normalize_wake(json.loads(args.json) if args.json else {"summary": args.summary or "notify"}, session)
    print(
        f"BUZZ_ADAPTER on_wake action=NotifyHuman id={(wake.get('event_id') or '')[:12]} "
        f"summary={wake.get('summary', '')[:80]}",
        flush=True,
    )
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)

    def add_common(sp: argparse.ArgumentParser) -> None:
        sp.add_argument("--runtime", default=DEFAULT_RUNTIME, help="local-llm | antigravity | …")
        sp.add_argument("--seat", default="demo-llm")

    sp = sub.add_parser("arm", help="Arm adapter state for a room")
    add_common(sp)
    sp.add_argument("--room", required=True, help="channel UUID")
    sp.add_argument("--room-name", default="")
    sp.add_argument("--transport", default="poll", choices=("poll", "push"))
    sp.add_argument(
        "--driver",
        default="",
        help="product sink: none|notify|local-llm|antigravity (env BUZZ_DRIVER)",
    )
    sp.add_argument(
        "--since",
        type=int,
        default=None,
        help="transport watermark (default: now, skip history)",
    )
    sp.add_argument("--self-pubkey", default="", help="suppress self facts (else BUZZ_PUBLIC_KEY)")
    sp.set_defaults(func=cmd_arm)

    sp = sub.add_parser("drivers", help="List product driver hooks")
    sp.add_argument("--driver", default="", help="show resolved default")
    sp.set_defaults(func=cmd_drivers)

    sp = sub.add_parser("disarm", help="Drop session marker")
    add_common(sp)
    sp.set_defaults(func=cmd_disarm)

    sp = sub.add_parser("status", help="Print BUZZ_MONITOR line")
    add_common(sp)
    sp.set_defaults(func=cmd_status)

    sp = sub.add_parser("health", help="push|poll|stale")
    add_common(sp)
    sp.add_argument("--stale-after", type=float, default=120.0)
    sp.set_defaults(func=cmd_health)

    sp = sub.add_parser("inject", help="Feed W1.1 wake JSON into on_wake")
    add_common(sp)
    sp.add_argument("--json", default="", help="single wake JSON object")
    sp.add_argument("--file", default="", help="JSON array or JSONL")
    sp.add_argument(
        "--new-turn-each",
        action="store_true",
        help="call new_turn before every wake (default: one turn for the batch)",
    )
    sp.set_defaults(func=cmd_inject)

    sp = sub.add_parser("demo-overflow", help="Synthetic 4-wake overflow proof")
    add_common(sp)
    sp.add_argument("--room", default="")
    sp.add_argument("--reset-guard", action="store_true", default=True)
    sp.add_argument("--no-reset-guard", action="store_false", dest="reset_guard")
    sp.set_defaults(func=cmd_demo_overflow)

    sp = sub.add_parser("notify", help="NotifyHuman path (no AdmitCortex)")
    add_common(sp)
    sp.add_argument("--json", default="")
    sp.add_argument("--summary", default="human notify")
    sp.set_defaults(func=cmd_notify)

    sp = sub.add_parser(
        "watch",
        help="Wire buzz messages watch (JSONL) → on_wake; poll fallback",
    )
    add_common(sp)
    sp.add_argument("--room", default="", help="channel UUID (auto-arm if not armed)")
    sp.add_argument("--room-name", default="")
    sp.add_argument(
        "--mode",
        default="auto",
        choices=("auto", "push", "poll"),
        help="auto feature-detects messages watch (default)",
    )
    sp.add_argument("--cli", default="", help="override buzz binary (else BUZZ_CLI / discover)")
    sp.add_argument("--since", type=int, default=None, help="resume watermark")
    sp.add_argument("--self-pubkey", default="")
    sp.add_argument("--timeout", type=float, default=None, help="CLI watch --timeout secs")
    sp.add_argument("--limit", type=int, default=None, help="CLI watch --limit if supported")
    sp.add_argument("--tick", type=float, default=15.0, help="poll fallback interval")
    sp.add_argument("--max-ticks", type=int, default=0, help="poll exit after N ticks (0=forever)")
    sp.add_argument("--max-facts", type=int, default=0, help="stop after N JSONL facts (0=forever)")
    sp.add_argument(
        "--shared-turn",
        action="store_true",
        help="one v0.2 turn budget across facts (overflow demo); default=new turn per fact",
    )
    sp.add_argument("--from-stdin", action="store_true", help="read JSONL from stdin (no CLI)")
    sp.add_argument("--file", default="", help="read recorded JSONL file (no CLI)")
    sp.set_defaults(func=cmd_watch)

    return p


def main(argv: Optional[list[str]] = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
