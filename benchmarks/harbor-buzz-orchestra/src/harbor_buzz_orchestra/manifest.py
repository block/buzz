"""Validated, content-addressed experiment manifests."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Literal, Self

import yaml
from pydantic import BaseModel, ConfigDict, Field, model_validator


class ManifestError(ValueError):
    """Raised when a manifest cannot be loaded or validated."""


class StrictModel(BaseModel):
    """Base model that rejects misspelled or unrecognised manifest fields."""

    model_config = ConfigDict(extra="forbid", frozen=True)


class ArtifactRef(StrictModel):
    """Reference to immutable prompt, persona, or skill content."""

    path: str = Field(min_length=1)
    sha256: str = Field(pattern=r"^[0-9a-f]{64}$")


class GenerationConfig(StrictModel):
    """Model generation controls frozen for a condition.

    Deliberately has no ``temperature``. ``buzz-agent`` exposes no temperature
    environment variable, so a manifest value could never reach the provider —
    it was hashed into the condition identity and silently discarded. Worse, the
    old default was ``0.0`` while the measured provider default is ``1.0``, so
    every manifest asserted a determinism it did not have. Do not re-add this
    field without a corresponding ``BUZZ_AGENT_*`` variable to carry it.
    """

    # Both optional: unset means the condition does not pin them and buzz-agent's
    # own defaults apply, which is the honest default for a study of Buzz as
    # shipped. Pinning them was an optimization, and an unhelpful one — a window
    # narrower than the model's real one just buys extra compactions. Same rule as
    # the compaction fields below: an unset field emits no environment variable, so
    # a variable's presence in a trial bundle means the experiment chose it.
    max_output_tokens: int | None = Field(default=None, gt=0)
    context_window_tokens: int | None = Field(default=None, gt=0)
    # Auto-compaction ("handoff") policy, passed straight through to
    # buzz-agent's BUZZ_AGENT_HANDOFF_PERCENT / BUZZ_AGENT_HANDOFF_AT_TOKENS.
    # The agent fires at whichever binds first. Leave both unset to inherit the
    # agent's defaults (90%, ceiling 272_000).
    #
    # These are real experimental variables: compaction cadence moves both cost
    # and task performance, so they belong in the manifest and therefore in the
    # condition hash.
    compact_at_percent: int | None = Field(default=None, ge=1, le=100)
    compact_at_tokens: int | None = Field(default=None, gt=0)
    # Reasoning effort. This is the G2 axis the container-runtime constant was
    # holding closed; see `container_runtime.THINKING_EFFORT`.
    #
    # Unset does NOT mean "inherit the provider default", unlike every sibling
    # field above. It means "inherit the harness constant", which is `medium`.
    # The divergence is deliberate: effort already had a harness-level default
    # that every existing condition was run under, so falling through to the
    # provider would silently re-define all of them. Existing manifests stay
    # byte-identical, keep their hashes, and keep running at medium.
    #
    # Effort is not one knob across providers -- it reaches the two routes we
    # use in different shapes, which matters when reading a result:
    #   * Anthropic (opus): `output_config.effort`, and the thinking budget goes
    #     8_192 -> 32_768 tokens from medium to xhigh (config.rs:35).
    #   * OpenAI Responses (sol/luna): `reasoning.effort` as a string.
    # buzz-agent clamps a level the model cannot honour down to the highest it
    # can, and only logs a warning (config.rs:206) -- so a condition that asks
    # for more than the model supports runs *silently* at less. Both benchmark
    # endpoints were checked to support xhigh before this field was added.
    thinking_effort: (
        Literal["none", "minimal", "low", "medium", "high", "xhigh", "max"] | None
    ) = None
    extra: dict[str, Any] = Field(default_factory=dict)

    @model_validator(mode="after")
    def validate_output_fits_window(self) -> Self:
        # max_output_tokens is not just a response cap: buzz-agent subtracts it
        # from the window to decide when to compact
        # (handoff.rs::token_threshold). A value at or above the window drives
        # that reservation to zero, which makes the agent compact on its very
        # first turn, ten times over, and then silently fall back to
        # truncation. "Set it huge so we never think about it" is therefore the
        # one setting that breaks a run outright, and it fails as degraded
        # behaviour rather than as an error — so reject it here instead.
        #
        # Only checkable when the manifest pins both. With either unset there is
        # no manifest-declared relationship to validate: buzz-agent applies its
        # own paired defaults, which already satisfy this.
        if self.max_output_tokens is None or self.context_window_tokens is None:
            return self
        if self.max_output_tokens >= self.context_window_tokens:
            raise ValueError(
                f"max_output_tokens {self.max_output_tokens} must be less than "
                f"context_window_tokens {self.context_window_tokens}; "
                "buzz-agent reserves it from the window, so an equal or larger "
                "value forces compaction on every turn"
            )
        return self

    @model_validator(mode="after")
    def validate_compaction(self) -> Self:
        # A ceiling above the window is inert, not wrong — that is exactly the
        # agent's own default at a 200k window — so only the impossible case is
        # rejected: a target the output reservation would override anyway.
        #
        # Needs both window and output reservation to compute the headroom, so an
        # unpinned window leaves nothing to compare against — buzz-agent resolves
        # the same check at runtime against its real defaults.
        if (
            self.compact_at_tokens is not None
            and self.context_window_tokens is not None
            and self.max_output_tokens is not None
            and self.compact_at_tokens
            > self.context_window_tokens - self.max_output_tokens
        ):
            raise ValueError(
                f"compact_at_tokens {self.compact_at_tokens} exceeds the window "
                f"minus max_output_tokens "
                f"({self.context_window_tokens - self.max_output_tokens}); "
                "buzz-agent reserves room for the response, so this can never fire"
            )
        return self


class AgentBudget(StrictModel):
    """Optional per-agent live safety limits."""

    max_calls: int | None = Field(default=None, gt=0)
    max_input_tokens: int | None = Field(default=None, gt=0)
    max_output_tokens: int | None = Field(default=None, gt=0)
    max_cost_usd: float | None = Field(default=None, gt=0)


class Price(StrictModel):
    """Frozen USD rates for one endpoint revision, per million tokens.

    ``output_per_million_usd`` also covers reasoning tokens: providers bill
    thinking at the output rate, so the trial total is correct even though the
    harness cannot separate the two.
    """

    input_per_million_usd: float = Field(ge=0)
    cached_input_per_million_usd: float = Field(ge=0)
    output_per_million_usd: float = Field(ge=0)
    # Fraction of input tokens assumed to be cache reads — a **fallback**, used
    # only when the agent did not report a real cache split.
    #
    # buzz-acp now emits the provider's own cache-served count on its usage line
    # and `price_usage` prices that measured number, recording which of the two it
    # used in `accounting_cache_read_tokens_are_measured`. This field therefore
    # governs only logs from an agent build that predates that field. It stays in
    # the manifest — and so in the condition hash — because a job priced by the
    # model is not comparable to one priced by measurement unless the assumption
    # travels with it.
    #
    # Defaults to 0.0, i.e. no discount, which makes a fallback *conservative*:
    # every input token is billed at the full rate, so an unmeasured run
    # overstates cost rather than inventing a discount. Prefer leaving it at 0.0
    # and letting the measurement carry the number. Setting it non-zero is a
    # deliberate claim about an endpoint's behaviour and should be calibrated
    # from that endpoint's own reported `cached_tokens`, never guessed.
    cache_read_rate: float = Field(default=0.0, ge=0.0, le=1.0)


class AgentClass(StrictModel):
    """A homogeneous class of agents in the trial roster."""

    id: str = Field(min_length=1, pattern=r"^[a-z0-9][a-z0-9._-]*$")
    kind: Literal["orchestrator", "worker"]
    role: str = Field(min_length=1)
    count: int = Field(gt=0)
    endpoint: str = Field(min_length=1)
    model_revision: str = Field(min_length=1)
    prompt: ArtifactRef
    persona: ArtifactRef | None = None
    skills: tuple[ArtifactRef, ...] = ()
    # Whether buzz-acp prepends its compiled-in `[Base]` platform-context
    # section ahead of the pinned prompt.
    #
    # A manifest field rather than deployment config because it changes the
    # effective system prompt, which is what a condition *is*. The base section
    # is ~12KB of production-workspace guidance — startup recovery sweeps,
    # RESEARCH/PLANS conventions, git worktrees, "silence is usually correct" —
    # written for a long-lived Buzz workspace rather than a graded container.
    # Every persona overrides it in prose, but prose is not proof, so the
    # ability to turn it off is what makes that override falsifiable.
    #
    # Defaults to True: production parity is the honest default for a study
    # about Buzz, since it is what a real Buzz agent receives.
    include_platform_prompt: bool = True
    # Optional now that every field inside it is: a roster entry that says nothing
    # about generation inherits buzz-agent's defaults wholesale. An empty block and
    # an absent one are the same condition, and both hash the same way.
    generation: GenerationConfig = Field(default_factory=GenerationConfig)
    budget: AgentBudget = AgentBudget()
    concurrency: int = Field(default=1, gt=0)

    @model_validator(mode="after")
    def validate_concurrency(self) -> Self:
        if self.concurrency > self.count:
            raise ValueError("concurrency cannot exceed count")
        return self


class TrialBudget(StrictModel):
    """Trial-wide hard limits enforced by the live runtime, not async receipts."""

    timeout_seconds: int = Field(gt=0)
    max_cost_usd: float | None = Field(default=None, gt=0)


class EnvironmentOverrides(StrictModel):
    """Container resources granted to a trial, replacing the task's own request.

    Harbor enforces each task's declared ``[environment]`` block as hard Docker
    limits, and the whole Buzz stack runs *inside* that container. A three-agent
    roster therefore puts three ``buzz-acp``/``buzz-agent``/``buzz-dev-mcp``
    process groups plus the task's own work inside whatever the task asked for —
    typically 1 vCPU and 2 GB. A team that thrashes there has been penalised for
    memory pressure, not for coordinating badly, which is a confound in the one
    thing the study measures.

    These belong in the manifest rather than on the command line because they
    change what a condition *is*. A resource level that varies between cells is
    a confound; one that varies without being recorded is an invisible confound.
    Putting them here makes them part of the condition hash, so two runs at
    different limits can never pool into one number by accident.

    ``None`` means "leave the task's own request alone". Note that Harbor
    *replaces* rather than raises (``environments/base.py``), so a value below a
    task's declared request shrinks it — any override must clear the largest
    request in the dataset or some tasks quietly get less than they do today.
    """

    override_cpus: int | None = Field(default=None, gt=0)
    override_memory_mb: int | None = Field(default=None, gt=0)
    override_storage_mb: int | None = Field(default=None, gt=0)


class ExperimentManifest(StrictModel):
    """Complete immutable input defining one benchmark condition."""

    schema_version: Literal["1"] = "1"
    condition: str = Field(min_length=1)
    roster: tuple[AgentClass, ...] = Field(min_length=1)
    prices: dict[str, Price]
    trial_budget: TrialBudget
    # Absent and empty are the same condition and hash identically, matching
    # how ``generation`` behaves on AgentClass.
    environment: EnvironmentOverrides = Field(default_factory=EnvironmentOverrides)
    metadata: dict[str, Any] = Field(default_factory=dict)

    @property
    def is_solo(self) -> bool:
        """True when the roster is a single agent with no workers to delegate to."""
        return sum(entry.count for entry in self.roster) == 1

    @model_validator(mode="after")
    def validate_roster(self) -> Self:
        ids = [entry.id for entry in self.roster]
        if len(ids) != len(set(ids)):
            raise ValueError("roster class ids must be unique")
        orchestrators = sum(
            entry.count for entry in self.roster if entry.kind == "orchestrator"
        )
        # Exactly one orchestrator, but zero workers is legal: that is the
        # single-agent baseline every multi-agent result is measured against.
        if orchestrators != 1:
            raise ValueError("roster must contain exactly one orchestrator")
        endpoints = {entry.endpoint for entry in self.roster}
        missing_prices = sorted(endpoints - self.prices.keys())
        if missing_prices:
            raise ValueError(f"prices missing for endpoints: {missing_prices}")
        return self

    def canonical_bytes(self) -> bytes:
        """Return stable UTF-8 JSON independent of YAML formatting and key order."""
        data = self.model_dump(mode="json", exclude_none=False)
        # An unpinned `thinking_effort` is dropped rather than serialised as
        # null, so that opening the effort axis did not re-identify every
        # condition that does not use it.
        #
        # The hash is meant to answer "are these two runs the same experiment?".
        # A manifest written before this field existed and the same manifest
        # loaded after it send a byte-identical container environment --
        # BUZZ_AGENT_THINKING_EFFORT=medium either way, because unset resolves to
        # the harness default. They *are* the same experiment, so they must hash
        # the same, or every result measured before the axis opened would stop
        # pooling with everything measured after it for no behavioural reason.
        #
        # Kept narrow on purpose. The blanket fix, `exclude_none=True`, would
        # also drop the other optional fields that currently serialise as null
        # and would shift far more hashes than it preserved.
        for entry in data.get("roster", []):
            generation = entry.get("generation")
            if not isinstance(generation, dict):
                continue
            if generation.get("thinking_effort") is None:
                generation.pop("thinking_effort", None)
        return json.dumps(
            data,
            sort_keys=True,
            separators=(",", ":"),
            ensure_ascii=False,
            allow_nan=False,
        ).encode("utf-8")

    @property
    def sha256(self) -> str:
        """Return the condition identity derived from canonical manifest content."""
        return hashlib.sha256(self.canonical_bytes()).hexdigest()

    @classmethod
    def load(cls, source: str | Path | dict[str, Any]) -> Self:
        """Load a manifest from a YAML/JSON file or an already-decoded mapping."""
        if isinstance(source, dict):
            raw = source
        else:
            path = Path(source).expanduser()
            try:
                raw = yaml.safe_load(path.read_text(encoding="utf-8"))
            except (OSError, yaml.YAMLError) as error:
                raise ManifestError(f"cannot load manifest {path}: {error}") from error
        if not isinstance(raw, dict):
            raise ManifestError("manifest root must be a mapping")
        try:
            return cls.model_validate(raw)
        except ValueError as error:
            raise ManifestError(f"invalid manifest: {error}") from error
