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
    """Model generation controls frozen for a condition."""

    temperature: float = Field(default=0.0, ge=0.0)
    max_output_tokens: int = Field(gt=0)
    context_window_tokens: int = Field(gt=0)
    extra: dict[str, Any] = Field(default_factory=dict)


class AgentBudget(StrictModel):
    """Optional per-agent live safety limits."""

    max_calls: int | None = Field(default=None, gt=0)
    max_input_tokens: int | None = Field(default=None, gt=0)
    max_output_tokens: int | None = Field(default=None, gt=0)
    max_cost_usd: float | None = Field(default=None, gt=0)


class Price(StrictModel):
    """Frozen USD rates for one endpoint revision, per million tokens."""

    input_per_million_usd: float = Field(ge=0)
    cached_input_per_million_usd: float = Field(ge=0)
    output_per_million_usd: float = Field(ge=0)


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
    generation: GenerationConfig
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


class ExperimentManifest(StrictModel):
    """Complete immutable input defining one benchmark condition."""

    schema_version: Literal["1"] = "1"
    condition: str = Field(min_length=1)
    roster: tuple[AgentClass, ...] = Field(min_length=1)
    prices: dict[str, Price]
    trial_budget: TrialBudget
    metadata: dict[str, Any] = Field(default_factory=dict)

    @model_validator(mode="after")
    def validate_roster(self) -> Self:
        ids = [entry.id for entry in self.roster]
        if len(ids) != len(set(ids)):
            raise ValueError("roster class ids must be unique")
        orchestrators = sum(
            entry.count for entry in self.roster if entry.kind == "orchestrator"
        )
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
