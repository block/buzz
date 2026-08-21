"""Runtime contract kept separate from Buzz resource provisioning."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol

from harbor.environments.base import BaseEnvironment

from .manifest import ExperimentManifest
from .provisioning import TrialHandle


@dataclass(frozen=True, slots=True)
class RuntimeResult:
    """Aggregate values returned by an orchestration runtime."""

    input_tokens: int = 0
    cached_input_tokens: int = 0
    output_tokens: int = 0
    cost_usd: float = 0.0
    metadata: dict[str, Any] = field(default_factory=dict)


class OrchestraRuntime(Protocol):
    """Runs the provisioned roster and writes trial artifacts into logs_dir."""

    async def run(
        self,
        *,
        instruction: str,
        environment: BaseEnvironment,
        manifest: ExperimentManifest,
        trial: TrialHandle,
    ) -> RuntimeResult: ...
