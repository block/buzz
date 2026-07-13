"""Thin subprocess wrapper over the ``buzz`` CLI — the production client path."""

from __future__ import annotations

import json
import subprocess
from typing import Any


class BuzzCliError(RuntimeError):
    """A buzz CLI invocation failed."""


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
        """Run a buzz subcommand and return its parsed JSON stdout."""
        command = [self._binary, *args]
        try:
            completed = subprocess.run(
                command,
                capture_output=True,
                text=True,
                timeout=self._timeout,
                env={
                    "BUZZ_RELAY_URL": self._relay_url,
                    "BUZZ_PRIVATE_KEY": self._secret_key,
                    "BUZZ_AUTH_TAG": self._auth_tag,
                    "PATH": _path(),
                },
            )
        except (OSError, subprocess.TimeoutExpired) as error:
            raise BuzzCliError(f"buzz {args[0]}: {error}") from error
        if completed.returncode != 0:
            raise BuzzCliError(
                f"buzz {' '.join(args)} exited {completed.returncode}: "
                f"{completed.stderr.strip() or completed.stdout.strip()}"
            )
        if not completed.stdout.strip():
            return None
        try:
            return json.loads(completed.stdout)
        except json.JSONDecodeError as error:
            raise BuzzCliError(
                f"buzz {args[0]} returned non-JSON output: {completed.stdout[:200]!r}"
            ) from error

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
