"""Thin subprocess wrapper over the ``buzz`` CLI — the production client path."""

from __future__ import annotations

import json
import random
import re
import subprocess
import time
from typing import Any

# How many times a relay-side "try again" is actually tried again.
#
# The relay caps API calls per identity per minute and rejects the overflow with
# a 429 it explicitly marks ``"retryable":true`` — the request never executed,
# and the identical request a moment later succeeds. Provisioning is bursty by
# construction: every concurrent trial creates a channel and adds its members at
# once, all as the same pinned user, so the burst lands on one counter. Treating
# that rejection as fatal scored a Terminal-Bench task zero for a reason that has
# nothing to do with the agent — one qemu-alpine-ssh trial died exactly that way
# six minutes into a full sweep, and which tasks it hits is a matter of timing.
#
# Seven attempts is the first count whose backoff (1+2+4+8+16+32 = 63s, before
# jitter, which only adds) outlasts the relay's 60-second fixed window. Six
# summed to ~35s, so every attempt would have landed inside the same window that
# was already rejecting them and the retry would have bought nothing.
MAX_ATTEMPTS = 7

# Grows to ~32s by the final attempt; capped so one wedged call cannot stall a
# trial past its budget.
_BACKOFF_BASE_SECONDS = 1.0
_BACKOFF_CAP_SECONDS = 32.0

_RETRY_HINT = re.compile(r"retry in (\d+)s")


class BuzzCliError(RuntimeError):
    """A buzz CLI invocation failed."""


def _is_retryable(message: str) -> bool:
    """Whether the relay said this failure would go away on its own.

    Keyed on the relay's own signals rather than on the exit code, because exit 2
    covers every network and relay error — including the permanent ones, which
    must stay fatal so a real misconfiguration is not retried into a timeout.
    """
    lowered = message.lower()
    return '"retryable":true' in lowered.replace(" ", "") or "rate-limited" in lowered


def _retry_delay(message: str, attempt: int) -> float:
    """How long to wait before retrying attempt ``attempt`` (1-based).

    Takes the larger of exponential backoff and the relay's own hint, then adds
    jitter. The jitter is the load-bearing part: every concurrent trial is
    rejected in the same instant by the same counter, so a fixed delay would send
    them all back together and reproduce the burst that caused the rejection.
    The relay's hint is frequently ``retry in 0s`` — the truthful answer for a
    fixed window about to roll over, and on its own an invitation to hot-loop.
    """
    backoff = min(_BACKOFF_BASE_SECONDS * (2 ** (attempt - 1)), _BACKOFF_CAP_SECONDS)
    hint = _RETRY_HINT.search(message)
    if hint:
        backoff = max(backoff, float(hint.group(1)))
    return backoff * random.uniform(1.0, 1.5)


class BuzzCli:
    """Run buzz CLI commands as one relay identity (key + NIP-OA auth tag)."""

    def __init__(
        self,
        relay_url: str,
        secret_key: str,
        auth_tag: str,
        *,
        binary: str = "buzz",
        timeout_seconds: float = 30.0,
    ) -> None:
        self._relay_url = relay_url
        self._secret_key = secret_key
        self._auth_tag = auth_tag
        self._binary = binary
        self._timeout = timeout_seconds

    def run(self, *args: str) -> Any:
        """Run a buzz subcommand and return its parsed JSON stdout.

        Retries failures the relay marks retryable; see ``MAX_ATTEMPTS``.
        """
        for attempt in range(1, MAX_ATTEMPTS + 1):
            completed = self._invoke(*args)
            if completed.returncode == 0:
                break
            detail = completed.stderr.strip() or completed.stdout.strip()
            if attempt < MAX_ATTEMPTS and _is_retryable(detail):
                time.sleep(_retry_delay(detail, attempt))
                continue
            raise BuzzCliError(
                f"buzz {' '.join(args)} exited {completed.returncode}"
                f"{f' after {attempt} attempts' if attempt > 1 else ''}: {detail}"
            )
        if not completed.stdout.strip():
            return None
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise BuzzCliError(
                f"buzz {args[0]} returned non-JSON output: {completed.stdout[:200]!r}"
            ) from error

    def _invoke(self, *args: str) -> subprocess.CompletedProcess[str]:
        """One attempt, with process-level failures still fatal.

        A spawn error or a hung call is not something the relay promised would
        resolve, so it is not retried here.
        """
        try:
            return subprocess.run(
                [self._binary, *args],
                capture_output=True,
                text=True,
                timeout=self._timeout,
                check=False,
                env={
                    "BUZZ_RELAY_URL": self._relay_url,
                    "BUZZ_PRIVATE_KEY": self._secret_key,
                    "BUZZ_AUTH_TAG": self._auth_tag,
                    "PATH": _path(),
                },
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise BuzzCliError(f"buzz {args[0]}: {error}") from error

    def create_private_channel(self, name: str, description: str) -> str:
        """Create a private stream channel; return its UUID."""
        response = self.run(
            "channels",
            "create",
            "--name",
            name,
            "--type",
            "stream",
            "--visibility",
            "private",
            "--description",
            description,
        )
        channel_id = response.get("channel_id") if isinstance(response, dict) else None
        if not channel_id:
            raise BuzzCliError(f"channel create returned no channel_id: {response}")
        return channel_id

    def add_member(self, channel_id: str, pubkey: str) -> None:
        self.run(
            "channels",
            "add-member",
            "--channel",
            channel_id,
            "--pubkey",
            pubkey,
            "--role",
            "member",
        )

    def archive_channel(self, channel_id: str) -> None:
        self.run("channels", "archive", "--channel", channel_id)


def _path() -> str:
    import os

    return os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin")
