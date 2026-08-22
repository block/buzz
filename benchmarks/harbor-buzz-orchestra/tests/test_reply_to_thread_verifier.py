import copy
import importlib.util
import json
from pathlib import Path

from harbor_buzz_orchestra.evidence import build_buzz_evidence
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle

# The Buzz-native tasks are a sibling dataset of this harness package.
DATASET_ROOT = Path(__file__).resolve().parents[2] / "buzz-dataset"
FIXTURES = Path(__file__).parent / "fixtures" / "transcripts"
VERIFIER = DATASET_ROOT / "reply-to-thread" / "tests" / "verify.py"
SPEC = importlib.util.spec_from_file_location("reply_to_thread_verifier", VERIFIER)
verifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(verifier)

CORRECT_ANSWER = (
    "Month 6 revenue: $160,811; month 6 expenses: $84,462; "
    "cumulative operating profit: $374,470."
)


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


def _evidence(fixture: str) -> dict:
    transcript = json.loads((FIXTURES / fixture).read_text(encoding="utf-8"))
    root, answer = transcript["messages"]
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id=transcript["channel_id"],
        credentials=(_credential("solo-1", "orchestrator", answer["pubkey"]),),
        user=_credential("user", "user", root["pubkey"]),
    )
    answer["content"] = CORRECT_ANSWER
    return build_buzz_evidence(
        trial=trial,
        messages=transcript["messages"],
        task_event_id=root["id"],
        completion_message_id=None,
        transcript_limit=1000,
    )


def test_correct_answer_in_direct_thread_reply_passes():
    metrics, details = verifier.score_evidence(_evidence("threaded.json"))

    assert metrics == {
        "reward": 1.0,
        "answer_correct": 1.0,
        "reply_to_thread": 1.0,
        "same_channel": 1.0,
        "expected_author": 1.0,
        "evidence_complete": 1.0,
    }
    assert details["selected_message_id"] is not None


def test_correct_top_level_answer_fails_only_threading_and_reward():
    metrics, _ = verifier.score_evidence(_evidence("top-level.json"))

    assert metrics["answer_correct"] == 1.0
    assert metrics["reply_to_thread"] == 0.0
    assert metrics["reward"] == 0.0


def test_wrong_answer_in_correct_thread_fails_correctness():
    evidence = _evidence("threaded.json")
    evidence["messages"][-1]["content"] = "Revenue $1, expenses $2, profit $3"

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["reply_to_thread"] == 1.0
    assert metrics["answer_correct"] == 0.0
    assert metrics["reward"] == 0.0


def test_work_showing_table_with_a_wrong_stated_answer_fails():
    # The month-6 rows are right, so an unanchored scan of every number in the
    # message would score this 1.0 despite the stated total being wrong.
    evidence = _evidence("threaded.json")
    evidence["messages"][-1]["content"] = (
        "Month 5: 153,153 / 82,806\n"
        "Month 6: 160,811 / 84,462\n"
        "Cumulative operating profit: $412,900"
    )

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["answer_correct"] == 0.0
    assert metrics["reward"] == 0.0


def test_labelled_multiline_answer_passes():
    evidence = _evidence("threaded.json")
    evidence["messages"][-1]["content"] = (
        "- Month 6 revenue: $160,811\n"
        "- Month 6 expenses: $84,462\n"
        "- Cumulative operating profit (months 1-6): $374,470"
    )

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["answer_correct"] == 1.0
    assert metrics["reward"] == 1.0


def test_reply_to_unrelated_event_fails_threading():
    evidence = _evidence("threaded.json")
    answer = evidence["messages"][-1]
    answer["reply_to_event_id"] = "unrelated"
    answer["tags"] = [
        ["h", evidence["trial"]["channel_id"]],
        ["e", "unrelated", "", "reply"],
    ]

    metrics, _ = verifier.score_evidence(evidence)

    assert metrics["answer_correct"] == 1.0
    assert metrics["reply_to_thread"] == 0.0
    assert metrics["reward"] == 0.0


def test_latest_agent_message_is_the_final_message_being_scored():
    evidence = _evidence("threaded.json")
    later = copy.deepcopy(evidence["messages"][-1])
    later.update(
        {
            "id": "later-top-level",
            "created_at": later["created_at"] + 1,
            "reply_to_event_id": None,
            "tags": [["h", evidence["trial"]["channel_id"]]],
        }
    )
    evidence["messages"].append(later)
    evidence["message_count"] += 1

    metrics, details = verifier.score_evidence(evidence)

    assert details["selected_message_id"] == "later-top-level"
    assert metrics["reply_to_thread"] == 0.0
    assert metrics["reward"] == 0.0


def test_missing_evidence_fails_closed():
    metrics, details = verifier.score_evidence(None)

    assert metrics["reward"] == 0.0
    assert all(value == 0.0 for value in metrics.values())
    assert "error" in details
