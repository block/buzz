#!/usr/bin/env python3
"""Product driver registry — AdmitCortex sinks only."""
from __future__ import annotations

import os
from typing import Any, Optional

from .antigravity import AntigravityDriver
from .base import DriverContext, DriverResult
from .local_llm import LocalLlmDriver
from .notify import NotifyDriver

# none = stdout cortex only (legacy stub behavior)
DRIVERS = {
    "none": None,
    "notify": NotifyDriver,
    "local-llm": LocalLlmDriver,
    "local_llm": LocalLlmDriver,
    "llm": LocalLlmDriver,
    "antigravity": AntigravityDriver,
    "agy": AntigravityDriver,
}

DEFAULT_DRIVER = "local-llm"


def list_drivers() -> list[str]:
    return sorted({k for k in DRIVERS if k not in ("local_llm", "llm", "agy")})


def resolve_driver_name(name: Optional[str] = None) -> str:
    raw = (name or os.environ.get("BUZZ_DRIVER") or DEFAULT_DRIVER).strip().lower()
    if raw in ("", "default"):
        raw = DEFAULT_DRIVER
    if raw not in DRIVERS:
        return DEFAULT_DRIVER
    return "local-llm" if raw in ("local_llm", "llm") else (
        "antigravity" if raw == "agy" else raw
    )


def get_driver(name: Optional[str] = None):
    key = resolve_driver_name(name)
    cls = DRIVERS.get(key)
    if cls is None:
        return None
    return cls()


def driver_context_from_session(session: Any, **overrides: Any) -> DriverContext:
    dry = overrides.pop("dry_run", None)
    if dry is None:
        dry = os.environ.get("BUZZ_DRIVER_DRY_RUN", "1") not in ("0", "false", "no")
    allow = overrides.pop("allow_reply", None)
    if allow is None:
        allow = os.environ.get("BUZZ_DRIVER_ALLOW_REPLY", "0") in ("1", "true", "yes")
    hitl = overrides.pop("hitl", None)
    if hitl is None:
        hitl = os.environ.get("BUZZ_DRIVER_HITL", "1") not in ("0", "false", "no")
    return DriverContext(
        runtime=getattr(session, "runtime", "local-llm"),
        seat=getattr(session, "seat", ""),
        room=getattr(session, "room", ""),
        room_name=getattr(session, "room_name", "") or "",
        transport=getattr(session, "transport", "poll"),
        hitl=hitl,
        allow_reply=bool(allow),
        dry_run=bool(dry),
        **{k: v for k, v in overrides.items() if k in DriverContext.__dataclass_fields__},
    )


def invoke_driver(
    driver_name: Optional[str],
    cortex: dict[str, Any],
    session: Any,
    **ctx_overrides: Any,
) -> Optional[DriverResult]:
    """Run product sink after AdmitCortex. None if driver=none."""
    name = resolve_driver_name(driver_name or getattr(session, "driver", None))
    if name == "none":
        return None
    driver = get_driver(name)
    if driver is None:
        return None
    ctx = driver_context_from_session(session, **ctx_overrides)
    result = driver.handle_admit(cortex, ctx)
    return result


def print_driver_result(result: DriverResult) -> None:
    line = (
        f"BUZZ_DRIVER status={result.status} driver={result.driver} "
        f"action={result.action} detail={result.detail[:120]}"
    )
    print(line, flush=True)
    if result.draft:
        # Single line for monitors; full draft may be multi-line → collapse
        flat = " ".join(result.draft.split())
        print(f"BUZZ_DRIVER draft={flat[:400]}", flush=True)
