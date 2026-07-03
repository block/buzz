"""Harbor custom-agent entry point for Buzz orchestration."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext

from .manifest import ExperimentManifest
from .provisioning import TrialProvisioner
from .runtime import OrchestraRuntime


class BuzzOrchestraAgent(BaseAgent):
    """Coordinate an arbitrary manifest-defined team through a Buzz trial."""

    # Set True only once the runtime writes a validated agent/trajectory.json.
    SUPPORTS_ATIF = False

    def __init__(
        self,
        logs_dir: Path,
        model_name: str | None = None,
        *,
        manifest: str | Path | dict[str, Any],
        provisioner: TrialProvisioner | None = None,
        runtime: OrchestraRuntime | None = None,
        run_id: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(logs_dir=logs_dir, model_name=model_name, **kwargs)
        self.manifest = ExperimentManifest.load(manifest)
        self.provisioner = provisioner
        self.runtime = runtime
        self.run_id = run_id

    @staticmethod
    def name() -> str:
        return "buzz-orchestra"

    def version(self) -> str:
        return "0.1.0"

    async def setup(self, environment: BaseEnvironment) -> None:
        """Fail fast when the provisioner is configured but its stack is unhealthy."""
        if self.provisioner is not None:
            self.provisioner.healthcheck()

    async def run(
        self,
        instruction: str,
        environment: BaseEnvironment,
        context: AgentContext,
    ) -> None:
        if self.provisioner is None or self.runtime is None:
            raise RuntimeError(
                "BuzzOrchestraAgent requires provisioner and runtime integrations; "
                "the adapter contract is installed but M1 wiring is incomplete"
            )

        context_id = self.context_id or environment.context_id
        if context_id is None:
            raise RuntimeError("Harbor context_id is required as the trial join key")
        trial_id = str(context_id)
        run_id = self.run_id or trial_id
        handle = self.provisioner.create_trial(run_id, trial_id, self.manifest)
        if handle.trial_id != trial_id:
            raise RuntimeError("provisioner returned a handle for a different trial_id")
        if handle.manifest_hash != self.manifest.sha256:
            raise RuntimeError("provisioner returned a handle for a different manifest")
        try:
            result = await self.runtime.run(
                instruction=instruction,
                environment=environment,
                manifest=self.manifest,
                trial=handle,
            )
        finally:
            self.provisioner.teardown(handle)

        context.n_input_tokens = result.input_tokens
        context.n_cache_tokens = result.cached_input_tokens
        context.n_output_tokens = result.output_tokens
        context.cost_usd = result.cost_usd
        context.metadata = {
            **result.metadata,
            "manifest_sha256": self.manifest.sha256,
            "condition": self.manifest.condition,
            "buzz_channel_id": handle.channel_id,
            "run_id": run_id,
            "trial_id": trial_id,
        }
