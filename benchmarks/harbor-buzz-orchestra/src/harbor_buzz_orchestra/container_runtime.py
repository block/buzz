"""Run the production Buzz agent stack inside the Harbor task container.

Each provisioned identity is a full ``buzz-acp`` → ``buzz-agent`` →
``buzz-dev-mcp`` process tree launched *inside* the task container — the same
binaries and the same MCP toolset (shell, file tools, the ``buzz`` CLI on
PATH) that the desktop app gives a Buzz agent. The harness stays outside:
it provisions, uploads the pinned binaries, posts the task as the trial
user, and observes the channel until the orchestrator publishes DONE.
"""

from __future__ import annotations

import asyncio
import json
import os
import shlex
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from harbor.environments.base import BaseEnvironment

from .manifest import AgentClass, ExperimentManifest
from .provisioning import AgentCredential, TrialHandle
from .runtime import RuntimeResult


DEFAULT_MAX_AGENT_ROUNDS = 32
# Container-side layout for the uploaded Buzz stack.
REMOTE_ROOT = "/opt/buzz"
REMOTE_BIN = f"{REMOTE_ROOT}/bin"
REMOTE_PROMPTS = f"{REMOTE_ROOT}/prompts"
REMOTE_LOGS = f"{REMOTE_ROOT}/logs"
# The relay is host-header tenant-bound (its community row is the authority
# of its own RELAY_URL), so agents must present that exact Host. When the
# relay actually lives outside the container, this forwarder listens on the
# canonical loopback address and bridges the byte stream to the gateway.
FORWARDER = f"{REMOTE_BIN}/relay-forwarder"
FORWARDER_LOG = f"{REMOTE_LOGS}/relay-forwarder.log"
# How many done-poll iterations between in-container liveness probes.
LIVENESS_EVERY = 10


class RuntimeLaunchError(RuntimeError):
    """Raised when a Buzz agent process cannot be launched or exits early."""


@dataclass(frozen=True, slots=True)
class EndpointLaunchConfig:
    """Deployment-specific environment needed to launch one manifest endpoint."""

    provider: str
    api_key_env: str
    env: dict[str, str] = field(default_factory=dict)


@dataclass(slots=True)
class _Agent:
    credential: AgentCredential
    pid: int
    stdout_log: str  # container path
    stderr_log: str  # container path


class BuzzContainerRuntime:
    """Launch one production Buzz agent stack per identity in the container."""

    def __init__(
        self,
        *,
        logs_dir: Path,
        artifact_root: Path,
        endpoints: dict[str, EndpointLaunchConfig],
        buzz_acp_binary: str = "buzz-acp",
        buzz_agent_binary: str = "buzz-agent",
        buzz_dev_mcp_binary: str = "buzz-dev-mcp",
        buzz_cli_binary: str = "buzz",
        relay_gateway: str = "",
        forwarder_binary: str = "relay-forwarder",
        max_agent_rounds: int = DEFAULT_MAX_AGENT_ROUNDS,
        readiness_timeout_seconds: float = 60.0,
        poll_seconds: float = 1.0,
    ) -> None:
        if max_agent_rounds <= 0:
            raise ValueError("max_agent_rounds must be positive")
        if readiness_timeout_seconds <= 0:
            raise ValueError("readiness_timeout_seconds must be positive")
        self.logs_dir = Path(logs_dir)
        self.artifact_root = Path(artifact_root)
        self.endpoints = endpoints
        # Linux builds uploaded into the task container:
        self.buzz_acp_binary = buzz_acp_binary
        self.buzz_agent_binary = buzz_agent_binary
        self.buzz_dev_mcp_binary = buzz_dev_mcp_binary
        # Host build used for user/provisioning operations only:
        self.buzz_cli_binary = buzz_cli_binary
        # Where the relay actually lives, as seen from inside the task
        # container (e.g. host.docker.internal:3600). When set, a loopback
        # forwarder bridges the agents' canonical relay address — the Host
        # the relay's community row is bound to — to this gateway.
        self.relay_gateway = relay_gateway
        self.forwarder_binary = forwarder_binary
        self.max_agent_rounds = max_agent_rounds
        self.readiness_timeout_seconds = readiness_timeout_seconds
        self.poll_seconds = poll_seconds

    async def run(
        self,
        *,
        instruction: str,
        environment: BaseEnvironment,
        manifest: ExperimentManifest,
        trial: TrialHandle,
    ) -> RuntimeResult:
        classes = self._classes_by_agent_id(manifest, trial.credentials)
        orchestrator = next(c for c in trial.credentials if c.role == "orchestrator")
        workers = [c for c in trial.credentials if c.agent_id != orchestrator.agent_id]
        if not workers:
            raise RuntimeLaunchError("Buzz orchestration requires at least one worker")
        trial_dir = self.logs_dir / "buzz"
        trial_dir.mkdir(parents=True, exist_ok=True)

        agents: list[_Agent] = []
        infra: list[_Agent] = []
        try:
            await self._install_stack(environment)
            forwarder = await self._start_forwarder(environment, trial)
            if forwarder is not None:
                infra.append(forwarder)
            await self._buzz_json(
                trial.user, trial, "users", "set-profile", "--name",
                trial.user.agent_id,
            )
            for credential in trial.credentials:
                await self._buzz_json(
                    credential, trial, "users", "set-profile", "--name",
                    credential.agent_id,
                )
                agents.append(
                    await self._launch_agent(
                        environment=environment,
                        trial=trial,
                        credential=credential,
                        agent_class=classes[credential.agent_id],
                        trial_dir=trial_dir,
                    )
                )
            await self._wait_for_agents_ready(
                environment, agents, trial.channel_id, infra
            )
            # The task arrives exactly as it would in production Buzz: a
            # user prompt @mentioning the orchestrator. The harness never
            # speaks as any agent.
            await self._send(
                trial.user, trial, f"@{orchestrator.agent_id} {instruction}"
            )
            final_message = await asyncio.wait_for(
                self._wait_for_done(environment, orchestrator, trial, agents + infra),
                timeout=manifest.trial_budget.timeout_seconds,
            )
            await self._verify_m1_output(environment, manifest)
        finally:
            await self._stop_agents(environment, agents + infra)
            await self._collect_logs(environment, trial_dir)

        return RuntimeResult(
            metadata={
                "completion_message_id": final_message["id"],
                "completion_message": final_message["content"],
                "agent_runtime": "in-container",
                "agent_hints_enabled": False,
                "task_seed": "user-identity-prompt",
                "agent_max_rounds": {
                    credential.agent_id: (
                        classes[credential.agent_id].budget.max_calls
                        or self.max_agent_rounds
                    )
                    for credential in trial.credentials
                },
            }
        )

    # -- container setup ------------------------------------------------------

    async def _install_stack(self, environment: BaseEnvironment) -> None:
        """Upload the pinned Linux binaries into the task container."""
        uploads = {
            f"{REMOTE_BIN}/buzz-acp": self.buzz_acp_binary,
            f"{REMOTE_BIN}/buzz-agent": self.buzz_agent_binary,
            f"{REMOTE_BIN}/buzz-dev-mcp": self.buzz_dev_mcp_binary,
        }
        if self.relay_gateway:
            uploads[FORWARDER] = self.forwarder_binary
        for source in uploads.values():
            if not Path(source).is_file():
                raise RuntimeLaunchError(f"agent binary not found: {source}")
        result = await environment.exec(
            f"mkdir -p {REMOTE_BIN} {REMOTE_PROMPTS} {REMOTE_LOGS}"
        )
        if result.return_code != 0:
            raise RuntimeLaunchError(
                f"cannot create {REMOTE_ROOT} in the task container: "
                f"{result.stderr or result.stdout}"
            )
        for target, source in uploads.items():
            await environment.upload_file(source, target)
        await environment.exec(f"chmod 0755 {REMOTE_BIN}/*")

    async def _start_forwarder(
        self, environment: BaseEnvironment, trial: TrialHandle
    ) -> _Agent | None:
        """Bridge the agents' canonical relay address to the real gateway.

        The relay resolves its tenant from the request ``Host`` header, so the
        agents must dial the exact authority its community row is bound to —
        ``trial.relay_ws_url``. When that address is loopback inside the task
        container but the relay lives on the Docker host, this starts the
        uploaded forwarder listening on the canonical address and pumping the
        byte stream to ``relay_gateway``. Returns ``None`` when no gateway is
        configured (the relay is reachable directly).
        """
        if not self.relay_gateway:
            return None
        # Listen on the IPv4 loopback explicitly: binding the name `localhost`
        # would pick whichever address family resolves first, while clients
        # iterate both — pinning v4 makes the pair deterministic. The Host
        # header the relay tenant-binds on comes from the URL the agents
        # dial (trial.relay_ws_url), not from the socket address.
        listen = self._ws_authority(trial.relay_ws_url).replace(
            "localhost", "127.0.0.1", 1
        )
        log = FORWARDER_LOG
        command = (
            f"{shlex.quote(FORWARDER)} {shlex.quote(listen)} "
            f"{shlex.quote(self.relay_gateway)} </dev/null "
            f">{shlex.quote(log)} 2>&1 & echo $!"
        )
        result = await environment.exec(command)
        try:
            pid = int((result.stdout or "").strip().splitlines()[-1])
        except (ValueError, IndexError) as error:
            raise RuntimeLaunchError(
                f"cannot launch relay forwarder: {result.stderr or result.stdout}"
            ) from error
        forwarder = _Agent(
            AgentCredential(
                agent_id="relay-forwarder", role="infra",
                nostr_secret_key="", nostr_pubkey="", nostr_auth_tag="",
                llm_endpoint="", llm_api_key="",
            ),
            pid, log, log,
        )
        deadline = asyncio.get_running_loop().time() + self.readiness_timeout_seconds
        while True:
            probe = await environment.exec(f"cat {shlex.quote(log)} 2>/dev/null")
            if "forwarding" in (probe.stdout or ""):
                return forwarder
            await self._raise_for_dead_agents(environment, [forwarder])
            if asyncio.get_running_loop().time() >= deadline:
                raise RuntimeLaunchError(
                    "relay forwarder did not report readiness; "
                    f"see {log} in the trial artifacts"
                )
            await asyncio.sleep(self.poll_seconds)

    @staticmethod
    def _ws_authority(relay_ws_url: str) -> str:
        """``host:port`` from a ws:// URL — the forwarder's listen address."""
        if not relay_ws_url.startswith("ws://"):
            raise RuntimeLaunchError(
                "relay_gateway forwarding requires a ws:// relay_ws_url"
            )
        authority = relay_ws_url.removeprefix("ws://").split("/", 1)[0]
        if ":" not in authority:
            authority += ":80"
        return authority

    async def _launch_agent(
        self,
        *,
        environment: BaseEnvironment,
        trial: TrialHandle,
        credential: AgentCredential,
        agent_class: AgentClass,
        trial_dir: Path,
    ) -> _Agent:
        if not credential.llm_endpoint:
            raise RuntimeLaunchError("credential llm_endpoint must not be empty")
        endpoint = self.endpoints.get(credential.llm_endpoint)
        if endpoint is None:
            raise RuntimeLaunchError(
                f"no launch config for endpoint {credential.llm_endpoint!r}"
            )
        self._reject_identity_overrides(endpoint)
        prompt_path = self.artifact_root / agent_class.prompt.path
        self._verify_artifact(prompt_path, agent_class.prompt.sha256)
        composed = self._compose_system_prompt(
            trial_dir=trial_dir,
            trial=trial,
            credential=credential,
            persona_path=prompt_path,
        )
        remote_prompt = f"{REMOTE_PROMPTS}/{credential.agent_id}.system-prompt.md"
        await environment.upload_file(composed, remote_prompt)

        stdout_log = f"{REMOTE_LOGS}/{credential.agent_id}.stdout.log"
        stderr_log = f"{REMOTE_LOGS}/{credential.agent_id}.stderr.log"
        env = self._agent_env(
            trial=trial,
            credential=credential,
            agent_class=agent_class,
            endpoint=endpoint,
            remote_prompt=remote_prompt,
        )
        command = (
            f"{shlex.quote(f'{REMOTE_BIN}/buzz-acp')} </dev/null "
            f">{shlex.quote(stdout_log)} 2>{shlex.quote(stderr_log)} & echo $!"
        )
        result = await environment.exec(command, env=env)
        try:
            pid = int((result.stdout or "").strip().splitlines()[-1])
        except (ValueError, IndexError) as error:
            raise RuntimeLaunchError(
                f"cannot launch agent {credential.agent_id}: "
                f"{result.stderr or result.stdout}"
            ) from error
        return _Agent(credential, pid, stdout_log, stderr_log)

    def _agent_env(
        self,
        *,
        trial: TrialHandle,
        credential: AgentCredential,
        agent_class: AgentClass,
        endpoint: EndpointLaunchConfig,
        remote_prompt: str,
    ) -> dict[str, str]:
        """The desktop-launch environment: real acp/agent/dev-mcp wiring."""
        return {
            **endpoint.env,
            "BUZZ_RELAY_URL": trial.relay_ws_url,
            "BUZZ_PRIVATE_KEY": credential.nostr_secret_key,
            # Desktop parity: the GUI also sets NOSTR_PRIVATE_KEY on buzz-acp
            # so buzz-dev-mcp's shim can wire git auth/signing for the agent.
            "NOSTR_PRIVATE_KEY": credential.nostr_secret_key,
            "BUZZ_AUTH_TAG": credential.nostr_auth_tag,
            "BUZZ_ACP_AGENT_COMMAND": f"{REMOTE_BIN}/buzz-agent",
            "BUZZ_ACP_AGENT_ARGS": "",
            "BUZZ_ACP_MCP_COMMAND": f"{REMOTE_BIN}/buzz-dev-mcp",
            "BUZZ_ACP_CHANNELS": trial.channel_id,
            "BUZZ_ACP_SUBSCRIBE": "mentions",
            "BUZZ_ACP_RESPOND_TO": "anyone",
            "BUZZ_ACP_NO_MEMORY": "true",
            "BUZZ_ACP_SYSTEM_PROMPT_FILE": remote_prompt,
            "BUZZ_AGENT_PROVIDER": endpoint.provider,
            "BUZZ_AGENT_MODEL": credential.llm_endpoint,
            "BUZZ_AGENT_MAX_OUTPUT_TOKENS": str(
                agent_class.generation.max_output_tokens
            ),
            "BUZZ_AGENT_MAX_CONTEXT_TOKENS": str(
                agent_class.generation.context_window_tokens
            ),
            "BUZZ_AGENT_MAX_ROUNDS": str(
                agent_class.budget.max_calls or self.max_agent_rounds
            ),
            # The pinned persona is the whole prompt: no hint-file or skill
            # discovery from the task filesystem (metadata reports this).
            "BUZZ_AGENT_NO_HINTS": "1",
            endpoint.api_key_env: credential.llm_api_key,
        }

    # -- lifecycle -------------------------------------------------------------

    async def _wait_for_agents_ready(
        self,
        environment: BaseEnvironment,
        agents: list[_Agent],
        channel_id: str,
        infra: list[_Agent] | None = None,
    ) -> None:
        """Wait until every ACP process confirms its trial-channel subscription."""
        marker = f"subscribed to channel {channel_id}"
        deadline = asyncio.get_running_loop().time() + self.readiness_timeout_seconds
        pending = {agent.credential.agent_id: agent for agent in agents}
        while pending:
            await self._raise_for_dead_agents(environment, agents + (infra or []))
            for agent_id, agent in list(pending.items()):
                result = await environment.exec(
                    f"cat {shlex.quote(agent.stdout_log)} "
                    f"{shlex.quote(agent.stderr_log)} 2>/dev/null"
                )
                if marker in (result.stdout or ""):
                    del pending[agent_id]
            if not pending:
                return
            if asyncio.get_running_loop().time() >= deadline:
                raise RuntimeLaunchError(
                    "agents did not subscribe to trial channel before readiness "
                    f"timeout: {sorted(pending)}"
                )
            await asyncio.sleep(self.poll_seconds)

    async def _wait_for_done(
        self,
        environment: BaseEnvironment,
        orchestrator: AgentCredential,
        trial: TrialHandle,
        agents: list[_Agent],
    ) -> dict[str, Any]:
        """Observe the channel as the trial user until the orchestrator posts DONE.

        Observation only: the harness never speaks as any agent. If the team
        stalls, the trial times out and the stall is the measured result.
        """
        polls = 0
        while True:
            if polls % LIVENESS_EVERY == 0:
                await self._raise_for_dead_agents(environment, agents)
            polls += 1
            messages = await self._buzz_json(
                trial.user, trial,
                "messages", "get", "--channel", trial.channel_id,
                "--limit", "100",
            )
            for message in messages:
                if message.get("pubkey") == orchestrator.nostr_pubkey and str(
                    message.get("content", "")
                ).startswith("DONE:"):
                    return message
            await asyncio.sleep(self.poll_seconds)

    async def _raise_for_dead_agents(
        self, environment: BaseEnvironment, agents: list[_Agent]
    ) -> None:
        if not agents:
            return
        probes = "; ".join(
            f"kill -0 {agent.pid} 2>/dev/null || echo DEAD:{agent.credential.agent_id}"
            for agent in agents
        )
        result = await environment.exec(probes)
        dead = [
            line.removeprefix("DEAD:")
            for line in (result.stdout or "").splitlines()
            if line.startswith("DEAD:")
        ]
        if dead:
            raise RuntimeLaunchError(
                f"agent processes exited early: {sorted(dead)}; "
                f"see {REMOTE_LOGS} in the trial artifacts"
            )

    @staticmethod
    async def _stop_agents(
        environment: BaseEnvironment, agents: list[_Agent]
    ) -> None:
        """Terminate every process of the uploaded stack (acp, agent, mcp)."""
        if not agents:
            return
        # Match by cmdline prefix via /proc: pkill/procps is not guaranteed
        # to exist in task images, the /proc filesystem is.
        sweep = (
            "for d in /proc/[0-9]*; do "
            f"grep -aq {REMOTE_BIN} \"$d/cmdline\" 2>/dev/null "
            "&& kill -TERM \"${d#/proc/}\" 2>/dev/null; done; true"
        )
        try:
            await environment.exec(sweep)
            await asyncio.sleep(2)
            await environment.exec(sweep.replace("-TERM", "-KILL"))
        except Exception:  # noqa: BLE001 — environment may already be gone
            pass

    async def _collect_logs(
        self, environment: BaseEnvironment, trial_dir: Path
    ) -> None:
        try:
            await environment.download_dir(REMOTE_LOGS, trial_dir)
        except Exception:  # noqa: BLE001 — best effort; env may be torn down
            pass

    # -- Buzz CLI as the trial user / provisioning identities -------------------

    @staticmethod
    async def _verify_m1_output(
        environment: BaseEnvironment, manifest: ExperimentManifest
    ) -> None:
        """Fail M1 immediately unless the artifact satisfies the grader contract."""
        if manifest.condition != "M1-hello-world":
            return
        result = await environment.exec(
            'python3 -c "from pathlib import Path; '
            "p = Path('/app/hello.txt'); "
            "assert p.is_file() and p.read_text().strip() == 'Hello, world!'\""
        )
        if result.return_code != 0:
            detail = (
                result.stderr or result.stdout or "grader-equivalent check failed"
            ).strip()
            raise RuntimeLaunchError(
                "M1 pre-verifier sanity probe failed: /app/hello.txt must exist "
                f"and its stripped text must equal 'Hello, world!' ({detail})"
            )

    async def _send(
        self, credential: AgentCredential, trial: TrialHandle, content: str
    ) -> None:
        await self._buzz_json(
            credential, trial,
            "messages", "send", "--channel", trial.channel_id,
            "--content", content,
        )

    async def _buzz_json(
        self, credential: AgentCredential, trial: TrialHandle, *args: str
    ) -> Any:
        process = await asyncio.create_subprocess_exec(
            self.buzz_cli_binary,
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env={
                **os.environ,
                "BUZZ_RELAY_URL": self._user_relay_url(trial),
                "BUZZ_PRIVATE_KEY": credential.nostr_secret_key,
                "BUZZ_AUTH_TAG": credential.nostr_auth_tag,
            },
        )
        stdout, stderr = await process.communicate()
        if process.returncode != 0:
            raise RuntimeLaunchError(
                f"buzz {shlex.join(args)} exited {process.returncode}: "
                f"{stderr.decode(errors='replace').strip()}"
            )
        try:
            return json.loads(stdout)
        except json.JSONDecodeError as error:
            raise RuntimeLaunchError("buzz returned invalid JSON") from error

    @staticmethod
    def _user_relay_url(trial: TrialHandle) -> str:
        """The relay as reachable from the HOST (user identity, harness).

        ``trial.relay_ws_url`` is the container view (the agents' runtime);
        ``trial.user_relay_url`` is the host view. Fall back to deriving an
        http URL from the ws URL for handles minted before v1.2.
        """
        if trial.user_relay_url:
            return trial.user_relay_url
        return BuzzContainerRuntime._cli_relay_url(trial.relay_ws_url)

    @staticmethod
    def _cli_relay_url(relay_ws_url: str) -> str:
        if relay_ws_url.startswith("ws://"):
            return f"http://{relay_ws_url.removeprefix('ws://')}"
        if relay_ws_url.startswith("wss://"):
            return f"https://{relay_ws_url.removeprefix('wss://')}"
        raise RuntimeLaunchError("trial relay_ws_url must use ws:// or wss://")

    # -- manifest plumbing -------------------------------------------------------

    @staticmethod
    def _classes_by_agent_id(
        manifest: ExperimentManifest, credentials: tuple[AgentCredential, ...]
    ) -> dict[str, AgentClass]:
        by_id = {entry.id: entry for entry in manifest.roster}
        result: dict[str, AgentClass] = {}
        for credential in credentials:
            class_id, separator, index = credential.agent_id.rpartition("-")
            match = by_id.get(class_id)
            if not separator or not index.isdigit() or match is None:
                raise RuntimeLaunchError(
                    f"credential {credential.agent_id!r} does not match a roster class"
                )
            if credential.role != match.kind:
                raise RuntimeLaunchError(
                    f"credential {credential.agent_id!r} role does not match manifest"
                )
            result[credential.agent_id] = match
        return result

    @staticmethod
    def _verify_artifact(path: Path, expected_sha256: str) -> None:
        import hashlib

        try:
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
        except OSError as error:
            raise RuntimeLaunchError(f"cannot read prompt {path}: {error}") from error
        if actual != expected_sha256:
            raise RuntimeLaunchError(
                f"prompt hash mismatch for {path}: expected {expected_sha256}, got {actual}"
            )

    def _compose_system_prompt(
        self,
        *,
        trial_dir: Path,
        trial: TrialHandle,
        credential: AgentCredential,
        persona_path: Path,
    ) -> Path:
        """Append the trial's team roster to the pinned persona.

        The analogue of a production Buzz workspace's team context: each agent
        knows its own identity, its channel, the user it reports to, and its
        teammates' names, pubkeys, and roles from its system prompt — it never
        has to discover them over the relay.
        """
        persona = persona_path.read_text(encoding="utf-8")
        lines = [
            "",
            "## Your team",
            "",
            f"You are `{credential.agent_id}` (pubkey `{credential.nostr_pubkey}`).",
            f"The team coordinates in Buzz channel `{trial.channel_id}`.",
            f"Tasks come from the user `{trial.user.agent_id}` "
            f"(pubkey `{trial.user.nostr_pubkey}`); address your final report "
            "to them.",
            "",
            "| Name | Role | Pubkey |",
            "|------|------|--------|",
        ]
        for teammate in trial.credentials:
            if teammate.agent_id == credential.agent_id:
                continue
            lines.append(
                f"| {teammate.agent_id} | {teammate.role} "
                f"| `{teammate.nostr_pubkey}` |"
            )
        composed = persona + "\n".join(lines) + "\n"
        path = trial_dir / f"{credential.agent_id}.system-prompt.md"
        path.write_text(composed, encoding="utf-8")
        path.chmod(0o600)
        return path

    @staticmethod
    def _reject_identity_overrides(endpoint: EndpointLaunchConfig) -> None:
        forbidden = {
            "BUZZ_RELAY_URL",
            "BUZZ_PRIVATE_KEY",
            "BUZZ_AUTH_TAG",
            "BUZZ_ACP_CHANNELS",
            "BUZZ_ACP_MCP_COMMAND",
            "BUZZ_ACP_AGENT_COMMAND",
        }
        overlap = forbidden & endpoint.env.keys()
        if overlap:
            raise RuntimeLaunchError(
                f"endpoint env cannot override trial identity: {sorted(overlap)}"
            )
