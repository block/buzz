from __future__ import annotations

import json
from datetime import UTC, datetime

import pytest
from pydantic import ValidationError
from qualification_http_fixture import openai_fixture

from harbor_buzz_orchestra.qualification import (
    EndpointProbeConfig,
    FailureCode,
    HostProvenance,
    LaunchProvenance,
    OpenAIEndpointQualifier,
    ProbeResult,
    QualificationReceipt,
    QualificationReceiptError,
    SourceProvenance,
    Verdict,
    build_receipt,
    canonical_sha256,
    endpoint_config_sha256,
    read_receipts_jsonl,
    redact,
    redacted_origin,
)

CANARY = "sk-reg17-CANARY-do-not-persist"
DIGEST = "a" * 64
NOW = datetime(2026, 9, 1, tzinfo=UTC)


def _source() -> SourceProvenance:
    return SourceProvenance(
        buzz_commit_sha="b" * 64,
        qualifier_commit_sha="c" * 64,
        dirty_tree=False,
        python_version="3.13.11",
        harbor_version="0.17.0",
        lockfile_sha256="d" * 64,
        server_implementation="fixture",
        server_source_revision="fixture-v1",
        model_id="fixture-model",
        model_revision="weights-v1",
    )


def _host() -> HostProvenance:
    return HostProvenance(
        os="TestOS",
        os_version="1",
        architecture="test64",
        chip_model="fixture-chip",
        ram_bytes=1024,
    )


def _launch(base_url: str = "http://127.0.0.1:1234/v1") -> LaunchProvenance:
    return LaunchProvenance(
        base_origin=redacted_origin(base_url),
        provider="openai",
        argv_sha256="e" * 64,
        flags={"threads": 2},
    )


def _pass_probe(probe_id: str) -> ProbeResult:
    return ProbeResult(
        id=probe_id,
        tier=1,
        started_at_utc=NOW,
        duration_ms=1,
        verdict=Verdict.PASS,
        reason_code="passed",
    )


def _receipt(*, probes: tuple[ProbeResult, ...] | None = None) -> QualificationReceipt:
    return build_receipt(
        run_id="run-17",
        manifest_sha256=DIGEST,
        endpoint_name="fixture/model",
        endpoint_config_sha256="f" * 64,
        source=_source(),
        host=_host(),
        launch=_launch(),
        probes=probes or (_pass_probe("basic_generation"),),
        created_at_utc=NOW,
    )


def _run(fault: str = "", *, timeout: float = 1) -> tuple[ProbeResult, ...]:
    with openai_fixture(fault=fault) as (base_url, _state):
        return OpenAIEndpointQualifier(
            EndpointProbeConfig(
                base_url=base_url,
                model="fixture-model",
                api_key=CANARY,
                timeout_seconds=timeout,
            )
        ).run()


def _by_id(probes: tuple[ProbeResult, ...], probe_id: str) -> ProbeResult:
    return next(probe for probe in probes if probe.id == probe_id)


def test_receipt_identity_is_canonical_and_validated() -> None:
    receipt = _receipt()
    loaded = QualificationReceipt.model_validate_json(receipt.canonical_line())
    assert loaded == receipt

    changed_timestamp = receipt.model_copy(
        update={"created_at_utc": datetime(2030, 1, 1, tzinfo=UTC)}
    )
    assert changed_timestamp.receipt_id == receipt.receipt_id

    raw = receipt.model_dump(mode="json")
    raw["endpoint_name"] = "other/model"
    with pytest.raises(ValidationError, match="canonical identity"):
        QualificationReceipt.model_validate(raw)


def test_source_provenance_accepts_native_git_sha1_revisions() -> None:
    source = _source().model_copy(
        update={"buzz_commit_sha": "b" * 40, "qualifier_commit_sha": "c" * 40}
    )
    assert SourceProvenance.model_validate(source.model_dump()) == source


def test_join_keys_and_probe_outcomes_change_identity() -> None:
    receipt = _receipt()
    changed = _receipt(
        probes=(
            ProbeResult(
                **{
                    **receipt.probes[0].model_dump(),
                    "verdict": Verdict.ERROR,
                    "reason_code": "timeout",
                }
            ),
        )
    )
    assert receipt.receipt_id != changed.receipt_id
    assert canonical_sha256(receipt.identity_fields()) == receipt.receipt_id


def test_summary_cannot_claim_tier1_only_receipt_qualifies() -> None:
    probe_ids = (
        "basic_generation",
        "streaming",
        "cancellation",
        "concurrency_isolation",
        "tool_call_continuation",
        "reasoning_control",
    )
    receipt = _receipt(probes=tuple(_pass_probe(probe_id) for probe_id in probe_ids))
    assert receipt.summary.pass_count == 6
    assert receipt.summary.qualifies is False

    raw = receipt.model_dump(mode="json")
    raw["summary"]["qualifies"] = True
    with pytest.raises(ValidationError, match="summary"):
        QualificationReceipt.model_validate(raw)


def test_schema_rejects_unknown_fields_and_duplicate_probes() -> None:
    raw = _receipt().model_dump(mode="json")
    raw["api_key"] = CANARY
    with pytest.raises(ValidationError, match="Extra inputs"):
        QualificationReceipt.model_validate(raw)

    with pytest.raises(ValidationError, match="probe ids must be unique"):
        _receipt(probes=(_pass_probe("streaming"), _pass_probe("streaming")))


def test_jsonl_damage_has_distinct_fail_closed_outcomes(tmp_path) -> None:
    path = tmp_path / "receipts.jsonl"
    path.write_bytes(b'{"schema_version":"1"}')
    with pytest.raises(QualificationReceiptError) as truncated:
        read_receipts_jsonl(path)
    assert truncated.value.code is FailureCode.TRUNCATED_JSONL

    path.write_bytes(b'{"schema_version":}\n')
    with pytest.raises(QualificationReceiptError) as malformed:
        read_receipts_jsonl(path)
    assert malformed.value.code is FailureCode.MALFORMED_JSONL

    path.write_bytes(b'{"schema_version":"1"}\n')
    with pytest.raises(QualificationReceiptError) as invalid:
        read_receipts_jsonl(path)
    assert invalid.value.code is FailureCode.INVALID_RECEIPT


def test_valid_jsonl_round_trips(tmp_path) -> None:
    receipt = _receipt()
    path = tmp_path / "receipts.jsonl"
    path.write_bytes(receipt.canonical_line())
    assert read_receipts_jsonl(path) == (receipt,)


def test_redaction_removes_secret_values_and_url_credentials() -> None:
    value = {
        "api_key": CANARY,
        "nested": {
            "password": "other-secret",
            "url": f"https://user:{CANARY}@example.test/v1?q={CANARY}#fragment",
            "message": f"failure contained {CANARY}",
        },
    }
    sanitized = redact(value, secrets=(CANARY,))
    encoded = json.dumps(sanitized)
    assert CANARY not in encoded
    assert "other-secret" not in encoded
    assert sanitized["nested"]["url"] == "https://example.test"
    assert redacted_origin(value["nested"]["url"]) == "https://example.test"


def test_endpoint_config_identity_never_hashes_secret_value() -> None:
    first = endpoint_config_sha256(
        {"provider": "openai", "api_key": CANARY, "base_url": "http://localhost/v1"}
    )
    second = endpoint_config_sha256(
        {"provider": "openai", "api_key": "different", "base_url": "http://localhost/v1"}
    )
    assert first == second


def test_all_tier1_probes_use_real_local_http_and_pass() -> None:
    with openai_fixture() as (base_url, state):
        qualifier = OpenAIEndpointQualifier(
            EndpointProbeConfig(base_url=base_url, model="fixture-model", api_key=CANARY)
        )
        probes = qualifier.run()

    assert [probe.id for probe in probes] == [
        "basic_generation",
        "streaming",
        "cancellation",
        "concurrency_isolation",
        "tool_call_continuation",
        "reasoning_control",
    ]
    assert all(probe.verdict is Verdict.PASS for probe in probes)
    assert len(state.requests) >= 9
    assert all(request["authorization"] == f"Bearer {CANARY}" for request in state.requests)
    assert CANARY not in json.dumps([probe.model_dump(mode="json") for probe in probes])


@pytest.mark.parametrize(
    ("fault", "timeout", "probe_id", "reason"),
    [
        ("timeout", 0.05, "basic_generation", "timeout"),
        ("429", 1, "basic_generation", "http_429"),
        ("5xx", 1, "basic_generation", "http_5xx"),
        ("invalid_sse", 1, "streaming", "invalid_sse"),
        ("cancellation", 1, "cancellation", "cancellation_failed"),
        (
            "crossed_nonces",
            1,
            "concurrency_isolation",
            "crossed_concurrent_nonces",
        ),
    ],
)
def test_transport_failures_remain_distinct(
    fault: str, timeout: float, probe_id: str, reason: str
) -> None:
    probe = _by_id(_run(fault, timeout=timeout), probe_id)
    assert probe.verdict is Verdict.ERROR
    assert probe.reason_code == reason
    assert CANARY not in probe.evidence


def test_unsupported_reasoning_control_is_not_a_pass() -> None:
    probe = _by_id(_run("reasoning_unsupported"), "reasoning_control")
    assert probe.verdict is Verdict.UNSUPPORTED
    assert probe.reason_code == "unsupported_reasoning_control"
