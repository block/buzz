"""Typed boundary between the Harbor adapter and Buzz trial provisioning."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from .manifest import ExperimentManifest


@dataclass(frozen=True, slots=True)
class AgentCredential:
    """One trial-scoped Buzz identity and attributed LLM credential."""

    agent_id: str
    # The roster *kind* — "orchestrator" or "worker" — despite the name. It is
    # what the runtime checks a credential against, so it stays as it is.
    role: str
    nostr_secret_key: str
    nostr_pubkey: str
    nostr_auth_tag: str
    llm_endpoint: str
    llm_api_key: str
    # v1.3 (additive, hence last): the manifest's ``role`` string — "lead",
    # "implementer", "critic", "navigator". Distinct from ``role`` above
    # because two entries can share a kind and still be different jobs: the
    # three-agent critic condition has a worker and a critic, both
    # ``kind: worker``. This is the value teammates see in their prompt, and
    # personas address each other by it, so without it a lead cannot tell
    # which teammate edits and which one only verifies. Empty means "fall
    # back to the kind", which keeps older provisioners working.
    manifest_role: str = ""


@dataclass(frozen=True, slots=True)
class TrialHandle:
    """Provisioned Buzz resources owned by one Harbor trial."""

    run_id: str
    trial_id: str
    manifest_hash: str
    relay_ws_url: str
    channel_id: str
    credentials: tuple[AgentCredential, ...]
    # The trial's human-analogue identity: owns the channel, posts the task
    # as a user prompt, and receives the final report. Never runs an agent
    # process, so its llm_endpoint and llm_api_key are empty strings.
    user: AgentCredential
    # v1.2 (additive): the relay as reachable from the HOST, where the user
    # identity and the harness run. ``relay_ws_url`` is the view from the
    # agents' runtime (the task container). Empty means both views coincide.
    user_relay_url: str = ""


@runtime_checkable
class TrialProvisioner(Protocol):
    """Creates and tears down trial-isolated Buzz resources synchronously."""

    def create_trial(
        self,
        run_id: str,
        trial_id: str,
        manifest: ExperimentManifest,
        channel_label: str | None = None,
    ) -> TrialHandle: ...

    def teardown(self, handle: TrialHandle) -> None: ...

    def healthcheck(self) -> None: ...
