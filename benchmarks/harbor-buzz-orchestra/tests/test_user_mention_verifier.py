import importlib.util
import json
from pathlib import Path

from harbor_buzz_orchestra.evidence import build_buzz_evidence
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle

# The Buzz-native tasks are a sibling dataset of this harness package.
DATASET_ROOT = Path(__file__).resolve().parents[2] / "buzz-dataset"
FIXTURES = Path(__file__).parent / "fixtures" / "transcripts"
VERIFIER = DATASET_ROOT / "user-mention" / "tests" / "verify.py"
SPEC = importlib.util.spec_from_file_location("user_mention_verifier", VERIFIER)
verifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(verifier)

CORRECT_ANSWER = "The annual cost is $5,328."


def _credential(agent_id: str, role: str, pubkey: str) -> AgentCredential:
    return AgentCredential(
        agent_id=agent_id,
        role=role,
        nostr_secret_key="secret",
        nostr_pubkey=pubkey,
        nostr_auth_tag="auth",
        llm_endpoint="model" if role != "user" else "",
        llm_api_key="key" if role != "user" else "",
    )


def _evidence(fixture: str, content: str = CORRECT_ANSWER) -> dict:
    transcript = json.loads((FIXTURES / fixture).read_text(encoding="utf-8"))
    root, answer = transcript["messages"]
    answer["content"] = content
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id=transcript["channel_id"],
        credentials=(_credential("solo-1", "orchestrator", answer["pubkey"]),),
        user=_credential("John Vincent Doe", "user", root["pubkey"]),
        task_name="user-mention",
    )
    return build_buzz_evidence(
        trial=trial,
        messages=transcript["messages"],
        task_event_id=root["id"],
        completion_message_id=None,
        transcript_limit=1000,
    )


def test_correct_answer_with_user_p_tag_passes():
    metrics, details = verifier.score_evidence(_evidence("threaded.json"))

    assert all(value == 1.0 for value in metrics.values())
    assert details["user_display_name"] == "John Vincent Doe"


def test_answer_text_without_p_tag_fails_delivery_mention():
    evidence = _evidence(
        "top-level.json", "@John Vincent Doe, the annual cost is $5,328."
    )

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["answer_correct"] == 1.0
    assert metrics["user_p_tagged"] == 0.0
    assert metrics["reward"] == 0.0


def test_p_tag_without_visible_display_name_passes():
    evidence = _evidence("threaded.json")

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["user_p_tagged"] == 1.0
    assert metrics["reward"] == 1.0


def test_wrong_answer_fails_correctness_only():
    evidence = _evidence("threaded.json", "The annual cost is $1.")

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["user_p_tagged"] == 1.0
    assert metrics["answer_correct"] == 0.0
    assert metrics["reward"] == 0.0


def test_missing_evidence_fails_closed():
    metrics, details = verifier.score_evidence(None)

    assert all(value == 0.0 for value in metrics.values())
    assert "error" in details
