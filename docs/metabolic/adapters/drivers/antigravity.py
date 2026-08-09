#!/usr/bin/env python3
"""antigravity driver — product hook surface (stub until real API wired).

Implements the same handle_admit contract so the third-runtime path is real in
shape. When Antigravity CLI/SDK is available, set:

  BUZZ_DRIVER_ANTIGRAVITY_CMD   command receiving cortex JSON on stdin

Until then: dry_run / not_implemented with a clear draft for dogfood.
"""
from __future__ import annotations

import json
import os
import subprocess
from typing import Any

from .base import DriverContext, DriverResult, cortex_prompt


class AntigravityDriver:
    name = "antigravity"

    def handle_admit(self, cortex: dict[str, Any], ctx: DriverContext) -> DriverResult:
        cmd = (os.environ.get("BUZZ_DRIVER_ANTIGRAVITY_CMD") or "").strip()
        payload = {
            "schema": "metabolic.driver.admit.v0",
            "cortex": cortex,
            "seat": ctx.seat,
            "room": ctx.room,
            "room_name": ctx.room_name,
            "hitl": ctx.hitl,
            "prompt": cortex_prompt(cortex, ctx),
        }

        if not cmd:
            return DriverResult(
                status="not_implemented",
                driver=self.name,
                action="noop",
                detail=(
                    "Antigravity product hook ready; set "
                    "BUZZ_DRIVER_ANTIGRAVITY_CMD to enable"
                ),
                draft=(
                    f"[antigravity stub] AdmitCortex "
                    f"{(cortex.get('event_id') or '')[:12]} "
                    f"{(cortex.get('summary') or '')[:80]}"
                ),
                meta={"interface": "handle_admit", "payload_schema": payload["schema"]},
            )

        if ctx.dry_run:
            return DriverResult(
                status="dry_run",
                driver=self.name,
                action="draft",
                detail="cmd configured but dry_run=1",
                draft=json.dumps({"would_invoke": cmd, "event_id": cortex.get("event_id")}),
                meta={"cmd": cmd},
            )

        try:
            result = subprocess.run(
                cmd,
                shell=True,
                input=json.dumps(payload),
                text=True,
                capture_output=True,
                timeout=float(os.environ.get("BUZZ_DRIVER_ANTIGRAVITY_TIMEOUT", "90")),
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            return DriverResult(
                status="error",
                driver=self.name,
                action="draft",
                detail=str(exc)[:160],
            )

        draft = (result.stdout or "").strip()[:800]
        if result.returncode != 0 and not draft:
            err = (result.stderr or "").strip().splitlines()
            return DriverResult(
                status="error",
                driver=self.name,
                action="draft",
                detail=(err[-1] if err else f"exit={result.returncode}")[:160],
            )

        return DriverResult(
            status="ok",
            driver=self.name,
            action="draft",
            detail=f"cmd exit={result.returncode}",
            draft=draft or "NO_REPLY",
        )
