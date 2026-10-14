"""Positive and adversarial fixtures for the expanded Buzz-native tasks."""

from __future__ import annotations

import importlib.util
from pathlib import Path
from types import ModuleType

from harbor_buzz_orchestra.task_fixtures import fixture_for

DATASET_ROOT = Path(__file__).resolve().parents[2] / "buzz-dataset"
AGENT = "a" * 64
USER = "u" * 64
CHANNEL = "channel"
ROOT = "root"


def _verifier(task: str) -> ModuleType:
    path = DATASET_ROOT / task / "tests" / "verify.py"
    spec = importlib.util.spec_from_file_location(f"{task}_verifier", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def _message(
    message_id: str,
    content: str,
    *,
    reply_to: str | None = ROOT,
    mentions: list[str] | None = None,
    pubkey: str = AGENT,
) -> dict:
    tags = [["h", CHANNEL]]
    if reply_to is not None:
        tags.append(["e", reply_to, "", "reply"])
    tags.extend(["p", value] for value in (mentions or []))
    return {
        "id": message_id,
        "pubkey": pubkey,
        "content": content,
        "tags": tags,
        "channel_id": CHANNEL,
        "reply_to_event_id": reply_to,
        "mentioned_pubkeys": mentions or [],
    }


def _base(task: str, user_name: str) -> dict:
    return {
        "schema_version": 1,
        "task_name": task,
        "task_event_id": ROOT,
        "truncated": False,
        "trial": {"channel_id": CHANNEL},
        "identities": {
            "solo-1": {"role": "orchestrator", "pubkey": AGENT},
            user_name: {"role": "user", "pubkey": USER},
        },
        "directory": [],
        "scripted_events": [],
        "messages": [],
    }


def test_multiline_message_preserves_layout_and_rejects_literal_escapes():
    verifier = _verifier("multiline-message")
    evidence = _base("multiline-message", "Eleanor June Brooks")
    evidence["messages"] = [_message("answer", verifier.EXPECTED, mentions=[USER])]

    metrics, _ = verifier.score_evidence(evidence)
    assert all(value == 1.0 for value in metrics.values())

    evidence["messages"][0]["content"] = verifier.EXPECTED.replace("\n", "\\n")
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["real_newlines"] == 0.0
    assert metrics["reward"] == 0.0


def test_narrative_agent_names_do_not_wake_bots():
    verifier = _verifier("narrative-agent-names")
    evidence = _base("narrative-agent-names", "Maya Elise Chen")
    bot_a, bot_b = "b" * 64, "c" * 64
    evidence["directory"] = [
        {"name": "Aurora Audit Bot", "role": "bot", "pubkey": bot_a},
        {"name": "Beacon Deploy Bot", "role": "bot", "pubkey": bot_b},
    ]
    content = "Aurora Audit Bot completed the audit.\nBeacon Deploy Bot remains idle."
    evidence["messages"] = [_message("answer", content, mentions=[USER])]

    metrics, _ = verifier.score_evidence(evidence)
    assert all(value == 1.0 for value in metrics.values())

    evidence["messages"][0]["mentioned_pubkeys"].append(bot_a)
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["bots_not_mentioned"] == 0.0

    evidence["messages"] = [
        _message("bad-wake", "@Aurora Audit Bot please check", mentions=[bot_a]),
        _message("answer", content, mentions=[USER]),
    ]
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["bots_not_mentioned"] == 0.0
    assert metrics["reward"] == 0.0

    evidence["messages"] = [
        _message(
            "answer",
            "Aurora Audit Bot remains idle.\nBeacon Deploy Bot completed the audit.",
            mentions=[USER],
        )
    ]
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["status_correct"] == 0.0


def test_interleaved_reports_require_all_inputs_and_one_callback():
    verifier = _verifier("interleaved-agent-reports")
    evidence = _base("interleaved-agent-reports", "Nora Isabel Grant")
    reporters = [
        ("Ledger Scout", "b" * 64),
        ("Risk Sentinel", "c" * 64),
        ("Ops Forecaster", "d" * 64),
    ]
    evidence["directory"] = [
        {"name": name, "role": "bot", "pubkey": pubkey} for name, pubkey in reporters
    ]
    labels = ("ledger-report", "risk-report", "operations-report")
    evidence["scripted_events"] = [
        {"label": label, "event_id": f"report-{index}"}
        for index, label in enumerate(labels, start=1)
    ]
    evidence["messages"] = [
        _message(f"report-{index}", label, pubkey=reporters[index - 1][1])
        for index, label in enumerate(labels, start=1)
    ] + [
        _message(
            "answer",
            (
                "- **North:** 17\n- **South:** 23\n- **reserve:** 11\n"
                "- **multiplier:** 3\n\n(17 + 23 − 11) × 3 = **87**"
            ),
            mentions=[USER],
        )
    ]

    metrics, _ = verifier.score_evidence(evidence)
    assert all(value == 1.0 for value in metrics.values())

    evidence["messages"][-1]["content"] = "North 17; final forecast 87."
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["inputs_complete"] == 0.0

    evidence["messages"][-1]["content"] = (
        "North 17; South 23; reserve 11; multiplier 3; final forecast 87."
    )
    evidence["messages"].insert(
        -1,
        _message("bad-wake", "@Ledger Scout thanks", mentions=[reporters[0][1]]),
    )
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["reporters_not_rementioned"] == 0.0
    assert metrics["reward"] == 0.0


def test_cross_thread_requests_require_two_isolated_replies():
    verifier = _verifier("cross-thread-requests")
    evidence = _base("cross-thread-requests", "Priya Simone Patel")
    evidence["scripted_events"] = [{"label": "beta-request", "event_id": "beta-root"}]
    evidence["messages"] = [
        _message("alpha-answer", "ALPHA result: 346", mentions=[USER]),
        _message(
            "beta-answer",
            "BETA result: 41",
            reply_to="beta-root",
            mentions=[USER],
        ),
    ]

    metrics, _ = verifier.score_evidence(evidence)
    assert all(value == 1.0 for value in metrics.values())

    evidence["messages"] = [
        _message(
            "combined", "ALPHA 346; BETA 41", reply_to="beta-root", mentions=[USER]
        )
    ]
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["thread_isolation"] == 0.0
    assert metrics["reward"] == 0.0

    evidence["messages"] = [
        _message("alpha-answer", "ALPHA result: 346", mentions=[USER]),
        _message("extra", "Both requests handled", mentions=[USER]),
        _message(
            "beta-answer",
            "BETA result: 41",
            reply_to="beta-root",
            mentions=[USER],
        ),
    ]
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["thread_isolation"] == 0.0


def test_ambiguous_user_mention_targets_only_profile_match():
    verifier = _verifier("ambiguous-user-mention")
    evidence = _base("ambiguous-user-mention", "Olivia Grace Park")
    target, other = "b" * 64, "c" * 64
    evidence["directory"] = [
        {
            "identity_id": "taylor-release-captain",
            "name": "Taylor Morgan Lee",
            "role": "user",
            "pubkey": target,
        },
        {
            "identity_id": "taylor-observer",
            "name": "Taylor Morgan Lee",
            "role": "user",
            "pubkey": other,
        },
    ]
    evidence["messages"] = [
        _message(
            "delivery", "@Taylor Morgan Lee ORCHID-72 approved", mentions=[target]
        ),
        _message(
            "callback", "Sent to the matching Taylor Morgan Lee.", mentions=[USER]
        ),
    ]

    metrics, _ = verifier.score_evidence(evidence)
    assert all(value == 1.0 for value in metrics.values())

    evidence["messages"][0]["mentioned_pubkeys"].append(other)
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["other_not_notified"] == 0.0
    assert metrics["reward"] == 0.0


def test_memory_retrieval_requires_correct_threaded_answer():
    verifier = _verifier("memory-retrieval")
    evidence = _base("memory-retrieval", "Amelia Rose Bennett")
    question_id = "memory-question"
    evidence["task_event_id"] = question_id
    answer = _message(
        "answer",
        "We had 352,345 total customers in April 2024.",
        reply_to=question_id,
        mentions=[USER],
    )
    evidence["messages"] = [answer]

    for correct_answer in (
        "352,345",
        "We had 352,345 total customers in April 2024.",
        "April 2024 total customers: 352345",
    ):
        evidence["messages"][0]["content"] = correct_answer
        metrics, _ = verifier.score_evidence(evidence)
        assert all(value == 1.0 for value in metrics.values())

    for answer_without_exact_total in (
        "361,250",
        "351,340",
        "$2,400 revenue per customer",
        "325,401",
        "3,710",
        "21,604",
        "352,344",
        "352,346",
        "352,000",
        "About 352 thousand",
        "Approximately 352.3 thousand",
    ):
        evidence["messages"][0]["content"] = answer_without_exact_total
        metrics, _ = verifier.score_evidence(evidence)
        assert metrics["answer_correct"] == 0.0
        assert metrics["reward"] == 0.0

    for distractor in verifier.DISTRACTOR_NUMBERS:
        evidence["messages"][0]["content"] = (
            f"We had 352,345 total customers. Another relevant count was {distractor:,}."
        )
        metrics, details = verifier.score_evidence(evidence)
        assert details["mentions_expected"] is True
        assert details["mentions_distractor"] is True
        assert metrics["answer_correct"] == 0.0
        assert metrics["reward"] == 0.0

    for noise_count in (352_000, 999_999):
        evidence["messages"][0]["content"] = (
            f"We had 352,345 total customers, approximately {noise_count:,}."
        )
        metrics, details = verifier.score_evidence(evidence)
        assert details["mentions_expected"] is True
        assert details["mentions_distractor"] is False
        assert details["noise_numbers"] == [float(noise_count)]
        assert metrics["answer_correct"] == 0.0
        assert metrics["reward"] == 0.0

    evidence["messages"][0]["content"] = "352,345"
    evidence["messages"][0]["reply_to_event_id"] = "wrong-question"
    metrics, _ = verifier.score_evidence(evidence)
    assert metrics["answer_correct"] == 0.0
    assert metrics["threaded_reply"] == 0.0
    assert metrics["reward"] == 0.0


def test_memory_retrieval_answer_exists_only_in_harness_seed():
    verifier = _verifier("memory-retrieval")
    fixture = fixture_for("memory-retrieval")
    instruction = (DATASET_ROOT / "memory-retrieval" / "instruction.md").read_text(
        encoding="utf-8"
    )

    seeds = {seed.slug: seed.value for seed in fixture.memory_seeds}
    assert set(seeds) == {
        "total-customers-per-month",
        "customer-value-metric",
        "customers-metrics-spring-24",
        "new-customers-april-2024",
        "total-customers-metric",
    }
    assert "352,345" in seeds["total-customers-metric"]
    assert sum("352,345" in value for value in seeds.values()) == 1
    seeded_distractors = frozenset(
        number
        for slug, value in seeds.items()
        if slug != "total-customers-metric"
        for number in verifier._numbers(value)
        if number != 2024
    )
    assert verifier.EXPECTED_CUSTOMERS == 352_345
    assert verifier.DISTRACTOR_NUMBERS == seeded_distractors
    assert "352,345" not in instruction
    assert "352345" not in instruction.replace(",", "")
