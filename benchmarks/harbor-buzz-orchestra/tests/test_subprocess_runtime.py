import hashlib
import json
from datetime import datetime
from pathlib import Path
from types import SimpleNamespace

import pytest
from harbor.environments.base import ExecResult

from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle
from harbor_buzz_orchestra.subprocess_runtime import (
    BuzzSubprocessRuntime,
    EndpointLaunchConfig,
    RuntimeLaunchError,
)


def write_manifest(tmp_path: Path) -> ExperimentManifest:
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    digest = hashlib.sha256(prompt.read_bytes()).hexdigest()
    return ExperimentManifest.load(
        {
            "condition": "test",
            "roster": [
                {
                    "id": "orch",
                    "kind": "orchestrator",
                    "role": "lead",
                    "count": 1,
                    "endpoint": "orch-model",
                    "model_revision": "r1",
                    "prompt": {"path": "prompt.md", "sha256": digest},
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                },
                {
                    "id": "worker",
                    "kind": "worker",
                    "role": "implementer",
                    "count": 1,
                    "endpoint": "worker-model",
                    "model_revision": "r1",
                    "prompt": {"path": "prompt.md", "sha256": digest},
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                },
            ],
            "prices": {
                "orch-model": {
                    "input_per_million_usd": 0,
                    "cached_input_per_million_usd": 0,
                    "output_per_million_usd": 0,
                },
                "worker-model": {
                    "input_per_million_usd": 0,
                    "cached_input_per_million_usd": 0,
                    "output_per_million_usd": 0,
                },
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


def runtime(tmp_path, **kwargs):
    return BuzzSubprocessRuntime(
        logs_dir=tmp_path / "logs",
        artifact_root=tmp_path,
        endpoints={
            "orch-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
            "worker-model": EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        },
        **kwargs,
    )


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
        "anthropic", "ANTHROPIC_API_KEY", {"BUZZ_PRIVATE_KEY": "bad"}
    )
    with pytest.raises(RuntimeLaunchError, match="identity"):
        runtime(tmp_path)._reject_identity_overrides(endpoint)


def test_relay_url_conversion_is_explicit(tmp_path):
    rt = runtime(tmp_path)
    assert rt._cli_relay_url("ws://relay:3000") == "http://relay:3000"
    assert rt._cli_relay_url("wss://relay") == "https://relay"
    with pytest.raises(RuntimeLaunchError, match="ws://"):
        rt._cli_relay_url("http://relay")


def test_mcp_wrapper_pins_agent_buzz_and_optional_socket(tmp_path):
    rt = runtime(tmp_path, buzz_cli_binary="/pinned/buzz")
    worker = rt._write_mcp_wrapper(
        trial_dir=tmp_path,
        agent_id="worker-1",
        socket_path=tmp_path / "broker.sock",
    )
    worker_content = worker.read_text()
    assert "worker-1" in worker_content
    assert "/pinned/buzz" in worker_content
    assert str(tmp_path / "broker.sock") in worker_content
    assert worker.stat().st_mode & 0o777 == 0o700

    orchestrator = rt._write_mcp_wrapper(
        trial_dir=tmp_path, agent_id="orch-1", socket_path=None
    )
    assert "socket_path=" not in orchestrator.read_text()


@pytest.mark.asyncio
@pytest.mark.parametrize(("configured", "expected"), [(None, "32"), (7, "7")])
async def test_launch_sets_bounded_agent_rounds(
    tmp_path, monkeypatch, configured, expected
):
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
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id="channel",
        credentials=(orch,),
    )
    captured = {}

    class Process:
        returncode = None

    async def create_subprocess_exec(*args, **kwargs):
        captured.update(kwargs["env"])
        return Process()

    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.create_subprocess_exec",
        create_subprocess_exec,
    )
    launched = await runtime(tmp_path)._launch_agent(
        trial=trial,
        credential=orch,
        agent_class=agent_class,
        socket_path=tmp_path / "broker.sock",
        trial_dir=tmp_path,
    )
    launched.stdout_stream.close()
    launched.stderr_stream.close()

    assert captured["BUZZ_AGENT_NO_HINTS"] == "1"
    assert captured["BUZZ_AGENT_MAX_ROUNDS"] == expected
    assert captured["BUZZ_ACP_MCP_COMMAND"].endswith("agent-mcp-orch-1")
    wrapper = Path(captured["BUZZ_ACP_MCP_COMMAND"])
    assert "/pinned/buzz" not in wrapper.read_text()  # default runtime binary is `buzz`


def test_runtime_rejects_unbounded_agent_rounds(tmp_path):
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, max_agent_rounds=0)
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, readiness_timeout_seconds=0)
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, worker_report_timeout_seconds=0)


@pytest.mark.asyncio
async def test_wait_for_agents_ready_requires_every_channel_subscription(
    tmp_path, monkeypatch
):
    rt = runtime(tmp_path, poll_seconds=0)
    trial_channel = "trial-channel"
    processes = []
    for agent_id in ("orch-1", "worker-1"):
        stdout_path = tmp_path / f"{agent_id}.stdout"
        stdout_path.write_text("")
        processes.append(
            SimpleNamespace(
                credential=credential(agent_id, "worker", "worker-model"),
                process=SimpleNamespace(returncode=None),
                stdout_path=stdout_path,
                stderr_path=tmp_path / f"{agent_id}.stderr",
            )
        )
    sleeps = 0

    async def make_ready(_):
        nonlocal sleeps
        sleeps += 1
        target = processes[sleeps - 1].stdout_path
        target.write_text(f"subscribed to channel {trial_channel}\n")

    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.sleep", make_ready
    )
    await rt._wait_for_agents_ready(processes, trial_channel)
    assert sleeps == 2


@pytest.mark.asyncio
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

    class Environment:
        commands = []

        async def exec(self, command, **kwargs):
            self.commands.append(command)
            return ExecResult(stdout="", stderr="", return_code=return_code)

    environment = Environment()
    if raises:
        with pytest.raises(RuntimeLaunchError, match="/app/hello.txt"):
            await runtime(tmp_path)._verify_m1_output(environment, manifest)
    else:
        await runtime(tmp_path)._verify_m1_output(environment, manifest)

    assert bool(environment.commands) == (condition == "M1-hello-world")
    if environment.commands:
        assert "p.read_text().strip() == 'Hello, world!'" in environment.commands[0]


@pytest.mark.asyncio
async def test_wait_for_done_requires_orchestrator_authorship(tmp_path, monkeypatch):
    rt = runtime(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id="channel",
        credentials=(orch,),
    )
    rounds = iter(
        [
            [{"id": "1", "pubkey": "someone-else", "content": "DONE: fake"}],
            [{"id": "2", "pubkey": orch.nostr_pubkey, "content": "DONE: real"}],
        ]
    )

    async def buzz_json(*args, **kwargs):
        return next(rounds)

    async def no_sleep(_):
        return None

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.sleep", no_sleep
    )
    result, recoveries = await rt._wait_for_done(orch, [], trial, [])
    assert json.dumps(result).find("real") > 0
    assert recoveries == []


@pytest.mark.asyncio
async def test_wait_for_done_reprompts_worker_once_after_unreported_terminal_work(
    tmp_path, monkeypatch
):
    rt = runtime(tmp_path, worker_report_timeout_seconds=1, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    worker = credential("worker-1", "worker", "worker-model")
    trial = TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://relay",
        channel_id="channel",
        credentials=(orch, worker),
    )
    (tmp_path / "logs").mkdir()
    (tmp_path / "logs" / "orchestration.jsonl").write_text(
        json.dumps(
            {
                "event": "terminal_exec",
                "agent_id": worker.agent_id,
                "ended_at": "2020-01-01T00:00:00+00:00",
            }
        )
        + "\n"
    )
    rounds = iter(
        [
            [],
            [{"id": "done", "pubkey": orch.nostr_pubkey, "content": "DONE: real"}],
        ]
    )
    sent = []

    async def buzz_json(*args, **kwargs):
        return next(rounds)

    async def send(*args):
        sent.append(args)

    async def no_sleep(_):
        return None

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    monkeypatch.setattr(rt, "_send", send)
    monkeypatch.setattr(
        "harbor_buzz_orchestra.subprocess_runtime.asyncio.sleep", no_sleep
    )

    result, recoveries = await rt._wait_for_done(orch, [worker], trial, [])
    assert result["id"] == "done"
    assert recoveries == [worker.agent_id]
    assert len(sent) == 1
    assert sent[0][0] == orch
    assert "Publish your result now" in sent[0][2]
    records = [
        json.loads(line)
        for line in (tmp_path / "logs" / "orchestration.jsonl")
        .read_text()
        .splitlines()
    ]
    recovery = records[-1]
    assert recovery["event"] == "worker_publish_recovery"
    assert recovery["worker_agent_id"] == worker.agent_id
    assert recovery["orchestrator_agent_id"] == orch.agent_id


def test_latest_worker_event_requires_successful_message_send_receipt(tmp_path):
    rt = runtime(tmp_path)
    records = [
        {
            "event": "buzz_exec",
            "agent_id": "worker-1",
            "args": ["messages", "get"],
            "ended_at": "2026-01-01T00:00:01+00:00",
            "return_code": 0,
            "error": None,
        },
        {
            "event": "buzz_exec",
            "agent_id": "worker-1",
            "args": ["messages", "send"],
            "ended_at": "2026-01-01T00:00:02+00:00",
            "return_code": 1,
            "error": "buzz exited 1",
        },
        {
            "event": "buzz_exec",
            "agent_id": "worker-1",
            "args": ["messages", "send"],
            "ended_at": "2026-01-01T00:00:03+00:00",
            "return_code": 0,
            "error": None,
        },
    ]
    assert rt._latest_worker_event(
        records, "worker-1", event="buzz_exec", require_message_send=True
    ) == datetime.fromisoformat("2026-01-01T00:00:03+00:00")


def test_recovery_policy_is_manifested_and_matches_runtime_bound(tmp_path):
    manifest = write_manifest(tmp_path).model_copy(
        update={
            "metadata": {
                "worker_publish_recovery": {
                    "enabled": True,
                    "max_attempts_per_worker": 1,
                    "timeout_seconds": 120,
                    "detection": (
                        "successful-worker-messages-send-receipt-after-terminal-exec"
                    ),
                }
            }
        }
    )
    runtime(tmp_path)._verify_recovery_policy(manifest)
    with pytest.raises(RuntimeLaunchError, match="worker_publish_recovery"):
        runtime(tmp_path, worker_report_timeout_seconds=60)._verify_recovery_policy(
            manifest
        )
