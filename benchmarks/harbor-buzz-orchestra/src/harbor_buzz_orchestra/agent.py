"""Harbor custom-agent entry point for Buzz orchestration."""

from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from harbor.agents.base import BaseAgent
from harbor.environments.base import BaseEnvironment
from harbor.models.agent.context import AgentContext
from harbor.models.trajectories import Agent, FinalMetrics, Step, Trajectory
from harbor.utils.trajectory_utils import format_trajectory_json

from .container_runtime import BuzzContainerRuntime, EndpointLaunchConfig
from .manifest import ExperimentManifest
from .provisioning import TrialProvisioner
from .runtime import OrchestraRuntime


class BuzzOrchestraAgent(BaseAgent):
    """Coordinate an arbitrary manifest-defined team through a Buzz trial."""

    SUPPORTS_ATIF = True

    def __init__(
        self,
        logs_dir: Path,
        model_name: str | None = None,
        *,
        manifest: str | Path | dict[str, Any],
        provisioner: TrialProvisioner | None = None,
        runtime: OrchestraRuntime | None = None,
        provisioner_factory: str | None = None,
        provisioner_config: str | Path | dict[str, Any] | None = None,
        artifact_root: str | Path | None = None,
        endpoint_config: str | Path | dict[str, Any] | None = None,
        buzz_acp_binary: str = "buzz-acp",
        buzz_agent_binary: str = "buzz-agent",
        buzz_dev_mcp_binary: str = "buzz-dev-mcp",
        goose_binary: str = "",
        codex_acp_binary: str = "",
        codex_binary: str = "",
        codex_code_mode_host_binary: str = "",
        codex_runtime_lib_dir: str = "",
        buzz_cli_binary: str = "buzz",
        relay_gateway: str = "",
        forwarder_binary: str = "relay-forwarder",
        run_id: str | None = None,
        **kwargs: Any,
    ) -> None:
        super().__init__(logs_dir=logs_dir, model_name=model_name, **kwargs)
        self.manifest = ExperimentManifest.load(manifest)
        self.provisioner = provisioner or self._build_provisioner(
            provisioner_factory, provisioner_config
        )
        self.runtime = runtime or self._build_runtime(
            logs_dir,
            artifact_root,
            endpoint_config,
            buzz_acp_binary,
            buzz_agent_binary,
            buzz_dev_mcp_binary,
            goose_binary,
            codex_acp_binary,
            codex_binary,
            codex_code_mode_host_binary,
            codex_runtime_lib_dir,
            buzz_cli_binary,
            relay_gateway,
            forwarder_binary,
        )
        self.run_id = run_id

    @staticmethod
    def name() -> str:
        return "buzz-orchestra"

    def version(self) -> str:
        return "0.1.0"

    def _write_trajectory(
        self,
        *,
        instruction: str,
        trial_id: str,
        result: Any,
    ) -> None:
        """Write the ATIF artifact required by Harbor leaderboard submissions.

        Buzz is a composite runtime, so its canonical cross-agent record is the
        relay transcript. Detailed ACP streaming and tool events remain beside
        it in ``buzz/<agent>.stdout.log``; the ATIF file captures the complete
        user/agent message history without inventing tool outputs that Buzz does
        not currently persist in a structured form.
        """
        transcript_path = self.logs_dir / "buzz" / "transcript.json"
        messages: list[dict[str, Any]] = []
        if transcript_path.is_file():
            payload = json.loads(transcript_path.read_text(encoding="utf-8"))
            raw_messages = payload.get("messages", [])
            if isinstance(raw_messages, list):
                messages = [
                    message for message in raw_messages if isinstance(message, dict)
                ]

        steps: list[Step] = []
        for message in messages:
            content = message.get("content")
            if not isinstance(content, str):
                continue
            author = str(message.get("author") or "unknown")
            created_at = message.get("created_at")
            timestamp = None
            if isinstance(created_at, (int, float)):
                timestamp = (
                    datetime.fromtimestamp(created_at, UTC)
                    .isoformat()
                    .replace("+00:00", "Z")
                )
            steps.append(
                Step(
                    step_id=len(steps) + 1,
                    timestamp=timestamp,
                    source="user" if author == "user" else "agent",
                    message=content,
                    extra={"buzz_author": author},
                )
            )

        if not steps:
            steps.append(Step(step_id=1, source="user", message=instruction))
            completion = result.metadata.get("completion_message")
            if isinstance(completion, str) and completion:
                steps.append(Step(step_id=2, source="agent", message=completion))

        model_revisions = sorted(
            {entry.model_revision or entry.endpoint for entry in self.manifest.roster}
        )
        trajectory = Trajectory(
            schema_version="ATIF-v1.7",
            session_id=trial_id,
            agent=Agent(
                name=self.name(),
                version=self.version(),
                model_name=model_revisions[0] if len(model_revisions) == 1 else None,
                extra={
                    "condition": self.manifest.condition,
                    "model_revisions": model_revisions,
                },
            ),
            steps=steps,
            notes=(
                "Composite Buzz relay transcript. Detailed ACP streaming and tool "
                "events are preserved in buzz/<agent>.stdout.log."
            ),
            final_metrics=FinalMetrics(
                total_prompt_tokens=result.input_tokens,
                total_cached_tokens=result.cached_input_tokens,
                total_completion_tokens=result.output_tokens,
                total_cost_usd=result.cost_usd,
                total_steps=len(steps),
            ),
        )
        (self.logs_dir / "trajectory.json").write_text(
            format_trajectory_json(trajectory.to_json_dict()) + "\n",
            encoding="utf-8",
        )

    @staticmethod
    def _load_mapping(
        source: str | Path | dict[str, Any] | None,
    ) -> dict[str, Any] | None:
        if source is None:
            return None
        if isinstance(source, dict):
            return source
        import json

        path = Path(source).expanduser()
        try:
            value = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise ValueError(f"cannot load JSON config {path}: {error}") from error
        if not isinstance(value, dict):
            raise TypeError(f"JSON config {path} must contain an object")
        return value

    @classmethod
    def _build_provisioner(
        cls,
        factory_path: str | None,
        config_source: str | Path | dict[str, Any] | None,
    ) -> TrialProvisioner | None:
        config = cls._load_mapping(config_source)
        if factory_path is None and config is None:
            return None
        if factory_path is None or config is None:
            raise ValueError(
                "provisioner_factory and provisioner_config must be provided together"
            )
        from harbor.utils.import_path import import_symbol

        factory = import_symbol(factory_path)
        return factory(config)

    @classmethod
    def _build_runtime(
        cls,
        logs_dir: Path,
        artifact_root: str | Path | None,
        endpoint_source: str | Path | dict[str, Any] | None,
        buzz_acp_binary: str,
        buzz_agent_binary: str,
        buzz_dev_mcp_binary: str,
        goose_binary: str,
        codex_acp_binary: str,
        codex_binary: str,
        codex_code_mode_host_binary: str,
        codex_runtime_lib_dir: str,
        buzz_cli_binary: str,
        relay_gateway: str,
        forwarder_binary: str,
    ) -> OrchestraRuntime | None:
        endpoint_data = cls._load_mapping(endpoint_source)
        if endpoint_data is None and artifact_root is None:
            return None
        if endpoint_data is None or artifact_root is None:
            raise ValueError(
                "artifact_root and endpoint_config must be provided together"
            )
        endpoints = {
            name: EndpointLaunchConfig(
                provider=value["provider"],
                api_key_env=value["api_key_env"],
                env=value.get("env", {}),
            )
            for name, value in endpoint_data.items()
        }
        return BuzzContainerRuntime(
            logs_dir=logs_dir,
            artifact_root=Path(artifact_root),
            endpoints=endpoints,
            buzz_acp_binary=buzz_acp_binary,
            buzz_agent_binary=buzz_agent_binary,
            buzz_dev_mcp_binary=buzz_dev_mcp_binary,
            goose_binary=goose_binary,
            codex_acp_binary=codex_acp_binary,
            codex_binary=codex_binary,
            codex_code_mode_host_binary=codex_code_mode_host_binary,
            codex_runtime_lib_dir=codex_runtime_lib_dir,
            buzz_cli_binary=buzz_cli_binary,
            relay_gateway=relay_gateway,
            forwarder_binary=forwarder_binary,
        )

    async def setup(self, environment: BaseEnvironment) -> None:
        """Fail fast when the provisioner is configured but its stack is unhealthy."""
        if self.provisioner is not None:
            self.provisioner.healthcheck()
        # Harbor excludes setup from agent_execution timing. Stage the large
        # third-party runtimes here so the experiment measures their work, not
        # a 100-300 MB upload. run() checks again for callers that skip setup.
        if isinstance(self.runtime, BuzzContainerRuntime):
            await self.runtime.prepare_external_harnesses(environment, self.manifest)

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
        # Human-readable channel label: the task short name, so a spectator
        # GUI shows one recognisable channel per problem per attempt.
        channel_label = getattr(environment, "environment_name", None)
        handle = self.provisioner.create_trial(
            run_id,
            trial_id,
            self.manifest,
            channel_label=channel_label,
            task_name=channel_label,
        )
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

        self._write_trajectory(
            instruction=instruction,
            trial_id=trial_id,
            result=result,
        )

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
