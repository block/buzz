#!/usr/bin/env python3
"""local-llm driver — bounded cortex sink for a process/CLI model.

Resolution order for the real CMD:
  1. BUZZ_DRIVER_LOCAL_LLM_CMD (explicit shell command; prompt on stdin)
  2. bundled drivers/run_local_llm.py (Ollama HTTP API, gemma3:4b default)

Safety:
  - dry_run default (BUZZ_DRIVER_DRY_RUN=1) → offline template, no model call
  - set BUZZ_DRIVER_DRY_RUN=0 to invoke real CMD
  - room content never executed as tools
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional

from .base import DriverContext, DriverResult, cortex_prompt


def resolve_local_llm_cmd() -> str:
    """Return shell command that reads prompt on stdin and prints draft."""
    explicit = (os.environ.get("BUZZ_DRIVER_LOCAL_LLM_CMD") or "").strip()
    if explicit:
        return explicit
    runner = Path(__file__).resolve().with_name("run_local_llm.py")
    if runner.is_file():
        # Quote paths — mono checkout may contain spaces ("PROJECTS/ buzz").
        py = shutil.which(sys.executable) or sys.executable
        return f"{_shell_quote(py)} {_shell_quote(str(runner))}"
    return ""


def _shell_quote(s: str) -> str:
    return "'" + s.replace("'", "'\"'\"'") + "'"


def ollama_reachable(host: Optional[str] = None, timeout: float = 1.5) -> bool:
    host = (host or os.environ.get("BUZZ_DRIVER_LOCAL_LLM_HOST") or "http://127.0.0.1:11434").rstrip(
        "/"
    )
    try:
        import urllib.request

        with urllib.request.urlopen(f"{host}/api/tags", timeout=timeout) as resp:
            return 200 <= resp.status < 300
    except Exception:
        return False


class LocalLlmDriver:
    name = "local-llm"

    def handle_admit(self, cortex: dict[str, Any], ctx: DriverContext) -> DriverResult:
        prompt = cortex_prompt(cortex, ctx)
        cmd = resolve_local_llm_cmd()

        if ctx.dry_run:
            summary = (cortex.get("summary") or "").strip()
            draft = (
                f"[local-llm dry_run] saw {cortex.get('t') or 'event'} "
                f"urgency={cortex.get('urgency') or 'P2'}: {summary[:200]}"
            )
            if "NO_REPLY" in summary.upper():
                draft = "NO_REPLY"
            detail = "offline summary draft (set BUZZ_DRIVER_DRY_RUN=0 for real model)"
            if cmd:
                detail += f"; cmd_ready={cmd.split()[-1] if cmd else ''}"
            return DriverResult(
                status="dry_run",
                driver=self.name,
                action="draft",
                detail=detail,
                draft=draft,
                meta={
                    "prompt_chars": len(prompt),
                    "hitl": ctx.hitl,
                    "cmd_configured": bool(cmd),
                },
            )

        if not cmd:
            return DriverResult(
                status="error",
                driver=self.name,
                action="draft",
                detail="no local-llm cmd (set BUZZ_DRIVER_LOCAL_LLM_CMD or ship run_local_llm.py)",
            )

        # Soft preflight when using bundled ollama runner
        if "run_local_llm.py" in cmd and not ollama_reachable():
            return DriverResult(
                status="error",
                driver=self.name,
                action="draft",
                detail="ollama unreachable at BUZZ_DRIVER_LOCAL_LLM_HOST (is ollama serve up?)",
                meta={"cmd": cmd},
            )

        timeout = float(os.environ.get("BUZZ_DRIVER_LOCAL_LLM_TIMEOUT") or "90")
        try:
            result = subprocess.run(
                cmd,
                shell=True,
                input=prompt,
                text=True,
                capture_output=True,
                timeout=timeout,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            return DriverResult(
                status="error",
                driver=self.name,
                action="draft",
                detail=str(exc)[:160],
            )

        draft = (result.stdout or "").strip() or "NO_REPLY"
        draft = draft[:800]
        if result.returncode != 0 and not (result.stdout or "").strip():
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
            draft=draft,
            meta={
                "allow_reply": ctx.allow_reply,
                "hitl": ctx.hitl,
                "which_shell": bool(shutil.which("sh")),
                "model": os.environ.get("BUZZ_DRIVER_LOCAL_LLM_MODEL") or "gemma3:4b",
            },
        )
