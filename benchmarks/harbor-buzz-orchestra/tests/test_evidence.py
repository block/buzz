import json
from pathlib import Path

from harbor_buzz_orchestra.evidence import build_buzz_evidence
from harbor_buzz_orchestra.provisioning import (
    AgentCredential,
    DirectoryIdentity,
    TrialHandle,
)

FIXTURES = Path(__file__).parent / "fixtures" / "transcripts"


def _credential(agent_id: str, role: str, pubkey: str) -> AgentCredential:
    return AgentCredential(
        agent_id=agent_id,
        role=role,
        nostr_secret_key=f"secret-{agent_id}",
        nostr_pubkey=pubkey,
        nostr_auth_tag=f"auth-{agent_id}",
        llm_endpoint="model" if role != "user" else "",
        llm_api_key="key" if role != "user" else "",
    )


def _load(name: str) -> dict:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def _trial(transcript: dict) -> TrialHandle:
    user_message, agent_message = transcript["messages"]
    return TrialHandle(
        run_id="run-1",
        trial_id="trial-1",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id=transcript["channel_id"],
        credentials=(_credential("solo-1", "orchestrator", agent_message["pubkey"]),),
        user=_credential("user", "user", user_message["pubkey"]),
    )


def _evidence(name: str) -> dict:
    transcript = _load(name)
    return build_buzz_evidence(
        trial=_trial(transcript),
        messages=list(reversed(transcript["messages"])),
        task_event_id=transcript["messages"][0]["id"],
        completion_message_id=transcript["messages"][-1]["id"],
        transcript_limit=1000,
    )


def test_normalizes_real_threaded_transcript_and_preserves_protocol_tags():
    evidence = _evidence("threaded.json")

    assert evidence["schema_version"] == 1
    assert evidence["message_count"] == 2
    assert evidence["messages"][0]["author_role"] == "user"
    reply = evidence["messages"][-1]
    assert reply["author"] == "solo-1"
    assert reply["author_role"] == "orchestrator"
    assert reply["channel_id"] == evidence["trial"]["channel_id"]
    assert reply["reply_to_event_id"] == evidence["task_event_id"]
    assert ["e", evidence["task_event_id"], "", "reply"] in reply["tags"]


def test_top_level_agent_message_has_no_derived_reply_destination():
    evidence = _evidence("top-level.json")

    assert evidence["messages"][-1]["reply_to_event_id"] is None


def test_malformed_messages_are_safe_and_secrets_are_never_exported():
    transcript = _load("threaded.json")
    trial = _trial(transcript)
    evidence = build_buzz_evidence(
        trial=trial,
        messages=[None, {"id": "broken", "tags": ["bad", ["e", 7]]}],
        task_event_id=None,
        completion_message_id=None,
        transcript_limit=1,
    )

    assert evidence["message_count"] == 1
    assert evidence["messages"][0]["tags"] == []
    assert evidence["truncated"] is True
    encoded = json.dumps(evidence)
    assert "nostr_secret_key" not in encoded
    assert "auth-user" not in encoded
    assert "secret-solo-1" not in encoded


def test_exports_only_public_directory_and_observed_channel_state():
    transcript = _load("threaded.json")
    trial = _trial(transcript)
    trial = TrialHandle(
        **{
            field: getattr(trial, field)
            for field in (
                "run_id",
                "trial_id",
                "manifest_hash",
                "relay_ws_url",
                "channel_id",
                "credentials",
                "user",
                "user_relay_url",
            )
        },
        task_name="create-channel-invite-users",
        directory=(DirectoryIdentity("benchmark-user-01", "user", "d" * 64),),
    )
    channels = [{"name": "fix-pr-1234", "members": []}]

    evidence = build_buzz_evidence(
        trial=trial,
        messages=[],
        task_event_id=None,
        completion_message_id=None,
        transcript_limit=1000,
        observed_channels=channels,
    )

    assert evidence["task_name"] == "create-channel-invite-users"
    assert evidence["directory"] == [
        {"name": "benchmark-user-01", "role": "user", "pubkey": "d" * 64}
    ]
    assert evidence["observed_channels"] == channels
