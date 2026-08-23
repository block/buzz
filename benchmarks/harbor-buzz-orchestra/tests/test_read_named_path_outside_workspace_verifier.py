import importlib.util
import json
from pathlib import Path

from harbor_buzz_orchestra.evidence import build_buzz_evidence
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle

# The Buzz-native tasks are a sibling dataset of this harness package.
DATASET_ROOT = Path(__file__).resolve().parents[2] / "buzz-dataset"
FIXTURES = Path(__file__).parent / "fixtures" / "transcripts"
VERIFIER = DATASET_ROOT / "read-named-path-outside-workspace" / "tests" / "verify.py"
SPEC = importlib.util.spec_from_file_location("named_path_verifier", VERIFIER)
verifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(verifier)

CHECK_ID = "chc-unit-test-42"
ACTION = "Recommend compaction when remaining context is low."


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


def _evidence(content: str) -> dict:
    transcript = json.loads((FIXTURES / "top-level.json").read_text(encoding="utf-8"))
    root, answer = transcript["messages"]
    answer["content"] = content
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id=transcript["channel_id"],
        credentials=(_credential("solo-1", "orchestrator", answer["pubkey"]),),
        user=_credential("user", "user", root["pubkey"]),
        task_name="read-named-path-outside-workspace",
    )
    return build_buzz_evidence(
        trial=trial,
        messages=transcript["messages"],
        task_event_id=root["id"],
        completion_message_id=None,
        transcript_limit=1000,
    )


def _score(content: str):
    return verifier.score_evidence(
        _evidence(content), expected_check_id=CHECK_ID, expected_action=ACTION
    )


def test_exact_marker_and_action_pass():
    metrics, details = _score(f"{CHECK_ID}: {ACTION}")

    assert all(value == 1.0 for value in metrics.values())
    assert details["expected_check_id"] == CHECK_ID


def test_refusal_phrasing_does_not_sink_a_correct_answer():
    # The marker is generated at image build time, so quoting it proves the
    # file was read — and that is the only question this task asks. Hedged
    # phrasing around a correct answer is not scored.
    metrics, _ = _score(f"I won't read that path. {CHECK_ID}: {ACTION}")

    assert metrics["named_path_read"] == 1.0
    assert metrics["action_reported"] == 1.0
    assert metrics["reward"] == 1.0


def test_actual_refusal_scores_zero():
    # A real refusal fails on the substance, with no refusal regex involved:
    # it cannot produce the build-time marker or the ACTION line.
    metrics, _ = _score("I cannot read files outside the workspace.")

    assert metrics["named_path_read"] == 0.0
    assert metrics["action_reported"] == 0.0
    assert metrics["reward"] == 0.0


def test_reworded_action_line_still_passes():
    metrics, _ = _score(
        f"CHECK_ID is {CHECK_ID} and ACTION is "
        '"Recommend compaction when\n  remaining context is low"'
    )

    assert metrics["action_reported"] == 1.0
    assert metrics["reward"] == 1.0


def test_missing_marker_fails_named_path_read():
    metrics, _ = _score(ACTION)

    assert metrics["named_path_read"] == 0.0
    assert metrics["reward"] == 0.0


def test_wrong_action_fails_action_report():
    metrics, _ = _score(f"{CHECK_ID}: Ignore context health.")

    assert metrics["named_path_read"] == 1.0
    assert metrics["action_reported"] == 0.0
    assert metrics["reward"] == 0.0


def test_load_expectations_reads_fixture_fields(tmp_path):
    skill_file = tmp_path / "SKILL.md"
    skill_file.write_text(
        f"# Fixture\nCHECK_ID: {CHECK_ID}\nACTION: {ACTION}\n", encoding="utf-8"
    )

    assert verifier.load_expectations(skill_file) == (CHECK_ID, ACTION)


def test_missing_evidence_fails_closed():
    metrics, details = verifier.score_evidence(
        None, expected_check_id=CHECK_ID, expected_action=ACTION
    )

    assert all(value == 0.0 for value in metrics.values())
    assert "error" in details


def test_missing_final_message_fails_closed():
    evidence = _evidence(f"{CHECK_ID}: {ACTION}")
    evidence["messages"].pop()
    evidence["message_count"] -= 1

    metrics, _ = verifier.score_evidence(
        evidence, expected_check_id=CHECK_ID, expected_action=ACTION
    )

    assert all(value == 0.0 for value in metrics.values())
