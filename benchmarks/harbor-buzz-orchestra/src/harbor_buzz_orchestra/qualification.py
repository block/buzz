"""Fail-closed qualification receipts for OpenAI-compatible backends.

This module deliberately owns protocol qualification outside the product runtime.
It uses only stdlib HTTP so probes exercise the configured endpoint directly.
"""

from __future__ import annotations

import concurrent.futures
import hashlib
import json
import platform
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import UTC, datetime
from enum import StrEnum
from pathlib import Path
from typing import Any, Literal, Self

from pydantic import BaseModel, ConfigDict, Field, model_validator

SCHEMA_VERSION = "1"
MANDATORY_TIER1_PROBES = (
    "basic_generation",
    "streaming",
    "cancellation",
    "concurrency_isolation",
    "tool_call_continuation",
    "reasoning_control",
)
_SECRET_KEY = re.compile(
    r"(?:^|_)(?:api_?)?(?:key|token|secret|password|authorization|credential|cookie)(?:$|_)",
    re.IGNORECASE,
)
_GIT_SHA = r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$"
_SHA256 = r"^[0-9a-f]{64}$"
_MAX_EVIDENCE = 512


class Verdict(StrEnum):
    PASS = "PASS"
    UNSUPPORTED = "UNSUPPORTED"
    REGRESSED = "REGRESSED"
    ERROR = "ERROR"


class FailureCode(StrEnum):
    MALFORMED_JSONL = "malformed_jsonl"
    TRUNCATED_JSONL = "truncated_jsonl"
    INVALID_RECEIPT = "invalid_receipt"


class QualificationReceiptError(ValueError):
    """A fail-closed receipt ingestion error with a stable machine reason."""

    def __init__(self, code: FailureCode, detail: str) -> None:
        self.code = code
        super().__init__(f"{code.value}: {detail}")


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class SourceProvenance(StrictModel):
    buzz_commit_sha: str = Field(pattern=_GIT_SHA)
    qualifier_commit_sha: str = Field(pattern=_GIT_SHA)
    dirty_tree: bool
    python_version: str = Field(min_length=1, max_length=80)
    harbor_version: str = Field(min_length=1, max_length=80)
    lockfile_sha256: str = Field(pattern=_SHA256)
    server_implementation: str = Field(min_length=1, max_length=120)
    server_source_revision: str = Field(min_length=1, max_length=160)
    model_id: str = Field(min_length=1, max_length=200)
    model_revision: str = Field(min_length=1, max_length=200)


class HostProvenance(StrictModel):
    os: str = Field(min_length=1, max_length=80)
    os_version: str = Field(min_length=1, max_length=160)
    architecture: str = Field(min_length=1, max_length=80)
    chip_model: str = Field(min_length=1, max_length=160)
    ram_bytes: int | None = Field(default=None, gt=0)


class LaunchProvenance(StrictModel):
    base_origin: str = Field(min_length=1, max_length=500)
    provider: str = Field(min_length=1, max_length=80)
    argv_sha256: str = Field(pattern=_SHA256)
    flags: dict[str, str | int | float | bool] = Field(default_factory=dict)

    @model_validator(mode="after")
    def redact_secret_flags(self) -> LaunchProvenance:
        """Redact launch flags whose keys look like credentials (fail-closed)."""
        if any(_SECRET_KEY.search(key) for key in self.flags):
            object.__setattr__(
                self,
                "flags",
                {
                    key: ("[REDACTED]" if _SECRET_KEY.search(key) else value)
                    for key, value in self.flags.items()
                },
            )
        return self


Metric = str | int | float | bool


class ProbeResult(StrictModel):
    id: str = Field(min_length=1, pattern=r"^[a-z0-9][a-z0-9_-]*$")
    tier: Literal[1, 2]
    started_at_utc: datetime
    duration_ms: int = Field(ge=0)
    verdict: Verdict
    reason_code: str = Field(min_length=1, pattern=r"^[a-z0-9][a-z0-9_-]*$")
    metrics: dict[str, Metric] = Field(default_factory=dict)
    evidence: str = Field(default="", max_length=_MAX_EVIDENCE)

    @model_validator(mode="after")
    def require_utc(self) -> Self:
        if self.started_at_utc.tzinfo is None:
            raise ValueError("probe timestamp must include a timezone")
        return self


class ReceiptSummary(StrictModel):
    pass_count: int = Field(ge=0)
    unsupported_count: int = Field(ge=0)
    regressed_count: int = Field(ge=0)
    error_count: int = Field(ge=0)
    tier2_joined: bool
    qualifies: bool


class QualificationReceipt(StrictModel):
    schema_version: Literal["1"] = SCHEMA_VERSION
    receipt_id: str = Field(pattern=_SHA256)
    created_at_utc: datetime
    run_id: str = Field(min_length=1, max_length=200)
    manifest_sha256: str = Field(pattern=_SHA256)
    endpoint_name: str = Field(min_length=1, max_length=200)
    endpoint_config_sha256: str = Field(pattern=_SHA256)
    baseline_receipt_id: str | None = Field(default=None, pattern=_SHA256)
    source: SourceProvenance
    host: HostProvenance
    launch: LaunchProvenance
    probes: tuple[ProbeResult, ...] = Field(min_length=1)
    summary: ReceiptSummary

    def identity_fields(self) -> dict[str, Any]:
        """Stable receipt content; wall-clock timestamps and timings are excluded."""
        return {
            "schema_version": self.schema_version,
            "run_id": self.run_id,
            "manifest_sha256": self.manifest_sha256,
            "endpoint_name": self.endpoint_name,
            "endpoint_config_sha256": self.endpoint_config_sha256,
            "baseline_receipt_id": self.baseline_receipt_id,
            "source": self.source.model_dump(mode="json"),
            "host": self.host.model_dump(mode="json"),
            "launch": self.launch.model_dump(mode="json"),
            "probes": [
                {
                    "id": probe.id,
                    "tier": probe.tier,
                    "verdict": probe.verdict.value,
                    "reason_code": probe.reason_code,
                    "metrics": probe.metrics,
                    "evidence": probe.evidence,
                }
                for probe in self.probes
            ],
        }

    def join_keys(self) -> dict[str, Any]:
        """Keys required to attach this receipt to one scored run endpoint."""
        return {
            "schema_version": self.schema_version,
            "run_id": self.run_id,
            "manifest_sha256": self.manifest_sha256,
            "endpoint_name": self.endpoint_name,
            "endpoint_config_sha256": self.endpoint_config_sha256,
        }

    def comparison_join_keys(self) -> dict[str, Any]:
        """Keys that must match before a baseline may label a regression."""
        return {
            "schema_version": self.schema_version,
            "endpoint_name": self.endpoint_name,
            "endpoint_config_sha256": self.endpoint_config_sha256,
            "model_id": self.source.model_id,
            "model_revision": self.source.model_revision,
            "probe_ids": [probe.id for probe in self.probes],
        }

    @model_validator(mode="after")
    def validate_identity_and_summary(self) -> Self:
        if self.created_at_utc.tzinfo is None:
            raise ValueError("receipt timestamp must include a timezone")
        ids = [probe.id for probe in self.probes]
        if len(ids) != len(set(ids)):
            raise ValueError("probe ids must be unique")
        if (
            any(probe.verdict is Verdict.REGRESSED for probe in self.probes)
            and not self.baseline_receipt_id
        ):
            raise ValueError("REGRESSED verdict requires a comparable baseline receipt")
        expected_id = canonical_sha256(self.identity_fields())
        if self.receipt_id != expected_id:
            raise ValueError("receipt_id does not match canonical identity")
        counts = {verdict: 0 for verdict in Verdict}
        for probe in self.probes:
            counts[probe.verdict] += 1
        expected = ReceiptSummary(
            pass_count=counts[Verdict.PASS],
            unsupported_count=counts[Verdict.UNSUPPORTED],
            regressed_count=counts[Verdict.REGRESSED],
            error_count=counts[Verdict.ERROR],
            tier2_joined=self.summary.tier2_joined,
            qualifies=(
                self.summary.tier2_joined
                and all(
                    probe.verdict is Verdict.PASS
                    for probe in self.probes
                    if probe.id in MANDATORY_TIER1_PROBES
                )
                and set(MANDATORY_TIER1_PROBES).issubset(ids)
            ),
        )
        if self.summary != expected:
            raise ValueError("summary does not match probe verdicts")
        return self

    def canonical_line(self) -> bytes:
        return canonical_json(self.model_dump(mode="json")) + b"\n"


class EndpointProbeConfig(StrictModel):
    base_url: str = Field(min_length=1)
    model: str = Field(min_length=1)
    api_key: str = Field(min_length=1)
    timeout_seconds: float = Field(default=10.0, gt=0, le=300)

    @model_validator(mode="after")
    def validate_url(self) -> Self:
        parsed = urllib.parse.urlsplit(self.base_url)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise ValueError("base_url must be an absolute HTTP(S) URL")
        return self


def canonical_json(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


def canonical_sha256(value: Any) -> str:
    return hashlib.sha256(canonical_json(value)).hexdigest()


def redacted_origin(url: str) -> str:
    """Keep only scheme/host/port, dropping userinfo, path, query, fragment."""
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme not in {"http", "https"} or not parsed.hostname:
        return "[REDACTED_URL]"
    host = parsed.hostname
    if ":" in host:
        host = f"[{host}]"
    port = f":{parsed.port}" if parsed.port is not None else ""
    return f"{parsed.scheme}://{host}{port}"


def redact(value: Any, *, secrets: tuple[str, ...] = ()) -> Any:
    """Recursively sanitize receipt/log-safe data without hashing secret values."""
    known = tuple(secret for secret in secrets if secret)
    if isinstance(value, dict):
        return {
            str(key): (
                "[REDACTED]"
                if _SECRET_KEY.search(str(key))
                else redact(item, secrets=known)
            )
            for key, item in value.items()
        }
    if isinstance(value, (list, tuple)):
        return [redact(item, secrets=known) for item in value]
    if isinstance(value, str):
        # Strip URL credentials/query before replacing secrets: the marker has
        # brackets and is not legal inside URL userinfo under Python 3.13.
        if "://" in value:
            try:
                parsed = urllib.parse.urlsplit(value)
                if parsed.scheme in {"http", "https"} and parsed.hostname:
                    value = redacted_origin(value)
            except ValueError:
                pass
        result = value
        for secret in known:
            result = result.replace(secret, "[REDACTED]")
        return result[:_MAX_EVIDENCE]
    return value


def endpoint_config_sha256(config: dict[str, Any]) -> str:
    """Hash a secret-free deployment join shape, never credential values."""
    sanitized = redact(config)
    return canonical_sha256(sanitized)


def read_receipts_jsonl(path: str | Path) -> tuple[QualificationReceipt, ...]:
    """Validate every prior line; any damaged line rejects the entire ledger."""
    raw = Path(path).read_bytes()
    if raw and not raw.endswith(b"\n"):
        raise QualificationReceiptError(
            FailureCode.TRUNCATED_JSONL, "final line is not newline-terminated"
        )
    receipts: list[QualificationReceipt] = []
    for line_number, line in enumerate(raw.splitlines(), 1):
        if not line.strip():
            raise QualificationReceiptError(
                FailureCode.MALFORMED_JSONL, f"blank line {line_number}"
            )
        try:
            value = json.loads(line)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise QualificationReceiptError(
                FailureCode.MALFORMED_JSONL, f"invalid JSON on line {line_number}"
            ) from error
        try:
            receipts.append(QualificationReceipt.model_validate(value))
        except ValueError as error:
            raise QualificationReceiptError(
                FailureCode.INVALID_RECEIPT, f"schema failure on line {line_number}"
            ) from error
    return tuple(receipts)


class _ProbeFailure(Exception):
    def __init__(self, reason: str, detail: str = "") -> None:
        self.reason = reason
        self.detail = detail
        super().__init__(detail)


class OpenAIEndpointQualifier:
    """Run deterministic, fail-closed tier-1 probes against a real HTTP endpoint."""

    def __init__(self, config: EndpointProbeConfig) -> None:
        self.config = config
        self._url = f"{config.base_url.rstrip('/')}/chat/completions"

    def run(self) -> tuple[ProbeResult, ...]:
        probes = (
            ("basic_generation", self._probe_basic),
            ("streaming", self._probe_streaming),
            ("cancellation", self._probe_cancellation),
            ("concurrency_isolation", self._probe_concurrency),
            ("tool_call_continuation", self._probe_tool_call),
            ("reasoning_control", self._probe_reasoning),
        )
        return tuple(self._capture(probe_id, probe) for probe_id, probe in probes)

    def _capture(self, probe_id: str, probe: Any) -> ProbeResult:
        started = datetime.now(UTC)
        before = time.monotonic()
        try:
            reason, metrics = probe()
            verdict = Verdict.PASS
            evidence = ""
        except _ProbeFailure as error:
            reason = error.reason
            verdict = (
                Verdict.UNSUPPORTED
                if reason.startswith("unsupported_")
                else Verdict.ERROR
            )
            metrics = {}
            evidence = str(redact(error.detail, secrets=(self.config.api_key,)))
        except Exception as error:  # noqa: BLE001 - fail-closed probe boundary
            reason = "unexpected_error"
            verdict = Verdict.ERROR
            metrics = {}
            evidence = str(redact(str(error), secrets=(self.config.api_key,)))
        return ProbeResult(
            id=probe_id,
            tier=1,
            started_at_utc=started,
            duration_ms=max(0, round((time.monotonic() - before) * 1000)),
            verdict=verdict,
            reason_code=reason,
            metrics=metrics,
            evidence=evidence,
        )

    def _post(self, payload: dict[str, Any], *, stream: bool = False) -> Any:
        request = urllib.request.Request(
            self._url,
            data=canonical_json({"model": self.config.model, **payload}),
            headers={
                "Authorization": f"Bearer {self.config.api_key}",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            response = urllib.request.urlopen(
                request, timeout=self.config.timeout_seconds
            )
            if stream:
                return response
            with response:
                body = response.read()
        except urllib.error.HTTPError as error:
            error.read(256)
            if error.code == 429:
                raise _ProbeFailure("http_429", "endpoint rate limited") from error
            if 500 <= error.code <= 599:
                raise _ProbeFailure("http_5xx", f"endpoint returned {error.code}") from error
            raise _ProbeFailure("http_error", f"endpoint returned {error.code}") from error
        except TimeoutError as error:
            raise _ProbeFailure("timeout", "endpoint deadline exceeded") from error
        except (urllib.error.URLError, ConnectionError, OSError) as error:
            raise _ProbeFailure("connection_error", str(error)) from error
        try:
            return json.loads(body)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise _ProbeFailure("invalid_json", "response was not valid JSON") from error

    @staticmethod
    def _message_text(response: Any) -> str:
        try:
            content = response["choices"][0]["message"]["content"]
        except (KeyError, IndexError, TypeError) as error:
            raise _ProbeFailure("invalid_response_shape", "missing message content") from error
        if not isinstance(content, str) or not content:
            raise _ProbeFailure("empty_response", "message content was empty")
        return content

    def _nonce_request(self, nonce: str, **extra: Any) -> str:
        response = self._post(
            {
                "messages": [
                    {
                        "role": "user",
                        "content": f"Return this nonce exactly once: {nonce}",
                    }
                ],
                "temperature": 0,
                **extra,
            }
        )
        return self._message_text(response)

    def _probe_basic(self) -> tuple[str, dict[str, Metric]]:
        nonce = "REG17-BASIC-7f29"
        if nonce not in self._nonce_request(nonce):
            raise _ProbeFailure("nonce_mismatch", "basic nonce was not returned")
        return "nonce_returned", {"nonce_matches": 1}

    def _read_sse(self, response: Any) -> tuple[str, int]:
        chunks: list[str] = []
        frames = 0
        terminal = False
        try:
            while True:
                try:
                    raw = response.readline()
                except TimeoutError as error:
                    raise _ProbeFailure("timeout", "stream deadline exceeded") from error
                if not raw:
                    break
                try:
                    line = raw.decode("utf-8").rstrip("\r\n")
                except UnicodeDecodeError as error:
                    raise _ProbeFailure("invalid_sse", "non-UTF-8 SSE frame") from error
                if not line or line.startswith(":"):
                    continue
                if not line.startswith("data:"):
                    raise _ProbeFailure("invalid_sse", "SSE line lacked data prefix")
                data = line[5:].strip()
                if data == "[DONE]":
                    terminal = True
                    break
                try:
                    event = json.loads(data)
                    delta = event["choices"][0]["delta"].get("content", "")
                except (json.JSONDecodeError, KeyError, IndexError, TypeError) as error:
                    raise _ProbeFailure("invalid_sse", "malformed SSE JSON frame") from error
                if not isinstance(delta, str):
                    raise _ProbeFailure("invalid_sse", "SSE content was not text")
                chunks.append(delta)
                frames += 1
        finally:
            response.close()
        if not terminal:
            raise _ProbeFailure("invalid_sse", "stream ended without [DONE]")
        if frames < 2 or not "".join(chunks):
            raise _ProbeFailure("invalid_sse", "stream required two non-empty frames")
        return "".join(chunks), frames

    def _open_stream(self, nonce: str) -> Any:
        return self._post(
            {
                "messages": [
                    {"role": "user", "content": f"Stream this nonce: {nonce}"}
                ],
                "temperature": 0,
                "stream": True,
            },
            stream=True,
        )

    def _probe_streaming(self) -> tuple[str, dict[str, Metric]]:
        nonce = "REG17-STREAM-41ac"
        text, frames = self._read_sse(self._open_stream(nonce))
        if nonce not in text:
            raise _ProbeFailure("stream_nonce_mismatch", "stream nonce was not returned")
        return "valid_sse", {"data_frames": frames}

    def _probe_cancellation(self) -> tuple[str, dict[str, Metric]]:
        response = self._open_stream("REG17-CANCEL-c031")
        try:
            first = response.readline()
            if not first.startswith(b"data:"):
                raise _ProbeFailure("invalid_sse", "cancel stream lacked an initial frame")
        finally:
            response.close()
        nonce = "REG17-LIVE-1db7"
        try:
            text = self._nonce_request(nonce)
        except _ProbeFailure as error:
            raise _ProbeFailure(
                "cancellation_failed", f"post-cancel liveness: {error.reason}"
            ) from error
        if nonce not in text:
            raise _ProbeFailure("cancellation_failed", "post-cancel nonce mismatch")
        return "cancelled_and_live", {"liveness_passed": 1}

    def _probe_concurrency(self) -> tuple[str, dict[str, Metric]]:
        nonces = ("REG17-CONCURRENT-A-ec11", "REG17-CONCURRENT-B-52d8")
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
            futures = [executor.submit(self._nonce_request, nonce) for nonce in nonces]
            try:
                outputs = [future.result(timeout=self.config.timeout_seconds + 1) for future in futures]
            except concurrent.futures.TimeoutError as error:
                raise _ProbeFailure("timeout", "concurrent request deadline exceeded") from error
        if any(nonces[index] not in outputs[index] for index in range(2)):
            raise _ProbeFailure("crossed_concurrent_nonces", "response nonce crossed")
        if nonces[1] in outputs[0] or nonces[0] in outputs[1]:
            raise _ProbeFailure("crossed_concurrent_nonces", "response was contaminated")
        return "concurrent_nonces_isolated", {"parallel_requests": 2}

    def _probe_tool_call(self) -> tuple[str, dict[str, Metric]]:
        nonce = "REG17-TOOL-2de4"
        payload = {
            "messages": [
                {"role": "user", "content": f"Call echo_probe with nonce {nonce}"}
            ],
            "temperature": 0,
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "echo_probe",
                        "description": "Echo a qualification nonce",
                        "parameters": {
                            "type": "object",
                            "properties": {"nonce": {"type": "string"}},
                            "required": ["nonce"],
                            "additionalProperties": False,
                        },
                    },
                }
            ],
            "tool_choice": {"type": "function", "function": {"name": "echo_probe"}},
        }
        response = self._post(payload)
        try:
            call = response["choices"][0]["message"]["tool_calls"][0]
            if call["function"]["name"] != "echo_probe":
                raise KeyError("wrong tool")
            arguments = json.loads(call["function"]["arguments"])
            call_id = call["id"]
        except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
            raise _ProbeFailure("unsupported_tool_calls", "invalid tool-call shape") from error
        if arguments != {"nonce": nonce}:
            raise _ProbeFailure("tool_arguments_mismatch", "tool arguments changed nonce")
        continuation = self._post(
            {
                "messages": [
                    *payload["messages"],
                    response["choices"][0]["message"],
                    {"role": "tool", "tool_call_id": call_id, "content": nonce},
                ],
                "temperature": 0,
            }
        )
        if nonce not in self._message_text(continuation):
            raise _ProbeFailure("tool_continuation_failed", "continuation lost nonce")
        return "tool_call_continued", {"tool_calls": 1}

    def _probe_reasoning(self) -> tuple[str, dict[str, Metric]]:
        nonce = "REG17-REASON-10b9"
        try:
            text = self._nonce_request(nonce, reasoning_effort="low")
        except _ProbeFailure as error:
            if error.reason == "http_error":
                raise _ProbeFailure(
                    "unsupported_reasoning_control", "reasoning control was rejected"
                ) from error
            raise
        if nonce not in text:
            raise _ProbeFailure("reasoning_nonce_mismatch", "reasoning nonce was lost")
        return "reasoning_control_accepted", {"requested_effort": "low"}


def build_receipt(
    *,
    run_id: str,
    manifest_sha256: str,
    endpoint_name: str,
    endpoint_config_sha256: str,
    source: SourceProvenance,
    host: HostProvenance,
    launch: LaunchProvenance,
    probes: tuple[ProbeResult, ...],
    created_at_utc: datetime | None = None,
    tier2_joined: bool = False,
) -> QualificationReceipt:
    counts = {verdict: 0 for verdict in Verdict}
    for probe in probes:
        counts[probe.verdict] += 1
    ids = {probe.id for probe in probes}
    qualifies = (
        tier2_joined
        and set(MANDATORY_TIER1_PROBES).issubset(ids)
        and all(
            probe.verdict is Verdict.PASS
            for probe in probes
            if probe.id in MANDATORY_TIER1_PROBES
        )
    )
    fields = {
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "manifest_sha256": manifest_sha256,
        "endpoint_name": endpoint_name,
        "endpoint_config_sha256": endpoint_config_sha256,
        "baseline_receipt_id": None,
        "source": source.model_dump(mode="json"),
        "host": host.model_dump(mode="json"),
        "launch": launch.model_dump(mode="json"),
        "probes": [
            {
                "id": probe.id,
                "tier": probe.tier,
                "verdict": probe.verdict.value,
                "reason_code": probe.reason_code,
                "metrics": probe.metrics,
                "evidence": probe.evidence,
            }
            for probe in probes
        ],
    }
    return QualificationReceipt(
        receipt_id=canonical_sha256(fields),
        created_at_utc=created_at_utc or datetime.now(UTC),
        run_id=run_id,
        manifest_sha256=manifest_sha256,
        endpoint_name=endpoint_name,
        endpoint_config_sha256=endpoint_config_sha256,
        source=source,
        host=host,
        launch=launch,
        probes=probes,
        summary=ReceiptSummary(
            pass_count=counts[Verdict.PASS],
            unsupported_count=counts[Verdict.UNSUPPORTED],
            regressed_count=counts[Verdict.REGRESSED],
            error_count=counts[Verdict.ERROR],
            tier2_joined=tier2_joined,
            qualifies=qualifies,
        ),
    )


def local_host_provenance(*, chip_model: str = "unknown", ram_bytes: int | None = None) -> HostProvenance:
    """Capture allowlisted host facts only (never hostname/user/network identity)."""
    return HostProvenance(
        os=platform.system() or "unknown",
        os_version=platform.release() or "unknown",
        architecture=platform.machine() or "unknown",
        chip_model=chip_model or "unknown",
        ram_bytes=ram_bytes,
    )
