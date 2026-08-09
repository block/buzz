#!/usr/bin/env python3
"""notify driver — human / desktop alert only; never posts to Buzz."""
from __future__ import annotations

import os
import shutil
import subprocess
from typing import Any

from .base import DriverContext, DriverResult


class NotifyDriver:
    name = "notify"

    def handle_admit(self, cortex: dict[str, Any], ctx: DriverContext) -> DriverResult:
        summary = (cortex.get("summary") or "")[:120]
        eid = (cortex.get("event_id") or "")[:12]
        urgency = cortex.get("urgency") or "P2"
        title = f"Buzz · {ctx.room_name or ctx.room[:8]} · {urgency}"
        body = f"{ctx.seat}: {summary} (id={eid})"

        if ctx.dry_run or os.environ.get("BUZZ_DRIVER_NOTIFY", "auto") == "0":
            return DriverResult(
                status="dry_run",
                driver=self.name,
                action="notify",
                detail=f"{title} | {body}",
                draft="",
                meta={"title": title, "body": body},
            )

        # Best-effort desktop notify; never fails the adapter hard.
        if shutil.which("notify-send"):
            try:
                subprocess.run(
                    ["notify-send", title, body],
                    check=False,
                    timeout=5,
                    capture_output=True,
                )
                return DriverResult(
                    status="ok",
                    driver=self.name,
                    action="notify",
                    detail="notify-send",
                    meta={"title": title},
                )
            except (OSError, subprocess.TimeoutExpired) as exc:
                return DriverResult(
                    status="error",
                    driver=self.name,
                    action="notify",
                    detail=str(exc)[:120],
                )

        return DriverResult(
            status="ok",
            driver=self.name,
            action="notify",
            detail="stdout-only (no notify-send)",
            meta={"title": title, "body": body},
        )
