"""Public setup declarations for Buzz-native benchmark tasks."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class DirectoryEntry:
    """A named identity to seed into the benchmark community."""

    name: str
    role: str
    identity_id: str | None = None
    about: str | None = None
    channel_member: bool = False

    @property
    def stable_id(self) -> str:
        """Identity key used for deterministic credentials and scripted events."""
        return self.identity_id or self.name


@dataclass(frozen=True, slots=True)
class ScriptedMessage:
    """A relay event injected immediately after the task's triggering event."""

    label: str
    actor: str
    content: str
    reply_to_task: bool = False
    mention_orchestrator: bool = True


@dataclass(frozen=True, slots=True)
class MemorySeed:
    """A cold-memory value seeded under the orchestrator's identity."""

    slug: str
    value: str


@dataclass(frozen=True, slots=True)
class BuzzTaskFixture:
    """Relay state a task needs before the agent receives its prompt."""

    directory: tuple[DirectoryEntry, ...] = ()
    scripted_messages: tuple[ScriptedMessage, ...] = ()
    observe_channel_names: tuple[str, ...] = ()
    user_display_name: str | None = None
    memory_seeds: tuple[MemorySeed, ...] = ()
    # Whether the task's verifier grades the exported relay snapshot. Only
    # these tasks fail when the export fails; a Terminal-Bench task is graded
    # by its own tests and must not be errored by a snapshot hiccup.
    requires_evidence: bool = False


CREATE_CHANNEL_TASK = "create-channel-invite-users"
CREATE_CHANNEL_NAME = "fix-pr-1234"
TARGET_USERS = ("benchmark-user-07", "benchmark-user-19", "benchmark-user-42")
TARGET_BOTS = ("benchmark-bot-03", "benchmark-bot-08")
USER_MENTION_TASK = "user-mention"
USER_MENTION_DISPLAY_NAME = "John Vincent Doe"
REPLY_TO_THREAD_TASK = "reply-to-thread"
READ_NAMED_PATH_TASK = "read-named-path-outside-workspace"
MULTILINE_MESSAGE_TASK = "multiline-message"
NARRATIVE_AGENT_NAMES_TASK = "narrative-agent-names"
INTERLEAVED_AGENT_REPORTS_TASK = "interleaved-agent-reports"
CROSS_THREAD_REQUESTS_TASK = "cross-thread-requests"
AMBIGUOUS_USER_MENTION_TASK = "ambiguous-user-mention"
MEMORY_RETRIEVAL_TASK = "memory-retrieval"

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
    requires_evidence=True,
)

_USER_MENTION_FIXTURE = BuzzTaskFixture(
    user_display_name=USER_MENTION_DISPLAY_NAME,
    requires_evidence=True,
)

_NARRATIVE_AGENT_NAMES_FIXTURE = BuzzTaskFixture(
    directory=(
        DirectoryEntry("Aurora Audit Bot", "bot", channel_member=True),
        DirectoryEntry("Beacon Deploy Bot", "bot", channel_member=True),
    ),
    user_display_name="Maya Elise Chen",
    requires_evidence=True,
)

_INTERLEAVED_AGENT_REPORTS_FIXTURE = BuzzTaskFixture(
    directory=(
        DirectoryEntry("Ledger Scout", "bot", channel_member=True),
        DirectoryEntry("Risk Sentinel", "bot", channel_member=True),
        DirectoryEntry("Ops Forecaster", "bot", channel_member=True),
    ),
    scripted_messages=(
        ScriptedMessage(
            label="ledger-report",
            actor="Ledger Scout",
            content=(
                "@{orchestrator} Ledger report: North is 17 and South is 23. "
                "Use both values in the requested synthesis."
            ),
            reply_to_task=True,
        ),
        ScriptedMessage(
            label="risk-report",
            actor="Risk Sentinel",
            content=(
                "@{orchestrator} Risk report: subtract a reserve of 11 before "
                "applying the operations multiplier."
            ),
            reply_to_task=True,
        ),
        ScriptedMessage(
            label="operations-report",
            actor="Ops Forecaster",
            content=("@{orchestrator} Operations report: the final multiplier is 3."),
            reply_to_task=True,
        ),
    ),
    user_display_name="Nora Isabel Grant",
    requires_evidence=True,
)

_CROSS_THREAD_REQUESTS_FIXTURE = BuzzTaskFixture(
    scripted_messages=(
        ScriptedMessage(
            label="beta-request",
            actor="user",
            content=(
                "@{orchestrator} BETA request: calculate (84 / 7) + 29. "
                "Reply with the BETA result, labelled BETA."
            ),
        ),
    ),
    user_display_name="Priya Simone Patel",
    requires_evidence=True,
)

_AMBIGUOUS_USER_MENTION_FIXTURE = BuzzTaskFixture(
    directory=(
        DirectoryEntry(
            "Taylor Morgan Lee",
            "user",
            identity_id="taylor-release-captain",
            about="Release captain. Routing code ORCHID-72.",
            channel_member=True,
        ),
        DirectoryEntry(
            "Taylor Morgan Lee",
            "user",
            identity_id="taylor-observer",
            about="Compliance observer. Routing code COBALT-19.",
            channel_member=True,
        ),
    ),
    user_display_name="Olivia Grace Park",
    requires_evidence=True,
)


# Noisy memories test retrieval of one relevant value through `buzz mem ls/get`.
_MEMORY_RETRIEVAL_FIXTURE = BuzzTaskFixture(
    user_display_name="Amelia Rose Bennett",
    memory_seeds=(
        MemorySeed(
            slug="total-customers-per-month",
            value="We average 361,250 customers per month.",
        ),
        MemorySeed(
            slug="customer-value-metric",
            value="Last month we had 351,340 customers with a $2400 revenue per customer",
        ),
        MemorySeed(
            slug="customers-metrics-spring-24",
            value=(
                "In March, we had 325,401 total customers. In April, we had "
                "3,710 active customers named John."
            ),
        ),
        MemorySeed(
            slug="new-customers-april-2024",
            value="There are 21,604 new customers in April 2024.",
        ),
        MemorySeed(
            slug="total-customers-metric",
            value="In April 2024, we had 352,345 total customers.",
        ),
    ),
    requires_evidence=True,
)

_FIXTURES = {
    CREATE_CHANNEL_TASK: _CREATE_CHANNEL_FIXTURE,
    USER_MENTION_TASK: _USER_MENTION_FIXTURE,
    REPLY_TO_THREAD_TASK: BuzzTaskFixture(requires_evidence=True),
    READ_NAMED_PATH_TASK: BuzzTaskFixture(requires_evidence=True),
    MULTILINE_MESSAGE_TASK: BuzzTaskFixture(
        user_display_name="Eleanor June Brooks", requires_evidence=True
    ),
    NARRATIVE_AGENT_NAMES_TASK: _NARRATIVE_AGENT_NAMES_FIXTURE,
    INTERLEAVED_AGENT_REPORTS_TASK: _INTERLEAVED_AGENT_REPORTS_FIXTURE,
    CROSS_THREAD_REQUESTS_TASK: _CROSS_THREAD_REQUESTS_FIXTURE,
    AMBIGUOUS_USER_MENTION_TASK: _AMBIGUOUS_USER_MENTION_FIXTURE,
    MEMORY_RETRIEVAL_TASK: _MEMORY_RETRIEVAL_FIXTURE,
}


def fixture_for(task_name: str | None) -> BuzzTaskFixture:
    """Return the declared setup for a task, or an empty setup."""
    return _FIXTURES.get(task_name or "", BuzzTaskFixture())
