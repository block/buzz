"""The container runtime must launch the production stack, unmodified."""

import hashlib
import json
import re
from pathlib import Path

import pytest
from harbor.environments.base import ExecResult

from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle
from harbor_buzz_orchestra.container_runtime import (
    REMOTE_BIN,
    REMOTE_LOGS,
    BuzzContainerRuntime,
    EndpointLaunchConfig,
    RuntimeLaunchError,
)


def write_manifest(tmp_path: Path) -> ExperimentManifest:
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    digest = hashlib.sha256(prompt.read_bytes()).hexdigest()
    roster_entry = {
        "count": 1,
        "model_revision": "r1",
        "prompt": {"path": "prompt.md", "sha256": digest},
        "generation": {"max_output_tokens": 100, "context_window_tokens": 1000},
    }
    return ExperimentManifest.load(
        {
            "condition": "test",
            "roster": [
                {"id": "orch", "kind": "orchestrator", "role": "lead",
                 "endpoint": "orch-model", **roster_entry},
                {"id": "worker", "kind": "worker", "role": "implementer",
                 "endpoint": "worker-model", **roster_entry},
            ],
            "prices": {
                name: {
                    "input_per_million_usd": 0,
                    "cached_input_per_million_usd": 0,
                    "output_per_million_usd": 0,
                }
                for name in ("orch-model", "worker-model")
            },
            "trial_budget": {"timeout_seconds": 30},
        }
    )


def credential(agent_id, role, endpoint):
    return AgentCredential(
        agent_id=agent_id,
        role=role,
        nostr_secret_key=f"secret-{agent_id}",
        nostr_pubkey=f"pubkey-{agent_id}",
        nostr_auth_tag="[]",
        llm_endpoint=endpoint,
        llm_api_key=f"key-{agent_id}",
    )


def user_credential():
    return AgentCredential(
        agent_id="user",
        role="user",
        nostr_secret_key="secret-user",
        nostr_pubkey="pubkey-user",
        nostr_auth_tag="[]",
        llm_endpoint="",
        llm_api_key="",
    )


def trial_handle(credentials, user_relay_url=""):
    return TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://host.docker.internal:3600",
        channel_id="channel",
        credentials=credentials,
        user=user_credential(),
        user_relay_url=user_relay_url,
    )


def runtime(tmp_path, **kwargs):
    return BuzzContainerRuntime(
        logs_dir=tmp_path / "logs",
        artifact_root=tmp_path,
        endpoints={
            "orch-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
            "worker-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        },
        **kwargs,
    )


class Environment:
    """Records execs/uploads; scripted stdout per command substring."""

    def __init__(self, responses=None):
        self.commands = []
        self.uploads = []
        self.responses = responses or {}

    async def exec(self, command, env=None, **kwargs):
        self.commands.append((command, env))
        for needle, result in self.responses.items():
            if needle in command:
                return result
        return ExecResult(stdout="", stderr="", return_code=0)

    async def upload_file(self, source, target):
        self.uploads.append((str(source), target))

    async def download_dir(self, source, target):
        pass


def test_maps_credentials_exactly_and_rejects_role_mismatch(tmp_path):
    manifest = write_manifest(tmp_path)
    credentials = (
        credential("orch-1", "orchestrator", "orch-model"),
        credential("worker-1", "worker", "worker-model"),
    )
    assert set(runtime(tmp_path)._classes_by_agent_id(manifest, credentials)) == {
        "orch-1",
        "worker-1",
    }
    bad = (credential("worker-1", "orchestrator", "worker-model"),)
    with pytest.raises(RuntimeLaunchError, match="role"):
        runtime(tmp_path)._classes_by_agent_id(manifest, bad)


def test_prompt_hash_and_identity_override_are_fail_closed(tmp_path):
    manifest = write_manifest(tmp_path)
    prompt_ref = manifest.roster[0].prompt
    runtime(tmp_path)._verify_artifact(tmp_path / prompt_ref.path, prompt_ref.sha256)
    (tmp_path / prompt_ref.path).write_text("changed", encoding="utf-8")
    with pytest.raises(RuntimeLaunchError, match="hash mismatch"):
        runtime(tmp_path)._verify_artifact(
            tmp_path / prompt_ref.path, prompt_ref.sha256
        )

    endpoint = EndpointLaunchConfig(
        "anthropic", "ANTHROPIC_API_KEY", {"BUZZ_ACP_MCP_COMMAND": "evil"}
    )
    with pytest.raises(RuntimeLaunchError, match="identity"):
        runtime(tmp_path)._reject_identity_overrides(endpoint)


def test_user_relay_url_prefers_host_view(tmp_path):
    rt = runtime(tmp_path)
    # v1.2 handles carry the host view for the trial user explicitly.
    assert (
        rt._user_relay_url(trial_handle((), user_relay_url="http://localhost:3600"))
        == "http://localhost:3600"
    )
    # pre-v1.2 handles fall back to deriving http from the agents' ws view.
    assert (
        rt._user_relay_url(trial_handle(()))
        == "http://host.docker.internal:3600"
    )
    with pytest.raises(RuntimeLaunchError, match="ws://"):
        rt._cli_relay_url("http://relay")


async def test_install_stack_uploads_the_pinned_stack(tmp_path):
    binaries = {}
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp"):
        path = tmp_path / name
        path.write_text("#!binary")
        binaries[name] = str(path)
    rt = runtime(
        tmp_path,
        buzz_acp_binary=binaries["buzz-acp"],
        buzz_agent_binary=binaries["buzz-agent"],
        buzz_dev_mcp_binary=binaries["buzz-dev-mcp"],
    )
    environment = Environment()
    await rt._install_stack(environment)
    assert {target for _, target in environment.uploads} == {
        f"{REMOTE_BIN}/buzz-acp",
        f"{REMOTE_BIN}/buzz-agent",
        f"{REMOTE_BIN}/buzz-dev-mcp",
    }
    assert any("chmod 0755" in cmd for cmd, _ in environment.commands)


async def test_install_stack_requires_binaries_on_disk(tmp_path):
    rt = runtime(tmp_path, buzz_acp_binary=str(tmp_path / "missing"))
    with pytest.raises(RuntimeLaunchError, match="binary not found"):
        await rt._install_stack(Environment())


async def test_forwarder_bridges_the_canonical_relay_address(tmp_path):
    from harbor_buzz_orchestra.container_runtime import FORWARDER

    forwarder = tmp_path / "relay-forwarder"
    forwarder.write_text("ELF")
    rt = runtime(
        tmp_path,
        relay_gateway="host.docker.internal:3600",
        forwarder_binary=str(forwarder),
    )
    trial = TrialHandle(
        run_id="run", trial_id="trial", manifest_hash="hash",
        relay_ws_url="ws://localhost:3600", channel_id="channel",
        credentials=(), user=user_credential(),
    )
    environment = Environment(
        responses={
            FORWARDER: ExecResult(stdout="99\n", stderr="", return_code=0),
            "cat ": ExecResult(
                stdout="forwarding 127.0.0.1:3600 -> host.docker.internal:3600",
                stderr="", return_code=0,
            ),
        }
    )
    agent = await rt._start_forwarder(environment, trial)
    assert agent is not None and agent.pid == 99
    launch = next(cmd for cmd, _ in environment.commands if FORWARDER in cmd)
    # Listens on the canonical loopback (host-header bound), targets the gateway.
    assert "127.0.0.1:3600" in launch
    assert "host.docker.internal:3600" in launch

    # No gateway configured: the relay is reachable directly, no forwarder.
    assert await runtime(tmp_path)._start_forwarder(Environment(), trial) is None
    with pytest.raises(RuntimeLaunchError, match="ws://"):
        rt._ws_authority("http://relay")


@pytest.mark.parametrize(("configured", "expected"), [(None, "32"), (7, "7")])
async def test_launch_wires_the_desktop_environment(tmp_path, configured, expected):
    manifest = write_manifest(tmp_path)
    agent_class = manifest.roster[0]
    if configured is not None:
        agent_class = agent_class.model_copy(
            update={
                "budget": agent_class.budget.model_copy(
                    update={"max_calls": configured}
                )
            }
        )
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="4242\n", stderr="", return_code=0)}
    )
    agent = await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial,
        credential=orch,
        agent_class=agent_class,
        trial_dir=tmp_path,
    )
    assert agent.pid == 4242
    command, env = environment.commands[-1]
    assert f"{REMOTE_BIN}/buzz-acp" in command
    # The real product wiring: acp spawns buzz-agent, which gets buzz-dev-mcp.
    assert env["BUZZ_ACP_AGENT_COMMAND"] == f"{REMOTE_BIN}/buzz-agent"
    assert env["BUZZ_ACP_MCP_COMMAND"] == f"{REMOTE_BIN}/buzz-dev-mcp"
    assert env["BUZZ_RELAY_URL"] == trial.relay_ws_url
    assert env["BUZZ_PRIVATE_KEY"] == orch.nostr_secret_key
    assert env["NOSTR_PRIVATE_KEY"] == orch.nostr_secret_key
    assert env["BUZZ_AGENT_NO_HINTS"] == "1"
    assert env["BUZZ_AGENT_MAX_ROUNDS"] == expected
    assert env["BUZZ_ACP_SYSTEM_PROMPT_FILE"].endswith("orch-1.system-prompt.md")
    # The composed prompt was uploaded into the container.
    assert any(
        target == env["BUZZ_ACP_SYSTEM_PROMPT_FILE"]
        for _, target in environment.uploads
    )


def test_runtime_rejects_unbounded_agent_rounds(tmp_path):
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, max_agent_rounds=0)
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, readiness_timeout_seconds=0)


async def test_wait_for_agents_ready_requires_every_channel_subscription(tmp_path):
    rt = runtime(tmp_path, poll_seconds=0)
    logs = {"orch-1": "", "worker-1": ""}

    class ReadyEnvironment(Environment):
        polls = 0

        async def exec(self, command, env=None, **kwargs):
            if command.startswith("cat "):
                agent_id = re.search(r"([\w-]+)\.stdout\.log", command).group(1)
                return ExecResult(
                    stdout=logs[agent_id], stderr="", return_code=0
                )
            return ExecResult(stdout="", stderr="", return_code=0)

    from harbor_buzz_orchestra.container_runtime import _Agent

    agents = [
        _Agent(
            credential(agent_id, "worker", "worker-model"),
            pid=1,
            stdout_log=f"{REMOTE_LOGS}/{agent_id}.stdout.log",
            stderr_log=f"{REMOTE_LOGS}/{agent_id}.stderr.log",
        )
        for agent_id in logs
    ]
    logs["orch-1"] = "subscribed to channel trial-channel\n"
    logs["worker-1"] = "subscribed to channel trial-channel\n"
    await rt._wait_for_agents_ready(ReadyEnvironment(), agents, "trial-channel")

    logs["worker-1"] = ""
    rt_timeout = runtime(tmp_path, poll_seconds=0, readiness_timeout_seconds=0.01)
    with pytest.raises(RuntimeLaunchError, match="worker-1"):
        await rt_timeout._wait_for_agents_ready(
            ReadyEnvironment(), agents, "trial-channel"
        )


async def test_dead_agent_processes_fail_the_trial(tmp_path):
    from harbor_buzz_orchestra.container_runtime import _Agent

    agents = [
        _Agent(credential("worker-1", "worker", "worker-model"), 7, "o", "e")
    ]
    environment = Environment(
        responses={
            "kill -0": ExecResult(stdout="DEAD:worker-1\n", stderr="", return_code=0)
        }
    )
    with pytest.raises(RuntimeLaunchError, match="worker-1"):
        await runtime(tmp_path)._raise_for_dead_agents(environment, agents)


@pytest.mark.parametrize(
    ("condition", "return_code", "raises"),
    [
        ("M1-hello-world", 0, False),
        ("M1-hello-world", 1, True),
        ("other", 1, False),
    ],
)
async def test_m1_output_probe_matches_grader_and_is_condition_scoped(
    tmp_path, condition, return_code, raises
):
    manifest = write_manifest(tmp_path).model_copy(update={"condition": condition})
    environment = Environment(
        responses={
            "hello.txt": ExecResult(stdout="", stderr="", return_code=return_code)
        }
    )
    if raises:
        with pytest.raises(RuntimeLaunchError, match="/app/hello.txt"):
            await runtime(tmp_path)._verify_m1_output(environment, manifest)
    else:
        await runtime(tmp_path)._verify_m1_output(environment, manifest)
    probed = [cmd for cmd, _ in environment.commands if "hello.txt" in cmd]
    assert bool(probed) == (condition == "M1-hello-world")


async def test_wait_for_done_requires_orchestrator_authorship(tmp_path, monkeypatch):
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    rounds = iter(
        [
            [{"id": "1", "pubkey": "someone-else", "content": "DONE: fake"}],
            [{"id": "2", "pubkey": orch.nostr_pubkey, "content": "DONE: real"}],
        ]
    )
    observers = []

    async def buzz_json(credential, *args, **kwargs):
        observers.append(credential.agent_id)
        return next(rounds)

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    result = await rt._wait_for_done(Environment(), orch, trial, [])
    assert json.dumps(result).find("real") > 0
    # observation happens as the trial user, never as an agent identity
    assert set(observers) == {"user"}


def test_composed_system_prompt_carries_persona_and_team_roster(tmp_path):
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    worker_1 = credential("worker-1", "worker", "worker-model")
    worker_2 = credential("worker-2", "worker", "worker-model")
    trial = trial_handle((orch, worker_1, worker_2))
    persona = tmp_path / "persona.md"
    persona.write_text("# Persona body\n", encoding="utf-8")

    path = rt._compose_system_prompt(
        trial_dir=tmp_path,
        trial=trial,
        credential=orch,
        persona_path=persona,
    )

    composed = path.read_text(encoding="utf-8")
    assert composed.startswith("# Persona body\n")
    assert "You are `orch-1` (pubkey `pubkey-orch-1`)" in composed
    assert f"channel `{trial.channel_id}`" in composed
    assert "user `user` (pubkey `pubkey-user`)" in composed
    # roster lists teammates, never the agent itself
    assert "| worker-1 | worker | `pubkey-worker-1` |" in composed
    assert "| worker-2 | worker | `pubkey-worker-2` |" in composed
    assert "| orch-1 " not in composed
    assert path.stat().st_mode & 0o777 == 0o600


async def test_stop_agents_sweeps_the_uploaded_stack(tmp_path):
    from harbor_buzz_orchestra.container_runtime import _Agent

    environment = Environment()
    agents = [_Agent(credential("orch-1", "orchestrator", "orch-model"), 1, "o", "e")]
    await BuzzContainerRuntime._stop_agents(environment, agents)
    sweeps = [cmd for cmd, _ in environment.commands if REMOTE_BIN in cmd]
    assert len(sweeps) == 2
    assert "kill -TERM" in sweeps[0] and "kill -KILL" in sweeps[1]
