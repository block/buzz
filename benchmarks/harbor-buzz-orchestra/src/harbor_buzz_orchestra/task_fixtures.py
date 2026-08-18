"""Public setup declarations for Buzz-native benchmark tasks."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DirectoryEntry:
    """A named identity to seed into the benchmark community."""

    name: str
    role: str


@dataclass(frozen=True, slots=True)
class BuzzTaskFixture:
    """Relay state a task needs before the agent receives its prompt."""

    directory: tuple[DirectoryEntry, ...] = ()
    observe_channel_names: tuple[str, ...] = ()
    user_display_name: str | None = None


CREATE_CHANNEL_TASK = "create-channel-invite-users"
CREATE_CHANNEL_NAME = "fix-pr-1234"
TARGET_USERS = ("benchmark-user-07", "benchmark-user-19", "benchmark-user-42")
TARGET_BOTS = ("benchmark-bot-03", "benchmark-bot-08")
USER_MENTION_TASK = "user-mention"
USER_MENTION_DISPLAY_NAME = "John Vincent Doe"

_CREATE_CHANNEL_FIXTURE = BuzzTaskFixture(
    directory=tuple(
        [
            DirectoryEntry(f"benchmark-user-{index:02d}", "user")
            for index in range(1, 51)
        ]
        + [
            DirectoryEntry(f"benchmark-bot-{index:02d}", "bot")
            for index in range(1, 11)
        ]
    ),
    observe_channel_names=(CREATE_CHANNEL_NAME,),
)

_USER_MENTION_FIXTURE = BuzzTaskFixture(
    user_display_name=USER_MENTION_DISPLAY_NAME,
)


def fixture_for(task_name: str | None) -> BuzzTaskFixture:
    """Return the declared setup for a task, or an empty setup."""
    if task_name == CREATE_CHANNEL_TASK:
        return _CREATE_CHANNEL_FIXTURE
    if task_name == USER_MENTION_TASK:
        return _USER_MENTION_FIXTURE
    return BuzzTaskFixture()
