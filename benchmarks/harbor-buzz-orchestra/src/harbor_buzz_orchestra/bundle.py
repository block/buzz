"""Per-trial artifact bundle: everything needed to re-read a trial later.

A benchmark sweep is expensive and unrepeatable — the models drift, the catalog
is mutable, and the container is gone the moment the trial ends. What survives
is whatever was written to disk while it ran. The point of this module is that
system prompts can be optimised *after* a sweep, which needs the exact prompt
bytes each agent saw next to what that agent actually cost.

Layout under the trial's ``logs_dir/buzz``::

    manifest.json                     the frozen condition, plus its sha256
    <agent-id>.system-prompt.md       composed prompt as the agent saw it
    <agent-id>.stdout.log / .stderr.log
    receipts.jsonl                    one priced usage row per agent
    endpoints.redacted.json           provider + env-var *names* only
    summary.json                      the index a reader starts from

**Secrets never enter the bundle.** Every agent runs with a live provider token
and a Nostr private key in its environment, so the bundle is scanned before it
is considered complete and the finding is recorded in ``summary.json``. The scan
is a backstop for the redaction, not a substitute for it.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any, Iterable

from .manifest import ExperimentManifest

if TYPE_CHECKING:  # pragma: no cover - import cycle guard
    from .accounting import TrialAccounting
    from .container_runtime import EndpointLaunchConfig
    from .provisioning import TrialHandle


BUNDLE_VERSION = "1"

# Shapes that must never reach disk. Deliberately broad: a false positive costs
# one look at a log line, a false negative publishes a live credential.
_SECRET_PATTERNS: tuple[tuple[str, re.Pattern[str]], ...] = (
    ("nostr-private-key", re.compile(r"\bnsec1[02-9ac-hj-np-z]{20,}", re.IGNORECASE)),
    ("anthropic-key", re.compile(r"\bsk-ant-[A-Za-z0-9_-]{20,}")),
    ("openai-key", re.compile(r"\bsk-[A-Za-z0-9]{32,}")),
    ("databricks-token", re.compile(r"\bdapi[0-9a-f]{32,}", re.IGNORECASE)),
    ("bearer-header", re.compile(r"[Aa]uthorization:\s*Bearer\s+\S{16,}")),
    # A bare 64-char hex string is the wire form of a Nostr secret key. It is
    # also the wire form of a Nostr *public* key and of a sha256 digest, both of
    # which the bundle is required to contain: every agent's pubkey is in its
    # own system prompt, and the relay owner's is in the first lines of every
    # agent log. So this pattern fires on every trial that ever runs.
    #
    # It is kept, because an unrecognised 64-hex string is worth a human glance,
    # but it is reported as review rather than counted against ``clean`` — a
    # gate that is red on 100% of runs is a gate nobody reads, which is strictly
    # worse than no gate at all. Definite leaks are caught by the patterns above
    # and by the exact-match pass over the trial's own secret material.
    ("hex-64", re.compile(r"(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])")),
)
# Kinds that are a prompt to look, not evidence of a leak. Excluded from
# ``clean`` but still recorded in the bundle.
_REVIEW_KINDS = frozenset({"hex-64"})
# Shortest known-secret literal worth substring-matching; see scan_for_secrets.
_MIN_SECRET_LEN = 16
# Filenames that legitimately contain 64-char hex (manifest and prompt digests).
_HEX_ALLOWED = {"manifest.json", "summary.json", "receipts.jsonl"}
# Bundles are read by humans and diffed by scripts; skip anything binary-ish.
_SCANNED_SUFFIXES = {".json", ".jsonl", ".log", ".md", ".txt", ".yaml", ".yml"}


@dataclass(frozen=True, slots=True)
class SecretFinding:
    """One suspected secret, located precisely enough to act on."""

    path: str
    line: int
    kind: str

    def to_json(self) -> dict[str, Any]:
        return {"path": self.path, "line": self.line, "kind": self.kind}


def scan_for_secrets(
    root: Path, known_secrets: Iterable[str] = ()
) -> tuple[SecretFinding, ...]:
    """Report credential-shaped strings anywhere in the bundle.

    ``known_secrets`` is the trial's own secret material — every agent's Nostr
    private key, its auth tag, and its provider API key. Matching those exactly
    is the highest-signal check available here, because a hit is not a
    credential-*shaped* string but the very credential this trial ran with. The
    shape patterns stay for everything we could not enumerate.
    """
    # Substring matching, so short values must be excluded: they collide with
    # ordinary text by accident and a scan that cries wolf gets ignored. Every
    # credential this harness actually mints is far longer — a Nostr key is 64
    # hex, an auth tag is a hex digest, a provider bearer is a JWT — so the
    # floor costs no real coverage. Anything shorter is a placeholder from a
    # test or an unset variable, not a secret.
    literals = tuple(
        sorted({secret for secret in known_secrets if len(secret) >= _MIN_SECRET_LEN})
    )
    findings: list[SecretFinding] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix.lower() not in _SCANNED_SUFFIXES:
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        relative = path.relative_to(root).as_posix()
        for number, line in enumerate(text.splitlines(), start=1):
            if any(literal in line for literal in literals):
                findings.append(SecretFinding(relative, number, "trial-credential"))
            for kind, pattern in _SECRET_PATTERNS:
                if kind == "hex-64" and path.name in _HEX_ALLOWED:
                    continue
                if pattern.search(line):
                    findings.append(SecretFinding(relative, number, kind))
    return tuple(findings)


def _trial_secrets(trial: TrialHandle) -> tuple[str, ...]:
    """Every secret string this trial handed to a container.

    The user identity is included: it owns the channel and its key is just as
    unpublishable as an agent's, even though it never runs an agent process.
    """
    return tuple(
        value
        for credential in (*trial.credentials, trial.user)
        for value in (
            credential.nostr_secret_key,
            credential.nostr_auth_tag,
            credential.llm_api_key,
        )
        if value
    )


def redact_endpoints(
    endpoints: dict[str, EndpointLaunchConfig],
) -> dict[str, Any]:
    """Record how each endpoint was configured without recording any secret.

    Keeps the provider and the *names* of the environment variables involved —
    enough to reproduce the run — and drops every value, because endpoint env
    is exactly where a host, a token, or a proxy URL would live.
    """
    return {
        name: {
            "provider": config.provider,
            "api_key_env": config.api_key_env,
            "env_keys": sorted(config.env),
        }
        for name, config in sorted(endpoints.items())
    }


def write(
    *,
    trial_dir: Path,
    manifest: ExperimentManifest,
    trial: TrialHandle,
    accounting: TrialAccounting,
    endpoints: dict[str, EndpointLaunchConfig],
) -> Path:
    """Assemble the bundle and return the path to its ``summary.json``.

    Never raises: this runs in the runtime's ``finally``, where an exception
    would replace a real trial failure with a bundling failure and lose both.
    """
    trial_dir.mkdir(parents=True, exist_ok=True)
    try:
        return _write(trial_dir, manifest, trial, accounting, endpoints)
    except Exception as error:  # noqa: BLE001 — never mask the real outcome
        fallback = trial_dir / "summary.json"
        try:
            fallback.write_text(
                json.dumps({"bundle_error": str(error)}, indent=2), encoding="utf-8"
            )
        except OSError:
            pass
        return fallback


def _write(
    trial_dir: Path,
    manifest: ExperimentManifest,
    trial: TrialHandle,
    accounting: TrialAccounting,
    endpoints: dict[str, EndpointLaunchConfig],
) -> Path:
    (trial_dir / "manifest.json").write_text(
        json.dumps(
            {
                "sha256": manifest.sha256,
                "manifest": manifest.model_dump(mode="json"),
            },
            indent=2,
            sort_keys=True,
        ),
        encoding="utf-8",
    )
    (trial_dir / "endpoints.redacted.json").write_text(
        json.dumps(redact_endpoints(endpoints), indent=2, sort_keys=True),
        encoding="utf-8",
    )
    accounting.write_receipts(trial_dir / "receipts.jsonl")

    findings = scan_for_secrets(trial_dir, _trial_secrets(trial))
    leaks = [finding for finding in findings if finding.kind not in _REVIEW_KINDS]
    review = [finding for finding in findings if finding.kind in _REVIEW_KINDS]
    summary = {
        "bundle_version": BUNDLE_VERSION,
        "condition": manifest.condition,
        "manifest_sha256": manifest.sha256,
        "solo_roster": manifest.is_solo,
        "trial_id": trial.trial_id,
        "channel_id": trial.channel_id,
        "roster": [
            {
                "agent_id": credential.agent_id,
                "role": credential.role,
                "endpoint": credential.llm_endpoint,
                "system_prompt": f"{credential.agent_id}.system-prompt.md",
            }
            for credential in trial.credentials
        ],
        "totals": {
            "input_tokens": accounting.input_tokens,
            "output_tokens": accounting.output_tokens,
            "cost_usd": accounting.cost_usd,
            "cost_usd_no_cache_discount": accounting.cost_usd_no_cache_discount,
            "cost_usd_including_handoff_bound": (
                accounting.cost_usd_including_handoff_bound
            ),
        },
        "accounting": {
            "reconciled": accounting.reconciled,
            "note": accounting.reconciliation_note,
            "warnings": list(accounting.warnings),
            "estimated_cache_read_tokens": accounting.estimated_cache_read_tokens,
            "assumes_cache_discount": accounting.assumes_cache_discount,
            "handoffs": accounting.handoffs,
            "handoff_truncations": accounting.handoff_truncations,
            "handoff_cost_usd_upper_bound": accounting.handoff_cost_usd_upper_bound,
        },
        "secret_scan": {
            # ``clean`` means no definite leak. Review items are listed
            # separately so the flag stays meaningful; see _REVIEW_KINDS.
            "clean": not leaks,
            "findings": [finding.to_json() for finding in leaks],
            "review": [finding.to_json() for finding in review],
        },
        "files": sorted(
            path.relative_to(trial_dir).as_posix()
            for path in trial_dir.rglob("*")
            if path.is_file()
        ),
    }
    path = trial_dir / "summary.json"
    path.write_text(json.dumps(summary, indent=2, sort_keys=True), encoding="utf-8")
    return path
