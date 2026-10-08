"""The container runtime must launch the production stack, unmodified."""

import asyncio
import hashlib
import json
import re
from dataclasses import replace
from pathlib import Path

import pytest
from harbor.environments.base import ExecResult

from harbor_buzz_orchestra.container_runtime import (
    REMOTE_BIN,
    REMOTE_EVIDENCE,
    REMOTE_LOGS,
    THINKING_EFFORT,
    BuzzContainerRuntime,
    EndpointLaunchConfig,
    RuntimeLaunchError,
)
from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import (
    AgentCredential,
    FixtureActor,
    TrialHandle,
)
from harbor_buzz_orchestra.task_fixtures import fixture_for


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
                {
                    "id": "orch",
                    "kind": "orchestrator",
                    "role": "lead",
                    "endpoint": "orch-model",
                    **roster_entry,
                },
                {
                    "id": "worker",
                    "kind": "worker",
                    "role": "implementer",
                    "endpoint": "worker-model",
                    **roster_entry,
                },
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
    assert rt._user_relay_url(trial_handle(())) == "http://host.docker.internal:3600"


async def test_collects_task_declared_channel_membership(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    trial = replace(
        trial_handle((credential("orch-1", "orchestrator", "orch-model"),)),
        task_name="create-channel-invite-users",
    )
    calls = []

    async def buzz_json(credential_arg, trial_arg, *args):
        calls.append((credential_arg, trial_arg, args))
        if args[:2] == ("channels", "search"):
            return [
                {
                    "channel_id": "created-channel",
                    "name": "fix-pr-1234",
                    "channel_type": "stream",
                    "visibility": "private",
                    "archived": False,
                    "ttl_seconds": 3600,
                }
            ]
        return [{"pubkey": "member", "role": "member"}]

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)

    observed = await rt._collect_observed_channels(trial)

    assert observed[0]["members"] == [{"pubkey": "member", "role": "member"}]
    assert calls[0][0].agent_id == "orch-1"
    assert calls[0][2] == (
        "channels",
        "search",
        "--query",
        "fix-pr-1234",
        "--exact",
        "--include-archived",
    )
    assert calls[1][2] == (
        "channels",
        "members",
        "--channel",
        "created-channel",
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
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://localhost:3600",
        channel_id="channel",
        credentials=(),
        user=user_credential(),
    )
    environment = Environment(
        responses={
            FORWARDER: ExecResult(stdout="99\n", stderr="", return_code=0),
            "cat ": ExecResult(
                stdout="forwarding 127.0.0.1:3600 -> host.docker.internal:3600",
                stderr="",
                return_code=0,
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


@pytest.mark.parametrize(("configured", "expected"), [(None, "0"), (7, "7")])
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


def test_memory_task_disables_auto_memory_injection(tmp_path):
    manifest = write_manifest(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = replace(trial_handle((orch,)), task_name="memory-retrieval")

    env = runtime(tmp_path)._agent_env(
        trial=trial,
        credential=orch,
        agent_class=manifest.roster[0],
        endpoint=EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        remote_prompt="/prompt.md",
    )

    assert env["BUZZ_ACP_CHANNELS"] == "channel"
    assert env["BUZZ_ACP_NO_MEMORY"] == "true"


@pytest.mark.asyncio
async def test_memory_seed_uses_agent_credentials_and_stdin(tmp_path, monkeypatch):
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = replace(trial_handle((orch,)), task_name="memory-retrieval")
    captured = []

    class Process:
        def __init__(self, invocation):
            self.invocation = invocation

        returncode = 0

        async def communicate(self, value):
            self.invocation["value"] = value
            return b"", b"wrote memory"

    async def create_subprocess_exec(*args, **kwargs):
        invocation = {"args": args, "env": kwargs["env"]}
        captured.append(invocation)
        return Process(invocation)

    monkeypatch.setattr(asyncio, "create_subprocess_exec", create_subprocess_exec)

    await runtime(tmp_path)._seed_memories(orch, trial)

    seeds = fixture_for("memory-retrieval").memory_seeds
    assert len(captured) == len(seeds)
    for invocation, seed in zip(captured, seeds, strict=True):
        assert invocation["args"][1:] == ("mem", "set", seed.slug, "-")
        assert invocation["env"]["BUZZ_PRIVATE_KEY"] == orch.nostr_secret_key
        assert invocation["value"] == seed.value.encode()


def test_runtime_validates_construction_bounds(tmp_path):
    # 0 is legal and means unbounded (BUZZ_AGENT_MAX_ROUNDS=0); the trial
    # budget is the clock. Only negatives are rejected.
    runtime(tmp_path, max_agent_rounds=0)
    with pytest.raises(ValueError, match="unbounded"):
        runtime(tmp_path, max_agent_rounds=-1)
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
                return ExecResult(stdout=logs[agent_id], stderr="", return_code=0)
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

    agents = [_Agent(credential("worker-1", "worker", "worker-model"), 7, "o", "e")]
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


async def test_send_mentions_by_pubkey_so_task_text_stays_inert(tmp_path, monkeypatch):
    """Task text is untrusted payload: `:%normal! @a` in a task statement must
    not be fed to member-name resolution (it would fail and kill the trial).
    An explicit --mention pins delivery to the orchestrator's pubkey."""
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    calls = []

    async def buzz_json(credential, trial, *args):
        calls.append(args)
        return {}

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)

    await rt._send(
        trial.user,
        trial,
        "@orch-1 run `:%normal! @a` on the file",
        mention=orch.nostr_pubkey,
    )
    assert calls[-1][-2:] == ("--mention", "pubkey-orch-1")

    # Without an explicit mention the send is unchanged (name resolution).
    await rt._send(trial.user, trial, "plain content")
    assert "--mention" not in calls[-1]
    assert calls[-1][-2:] == ("--content", "plain content")


async def test_sends_task_declared_actor_messages_and_records_event_ids(
    tmp_path, monkeypatch
):
    rt = runtime(tmp_path)
    orch = credential("solo-1", "orchestrator", "orch-model")
    reporters = tuple(
        FixtureActor(name, credential(name, "bot", ""))
        for name in ("Ledger Scout", "Risk Sentinel", "Ops Forecaster")
    )
    trial = replace(
        trial_handle((orch,)),
        task_name="interleaved-agent-reports",
        fixture_actors=reporters,
    )
    calls = []

    async def send(actor, trial_arg, content, **kwargs):
        calls.append((actor.agent_id, trial_arg, content, kwargs))
        return {"event_id": f"event-{len(calls)}"}

    monkeypatch.setattr(rt, "_send", send)

    events = await rt._send_scripted_messages(
        trial=trial, orchestrator=orch, task_event_id="task-root"
    )

    assert [event["label"] for event in events] == [
        "ledger-report",
        "risk-report",
        "operations-report",
    ]
    assert [event["event_id"] for event in events] == [
        "event-1",
        "event-2",
        "event-3",
    ]
    assert {call[0] for call in calls} == {
        "Ledger Scout",
        "Risk Sentinel",
        "Ops Forecaster",
    }
    assert all(call[3]["mention"] == orch.nostr_pubkey for call in calls)
    assert all(call[3]["reply_to"] == "task-root" for call in calls)


async def test_scripted_message_requires_an_event_id(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    orch = credential("solo-1", "orchestrator", "orch-model")
    trial = replace(
        trial_handle((orch,)),
        task_name="cross-thread-requests",
    )

    async def send(*args, **kwargs):
        return {}

    monkeypatch.setattr(rt, "_send", send)

    with pytest.raises(RuntimeLaunchError, match="did not return an event ID"):
        await rt._send_scripted_messages(
            trial=trial, orchestrator=orch, task_event_id="task-root"
        )


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


async def test_solo_turn_end_completes_without_done_message(tmp_path, monkeypatch):
    from harbor_buzz_orchestra.container_runtime import _Agent

    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    solo = _Agent(orch, 7, "stdout.log", "stderr.log")
    environment = Environment(
        responses={
            "cat ": ExecResult(
                stdout="turn complete for channel: end_turn\n",
                stderr="",
                return_code=0,
            )
        }
    )

    async def buzz_json(*args, **kwargs):
        return []

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    assert await rt._wait_for_done(environment, orch, trial, [], solo=solo) is None


async def test_scripted_events_wait_for_delivery_receipt(tmp_path, monkeypatch):
    from harbor_buzz_orchestra.container_runtime import _Agent

    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    solo = _Agent(orch, 7, "stdout.log", "stderr.log")
    alpha = {"id": "alpha", "pubkey": orch.nostr_pubkey, "content": "ALPHA"}
    beta = {"id": "beta", "pubkey": orch.nostr_pubkey, "content": "BETA"}
    scripted_event_id = "b" * 64
    message_rounds = iter([[alpha]] * 8 + [[alpha, beta]] * 2)
    turn_rounds = iter(
        [(1, 1, set())] * 8 + [(2, 1, set()), (2, 2, {scripted_event_id})]
    )
    polls = 0

    async def buzz_json(*args, **kwargs):
        nonlocal polls
        polls += 1
        return next(message_rounds)

    async def turn_status(*args, **kwargs):
        return next(turn_rounds)

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    monkeypatch.setattr(rt, "_turn_status", turn_status)

    result = await rt._wait_for_done(
        Environment(),
        orch,
        trial,
        [],
        solo=solo,
        scripted_event_ids={scripted_event_id},
    )

    assert result["id"] == "beta"
    assert polls == 10


async def test_scripted_events_do_not_stop_an_active_turn(tmp_path, monkeypatch):
    from harbor_buzz_orchestra.container_runtime import _Agent

    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    solo = _Agent(orch, 7, "stdout.log", "stderr.log")
    messages = [
        {"id": "alpha", "pubkey": orch.nostr_pubkey, "content": "ALPHA"},
        {"id": "beta", "pubkey": orch.nostr_pubkey, "content": "DONE: BETA"},
    ]
    scripted_event_id = "b" * 64
    turn_rounds = iter([(2, 1, {scripted_event_id}), (2, 2, {scripted_event_id})])
    polls = 0

    async def buzz_json(*args, **kwargs):
        nonlocal polls
        polls += 1
        return messages

    async def turn_status(*args, **kwargs):
        return next(turn_rounds)

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    monkeypatch.setattr(rt, "_turn_status", turn_status)

    result = await rt._wait_for_done(
        Environment(),
        orch,
        trial,
        [],
        solo=solo,
        scripted_event_ids={scripted_event_id},
    )

    assert result["id"] == "beta"
    assert polls == 2


def test_turn_status_parses_completed_batch_and_successful_steer_receipts():
    batch_event_id = "a" * 64
    steer_event_id = "b" * 64
    rejected_event_id = "c" * 64
    output = "\n".join(
        [
            "turn starting for channel test",
            f"turn delivered Buzz events for channel test: {batch_event_id}",
            "turn complete for channel test: end_turn",
            (
                "non-cancelling steer ack received "
                f"event_id={steer_event_id} ack=Ok(Success {{ session_id: session }})"
            ),
            (
                "non-cancelling steer ack received "
                f"event_id={rejected_event_id} ack=Ok(Err(OutcomeRejected))"
            ),
        ]
    )

    assert BuzzContainerRuntime._parse_turn_status(output) == (
        1,
        1,
        {batch_event_id, steer_event_id},
    )


async def test_collect_evidence_uploads_verifier_artifact(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    root_id = "root-event"
    reply_id = "reply-event"
    messages = [
        {
            "id": root_id,
            "kind": 9,
            "created_at": 1,
            "pubkey": trial.user.nostr_pubkey,
            "content": "question",
            "tags": [["h", trial.channel_id], ["p", orch.nostr_pubkey]],
        },
        {
            "id": reply_id,
            "kind": 9,
            "created_at": 2,
            "pubkey": orch.nostr_pubkey,
            "content": "answer",
            "tags": [["h", trial.channel_id], ["e", root_id, "", "reply"]],
        },
    ]

    async def buzz_json(*args, **kwargs):
        return messages

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    environment = Environment()
    trial_dir = tmp_path / "trial"
    trial_dir.mkdir()

    assert await rt._collect_evidence(
        environment=environment,
        trial=trial,
        trial_dir=trial_dir,
        task_event_id=root_id,
        completion_message_id=reply_id,
    )
    assert environment.uploads[-1][1] == REMOTE_EVIDENCE
    evidence = json.loads((trial_dir / "buzz-evidence.json").read_text())
    assert evidence["messages"][-1]["reply_to_event_id"] == root_id
    assert (trial_dir / "transcript.json").is_file()


async def test_failed_evidence_snapshot_records_the_reason(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))

    async def buzz_json(*args, **kwargs):
        raise RuntimeError("relay unreachable")

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    trial_dir = tmp_path / "trial"
    trial_dir.mkdir()

    assert not await rt._collect_evidence(
        environment=Environment(),
        trial=trial,
        trial_dir=trial_dir,
        task_event_id="root-event",
        completion_message_id=None,
    )
    # The caller only sees a bool, so the cause has to survive as an artifact —
    # otherwise a failed export is indistinguishable from a quiet relay.
    assert "relay unreachable" in (trial_dir / "buzz-evidence-error.txt").read_text()
    assert not (trial_dir / "buzz-evidence.json").exists()


def test_runtime_logging_keeps_readiness_and_turn_completion_signals(tmp_path):
    rt = runtime(tmp_path)
    assert rt._rust_log(None) == "buzz_acp=info,pool::prompt=info"
    assert rt._rust_log("custom=debug") == (
        "custom=debug,buzz_acp=info,pool::prompt=info"
    )


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


def test_only_evidence_grading_tasks_fail_on_a_missing_snapshot():
    # Terminal-Bench tasks share this runtime but are graded by their own
    # tests, so a snapshot hiccup must not turn a real result into an error.
    assert fixture_for("reply-to-thread").requires_evidence
    assert fixture_for("user-mention").requires_evidence
    assert fixture_for("read-named-path-outside-workspace").requires_evidence
    assert fixture_for("create-channel-invite-users").requires_evidence
    assert not fixture_for("cobol-modernization").requires_evidence
    assert not fixture_for(None).requires_evidence


@pytest.mark.parametrize(
    ("pinned", "expected"), [(None, THINKING_EFFORT), ("high", "high")]
)
async def test_thinking_effort_reaches_the_agent(tmp_path, pinned, expected):
    manifest = write_manifest(tmp_path)
    agent_class = manifest.roster[0]
    if pinned is not None:
        agent_class = agent_class.model_copy(
            update={
                "generation": agent_class.generation.model_copy(
                    update={"thinking_effort": pinned}
                )
            }
        )
    orch = credential("orch-1", "orchestrator", "orch-model")
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="4242\n", stderr="", return_code=0)}
    )
    await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=agent_class,
        trial_dir=tmp_path,
    )
    _, env = environment.commands[-1]
    assert env["BUZZ_AGENT_THINKING_EFFORT"] == expected
