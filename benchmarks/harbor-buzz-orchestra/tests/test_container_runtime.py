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
    DEFAULT_MAX_AGENT_ROUNDS,
    DEFAULT_RUST_LOG,
    REMOTE_BIN,
    REMOTE_CA_BUNDLE,
    REMOTE_LOGS,
    SYSTEM_CA_BUNDLE,
    BuzzContainerRuntime,
    EndpointLaunchConfig,
    RuntimeLaunchError,
    _Agent,
)
from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle


def write_manifest(
    tmp_path: Path, *, include_platform_prompt: bool = True
) -> ExperimentManifest:
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    digest = hashlib.sha256(prompt.read_bytes()).hexdigest()
    roster_entry = {
        "count": 1,
        "model_revision": "r1",
        "prompt": {"path": "prompt.md", "sha256": digest},
        "include_platform_prompt": include_platform_prompt,
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


def credential(agent_id, role, endpoint, manifest_role=""):
    return AgentCredential(
        agent_id=agent_id,
        role=role,
        manifest_role=manifest_role,
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


# The /proc probe names the forwarder binary too, so "does this command mention
# FORWARDER" no longer distinguishes a launch from a lookup. Key on the
# backgrounding suffix, which only the launch has.
FORWARDER_LAUNCH = "& echo $!"


def _is_forwarder_launch(command):
    from harbor_buzz_orchestra.container_runtime import FORWARDER

    return FORWARDER in command and FORWARDER_LAUNCH in command


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
        REMOTE_CA_BUNDLE,
    }
    assert any("chmod 0755" in cmd for cmd, _ in environment.commands)


async def test_install_stack_requires_binaries_on_disk(tmp_path):
    rt = runtime(tmp_path, buzz_acp_binary=str(tmp_path / "missing"))
    with pytest.raises(RuntimeLaunchError, match="binary not found"):
        await rt._install_stack(Environment())


async def test_trust_anchors_are_shipped_because_task_images_may_have_none(tmp_path):
    """A task image without ca-certificates must not cost the agent the trial.

    buzz-agent links reqwest's rustls feature, which loads roots from the
    *container's* trust store. Several Terminal-Bench images ship none, so
    `Client::builder().build()` fails and the agent exits before it reaches the
    relay — scoring those tasks zero for every condition, for a reason that has
    nothing to do with the model. Uploading our own bundle and pointing
    SSL_CERT_FILE at it is what makes those tasks measurable at all.
    """
    binaries = {}
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp"):
        path = tmp_path / name
        path.write_text("#!binary")
        binaries[name] = str(path)
    bundle_path = tmp_path / "cacert.pem"
    bundle_path.write_text("-----BEGIN CERTIFICATE-----\n")
    rt = runtime(
        tmp_path,
        buzz_acp_binary=binaries["buzz-acp"],
        buzz_agent_binary=binaries["buzz-agent"],
        buzz_dev_mcp_binary=binaries["buzz-dev-mcp"],
        ca_bundle=str(bundle_path),
    )
    environment = Environment()
    await rt._install_stack(environment)
    assert (str(bundle_path), REMOTE_CA_BUNDLE) in environment.uploads
    # Outside REMOTE_BIN, so the executable chmod does not apply to it.
    assert not REMOTE_CA_BUNDLE.startswith(f"{REMOTE_BIN}/")

    orch = credential("orch-1", "orchestrator", "orch-model")
    env = rt._agent_env(
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=write_manifest(tmp_path).roster[0],
        endpoint=EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        remote_prompt="/opt/buzz/prompts/orch-1.system-prompt.md",
    )
    assert env["SSL_CERT_FILE"] == REMOTE_CA_BUNDLE
    # Never SSL_CERT_DIR: rustls-native-certs splits it on ':' and requires
    # every entry to be an existing directory, so an empty value names one bad
    # path and fails exactly the way this is meant to prevent.
    assert "SSL_CERT_DIR" not in env


def _stack_runtime(tmp_path, **kwargs):
    """A runtime whose binaries and CA bundle all exist on disk."""
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp"):
        (tmp_path / name).write_text("#!binary")
    bundle_path = tmp_path / "cacert.pem"
    bundle_path.write_text("-----BEGIN CERTIFICATE-----\n")
    return runtime(
        tmp_path,
        buzz_acp_binary=str(tmp_path / "buzz-acp"),
        buzz_agent_binary=str(tmp_path / "buzz-agent"),
        buzz_dev_mcp_binary=str(tmp_path / "buzz-dev-mcp"),
        ca_bundle=str(bundle_path),
        **kwargs,
    )


async def test_system_trust_store_is_seeded_for_apt_curl_and_pip(tmp_path):
    """SSL_CERT_FILE covers buzz-agent; nothing else in the container reads it.

    apt, curl and pip all read /etc/ssl/certs/ca-certificates.crt. On an image
    that ships no ca-certificates package that path is missing, so an agent
    that rewrites sources.list to https leaves the *verifier* unable to
    validate anything -- it reports `E: Unable to locate package curl`, never
    installs pytest, and the task scores 0.0 as if the model were wrong.
    """
    rt = _stack_runtime(tmp_path)
    environment = Environment(
        {SYSTEM_CA_BUNDLE: ExecResult(stdout="seeded", stderr="", return_code=0)}
    )
    assert await rt._install_stack(environment) == "seeded"
    seed = [cmd for cmd, _ in environment.commands if SYSTEM_CA_BUNDLE in cmd]
    assert len(seed) == 1
    # Copied from the bundle we already uploaded: no network, no package
    # manager, so this cannot itself become another egress dependency.
    assert REMOTE_CA_BUNDLE in seed[0]
    # -s, not -f: a partial `apt-get install ca-certificates` leaves a
    # zero-byte file that validates nothing but satisfies an existence test.
    assert f"[ -s {SYSTEM_CA_BUNDLE} ]" in seed[0]


async def test_an_images_own_trust_store_is_never_overwritten(tmp_path):
    """A task about certificate handling must keep the store its image shipped."""
    rt = _stack_runtime(tmp_path)
    environment = Environment(
        {SYSTEM_CA_BUNDLE: ExecResult(stdout="present", stderr="", return_code=0)}
    )
    assert await rt._install_stack(environment) == "present"


async def test_an_unseedable_trust_store_is_recorded_not_fatal(tmp_path):
    """A read-only /etc costs https, but killing the trial would cost everything.

    The agent still reaches its provider through SSL_CERT_FILE, so the trial
    remains runnable. What must not happen is silence: the disposition is
    returned so a later 0.0 can be read as suspect rather than as a wrong
    answer -- exactly the distinction the A1 sweep could not make.
    """
    rt = _stack_runtime(tmp_path)
    environment = Environment(
        {SYSTEM_CA_BUNDLE: ExecResult(stdout="failed", stderr="", return_code=1)}
    )
    assert await rt._install_stack(environment) == "failed"
    # An exec that returns nothing at all is a failure too, not a pass.
    blank = Environment(
        {SYSTEM_CA_BUNDLE: ExecResult(stdout="", stderr="", return_code=0)}
    )
    assert await rt._install_stack(blank) == "failed"


async def test_install_stack_requires_the_ca_bundle_on_disk(tmp_path):
    """Fail loudly at upload rather than as an opaque agent crash later."""
    for name in ("buzz-acp", "buzz-agent", "buzz-dev-mcp"):
        (tmp_path / name).write_text("#!binary")
    rt = runtime(
        tmp_path,
        buzz_acp_binary=str(tmp_path / "buzz-acp"),
        buzz_agent_binary=str(tmp_path / "buzz-agent"),
        buzz_dev_mcp_binary=str(tmp_path / "buzz-dev-mcp"),
        ca_bundle=str(tmp_path / "missing.pem"),
    )
    with pytest.raises(RuntimeLaunchError, match="CA bundle not found"):
        await rt._install_stack(Environment())


async def test_forwarder_bridges_the_canonical_relay_address(tmp_path):
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
            FORWARDER_LAUNCH: ExecResult(stdout="99\n", stderr="", return_code=0),
            "cat ": ExecResult(
                stdout="forwarding 127.0.0.1:3600 -> host.docker.internal:3600",
                stderr="",
                return_code=0,
            ),
        }
    )
    agent = await rt._start_forwarder(environment, trial)
    assert agent is not None and agent.pid == 99
    launch = next(cmd for cmd, _ in environment.commands if _is_forwarder_launch(cmd))
    # Listens on the canonical loopback (host-header bound), targets the gateway.
    assert "127.0.0.1:3600" in launch
    assert "host.docker.internal:3600" in launch

    # No gateway configured: the relay is reachable directly, no forwarder.
    assert await runtime(tmp_path)._start_forwarder(Environment(), trial) is None
    with pytest.raises(RuntimeLaunchError, match="ws://"):
        rt._ws_authority("http://relay")


@pytest.mark.parametrize(
    ("configured", "expected"),
    [(None, str(DEFAULT_MAX_AGENT_ROUNDS)), (7, "7")],
)
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


async def test_one_turn_may_last_as_long_as_the_trial_budget(tmp_path):
    """A solo agent's single turn is the whole trial, so the caps must agree.

    buzz-acp defaults to a 2h per-turn ceiling, and Terminal-Bench's longest
    tasks allow more. Leaving the default would end those turns mid-task with
    nothing published — the silent failure mode the round cap also has.
    """
    manifest = write_manifest(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    env = runtime(tmp_path)._agent_env(
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=manifest.roster[0],
        endpoint=EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        remote_prompt="/opt/buzz/prompts/orch-1.system-prompt.md",
        turn_timeout_seconds=12000,
    )
    assert env["BUZZ_ACP_MAX_TURN_DURATION"] == "12000"

    # The idle timer moves with the cap, or it becomes the deadline that
    # actually fires: at its 900s default it ended a live trial on a task
    # Harbor allowed 1800s. One second under keeps buzz-acp's
    # idle < max_turn startup invariant.
    assert env["BUZZ_ACP_IDLE_TIMEOUT"] == "11999"

    # Unknown budget leaves buzz-acp's own default alone rather than sending 0,
    # which would be read as "no time at all".
    unset = runtime(tmp_path)._agent_env(
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=manifest.roster[0],
        endpoint=EndpointLaunchConfig("anthropic", "ANTHROPIC_API_KEY"),
        remote_prompt="/opt/buzz/prompts/orch-1.system-prompt.md",
    )
    assert "BUZZ_ACP_MAX_TURN_DURATION" not in unset
    assert "BUZZ_ACP_IDLE_TIMEOUT" not in unset


async def test_launch_enables_the_usage_log_target(tmp_path):
    """Without this directive every trial silently reports zero tokens."""
    manifest = write_manifest(tmp_path)
    orch = credential("orch-1", "orchestrator", "orch-model")
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="1\n", stderr="", return_code=0)}
    )
    await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=manifest.roster[0],
        trial_dir=tmp_path,
    )
    _, env = environment.commands[-1]
    assert "acp::usage=debug" in env["RUST_LOG"]


@pytest.mark.parametrize(
    ("configured", "expected"),
    [
        (None, DEFAULT_RUST_LOG),
        # Operator verbosity is preserved, but never at the cost of the targets
        # the harness reads: tokens, and whether a turn has ended.
        (
            "buzz_acp=trace",
            (
                "buzz_acp=trace,acp::usage=debug,pool::prompt=info,"
                "acp::stream=info,acp::tool=info"
            ),
        ),
        # A target the operator already set keeps the operator's level.
        (
            "acp::usage=trace,pool::prompt=trace",
            "acp::usage=trace,pool::prompt=trace,acp::stream=info,acp::tool=info",
        ),
    ],
)
def test_stack_rust_log_never_drops_a_target_the_harness_reads(configured, expected):
    assert BuzzContainerRuntime._stack_rust_log(configured) == expected


def test_default_rust_log_enables_every_target_the_harness_reads():
    """Both are load-bearing: one prices the trial, one ends it.

    The first full sweep ran without the turn target and paid 3.2 hours of dead
    waiting for it.
    """
    for target in ("acp::usage", "pool::prompt"):
        assert target in DEFAULT_RUST_LOG


def test_solo_prompt_states_there_is_nobody_to_delegate_to(tmp_path):
    """A solo agent told nothing about its team burns rounds hunting for one."""
    rt = runtime(tmp_path)
    solo = credential("solo-1", "orchestrator", "orch-model")
    persona = tmp_path / "persona.md"
    persona.write_text("# Solo\n", encoding="utf-8")

    composed = rt._compose_system_prompt(
        trial_dir=tmp_path,
        trial=trial_handle((solo,)),
        credential=solo,
        persona_path=persona,
    ).read_text(encoding="utf-8")

    assert "no teammates" in composed
    assert "| Name | Role | Pubkey |" not in composed


async def test_solo_roster_runs_and_is_priced(tmp_path, monkeypatch):
    """The zero-worker baseline must run, and must not report $0.00 by default."""
    manifest = ExperimentManifest.load(
        {
            "condition": "solo",
            "roster": [
                {
                    "id": "solo",
                    "kind": "orchestrator",
                    "role": "lead",
                    "count": 1,
                    "endpoint": "orch-model",
                    "model_revision": "r1",
                    "prompt": {
                        "path": "prompt.md",
                        "sha256": hashlib.sha256(b"prompt").hexdigest(),
                    },
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                }
            ],
            "prices": {
                "orch-model": {
                    "input_per_million_usd": 10,
                    "cached_input_per_million_usd": 1,
                    "output_per_million_usd": 30,
                }
            },
            "trial_budget": {"timeout_seconds": 30},
        }
    )
    (tmp_path / "prompt.md").write_text("prompt", encoding="utf-8")
    solo = credential("solo-1", "orchestrator", "orch-model")
    trial = trial_handle((solo,))
    rt = runtime(tmp_path, poll_seconds=0)

    class SoloEnvironment(Environment):
        async def exec(self, command, env=None, **kwargs):
            self.commands.append((command, env))
            if "buzz-acp" in command:
                return ExecResult(stdout="99\n", stderr="", return_code=0)
            if command.startswith("cat "):
                # Both markers the harness polls this log for: the readiness
                # subscription, and the turn's usage line.
                return ExecResult(
                    stdout=(
                        "subscribed to channel channel\n"
                        "goose usage update input=2000000 output=1000000\n"
                    ),
                    stderr="",
                    return_code=0,
                )
            return ExecResult(stdout="", stderr="", return_code=0)

        async def download_dir(self, source, target):
            # Stand in for the real log download.
            Path(target).mkdir(parents=True, exist_ok=True)
            (Path(target) / "solo-1.stdout.log").write_text(
                "2026-07-26T00:00:00Z DEBUG acp::usage: goose usage update "
                "session_id=s input=2000000 output=1000000\n",
                encoding="utf-8",
            )

    monkeypatch.setattr(rt, "_install_stack", lambda environment: _noop())
    monkeypatch.setattr(
        rt,
        "_wait_for_done",
        lambda *a, **k: _value({"id": "m1", "content": "DONE: done"}),
    )

    async def buzz_json(*args, **kwargs):
        return []

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)

    result = await rt.run(
        instruction="do the thing",
        environment=SoloEnvironment(),
        manifest=manifest,
        trial=trial,
    )

    assert result.input_tokens == 2_000_000
    assert result.output_tokens == 1_000_000
    # $10/Mtok in + $30/Mtok out
    assert result.cost_usd == pytest.approx(20.0 + 30.0)
    assert result.metadata["solo_roster"] is True
    assert result.metadata["accounting_reconciled"] is True
    assert result.metadata["agent_active_seconds"] >= 0
    timing = json.loads((tmp_path / "logs" / "buzz" / "timing.json").read_text())
    assert timing["agent_active_seconds"] == pytest.approx(
        result.metadata["agent_active_seconds"]
    )
    assert timing["usage_settle_seconds"] >= 0
    # The bundle landed next to the logs.
    summary = json.loads((tmp_path / "logs" / "buzz" / "summary.json").read_text())
    assert summary["solo_roster"] is True
    assert summary["secret_scan"]["clean"] is True


async def _noop():
    return None


async def _value(value):
    return value


def test_runtime_validates_construction_bounds(tmp_path):
    # 0 is legal and means unbounded (BUZZ_AGENT_MAX_ROUNDS=0); the trial
    # budget is the clock. Only negatives are rejected.
    runtime(tmp_path, max_agent_rounds=0)
    with pytest.raises(ValueError, match="unbounded"):
        runtime(tmp_path, max_agent_rounds=-1)
    with pytest.raises(ValueError, match="positive"):
        runtime(tmp_path, readiness_timeout_seconds=0)
    with pytest.raises(ValueError, match="negative"):
        runtime(tmp_path, usage_settle_seconds=-1)


def settle_agent(agent_id="solo-1"):
    return _Agent(
        credential=credential(agent_id, "orchestrator", "orch-model"),
        pid=99,
        stdout_log=f"{REMOTE_LOGS}/{agent_id}.stdout.log",
        stderr_log=f"{REMOTE_LOGS}/{agent_id}.stderr.log",
    )


async def test_settle_waits_for_a_usage_line_that_has_not_landed_yet(tmp_path):
    """The one record of a trial's tokens is written after `DONE:` is published.

    buzz-agent emits its usage notification just before returning the
    session/prompt response, so a solo agent's only usage line trails the
    `DONE:` tool call it is billed for.
    """
    rt = runtime(tmp_path, poll_seconds=0, usage_settle_seconds=5)

    class LateUsage(Environment):
        polls = 0

        async def exec(self, command, env=None, **kwargs):
            self.polls += 1
            stdout = "goose usage update input=1 output=2\n" if self.polls >= 3 else ""
            return ExecResult(stdout=stdout, stderr="", return_code=0)

    environment = LateUsage()
    await rt._settle_usage(environment, [settle_agent()])
    assert environment.polls == 3


async def test_settle_reads_both_streams_of_every_agent(tmp_path):
    """A pair's usage lines arrive independently, and either stream may hold one."""
    rt = runtime(tmp_path, poll_seconds=0, usage_settle_seconds=5)
    seen = []

    class TwoAgents(Environment):
        async def exec(self, command, env=None, **kwargs):
            seen.append(command)
            # The driver reports at once; the navigator only on its second poll.
            if "orch-1" in command or len(seen) > 2:
                return ExecResult(
                    stdout="goose usage update input=1 output=2\n",
                    stderr="",
                    return_code=0,
                )
            return ExecResult(stdout="", stderr="", return_code=0)

    await rt._settle_usage(
        TwoAgents(), [settle_agent("orch-1"), settle_agent("worker-1")]
    )
    # The satisfied agent drops out; only the quiet one is polled again.
    assert len(seen) == 3
    assert all(
        ".stdout.log" in command and ".stderr.log" in command for command in seen
    )
    assert "orch-1" not in seen[-1]


async def test_a_usage_line_that_never_comes_does_not_hang_the_sweep(tmp_path):
    """Bounded by design: a lost cost record is cheaper than a stalled run.

    The accounting layer already reports the trial as unpriced, and the task's
    own result stands regardless.
    """
    rt = runtime(tmp_path, poll_seconds=0, usage_settle_seconds=0)

    class NeverReports(Environment):
        polls = 0

        async def exec(self, command, env=None, **kwargs):
            self.polls += 1
            return ExecResult(stdout="", stderr="", return_code=0)

    environment = NeverReports()
    await rt._settle_usage(environment, [settle_agent()])
    assert environment.polls == 1


async def test_usage_is_settled_before_teardown_kills_the_agent(tmp_path, monkeypatch):
    """The whole point of the wait: it has to happen while the agent is alive.

    24 of 89 trials in the first full solo sweep reported zero tokens — 14 of
    them passing — because teardown fired inside this window.
    """
    manifest = write_manifest(tmp_path)
    credentials = (
        credential("orch-1", "orchestrator", "orch-model", "lead"),
        credential("worker-1", "worker", "worker-model", "implementer"),
    )
    trial = trial_handle(credentials)
    rt = runtime(tmp_path, poll_seconds=0, usage_settle_seconds=5)
    order = []

    class LateUsageEnvironment(Environment):
        done = False
        settles = 0

        async def exec(self, command, env=None, **kwargs):
            if "buzz-acp" in command:
                return ExecResult(stdout="99\n", stderr="", return_code=0)
            if command.startswith("cat "):
                stdout = "subscribed to channel channel\n"
                if self.done:
                    self.settles += 1
                    if self.settles >= 4:  # two agents, two rounds
                        order.append("usage")
                        stdout += "goose usage update input=1 output=2\n"
                return ExecResult(stdout=stdout, stderr="", return_code=0)
            if "/proc/[0-9]*" in command:
                order.append("kill")
            return ExecResult(stdout="", stderr="", return_code=0)

    environment = LateUsageEnvironment()

    async def done(*args, **kwargs):
        environment.done = True
        return {"id": "m1", "content": "DONE: done"}

    monkeypatch.setattr(rt, "_install_stack", lambda env: _noop())
    monkeypatch.setattr(rt, "_wait_for_done", done)
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value([]))

    await rt.run(
        instruction="do the thing",
        environment=environment,
        manifest=manifest,
        trial=trial,
    )

    assert order[0] == "usage", "teardown ran before the usage line was flushed"
    assert "kill" in order


async def test_usage_is_settled_when_the_trial_times_out(tmp_path, monkeypatch):
    """The timeout path has to settle too, and it is the path that matters most.

    It used to fall straight through to the kill, on the reasoning that a turn
    which never completed had nothing to flush. buzz-agent now reports after
    every provider round, so an interrupted turn HAS reported — and under
    `continue_until_timeout` every phase but the last ends here. Skipping the
    settle on this path is what left 97% of one measured run's receipt rows at
    all zeros while the provider billed it in full.
    """
    manifest = write_manifest(tmp_path)
    trial = trial_handle((credential("orch-1", "orchestrator", "orch-model", "lead"),))
    rt = runtime(tmp_path, poll_seconds=0, usage_settle_seconds=5)
    order = []

    class TimingOutEnvironment(Environment):
        async def exec(self, command, env=None, **kwargs):
            if "buzz-acp" in command:
                return ExecResult(stdout="99\n", stderr="", return_code=0)
            if command.startswith("cat "):
                order.append("usage")
                return ExecResult(
                    stdout="goose usage update input=1 output=2\n",
                    stderr="",
                    return_code=0,
                )
            if "/proc/[0-9]*" in command:
                order.append("kill")
            return ExecResult(stdout="", stderr="", return_code=0)

    async def never_done(*args, **kwargs):
        await asyncio.sleep(3600)

    monkeypatch.setattr(rt, "_install_stack", lambda env: _noop())
    monkeypatch.setattr(rt, "_wait_for_agents_ready", lambda *a, **k: _noop())
    monkeypatch.setattr(rt, "_wait_for_done", never_done)
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value([]))
    # TrialBudget is frozen, so swap in a zero-budget copy rather than mutating.
    budget = manifest.trial_budget.model_copy(update={"timeout_seconds": 0})
    manifest = manifest.model_copy(update={"trial_budget": budget})

    with pytest.raises(asyncio.TimeoutError):
        await rt.run(
            instruction="do the thing",
            environment=TimingOutEnvironment(),
            manifest=manifest,
            trial=trial,
        )

    assert "usage" in order, "the timeout path never settled usage"
    assert order.index("usage") < order.index("kill"), (
        "usage was settled after teardown killed the agent, which is the same "
        "as not settling at all"
    )


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


async def test_a_lone_agent_that_ends_its_turn_ends_the_trial(tmp_path, monkeypatch):
    """Nobody left to wake it, so waiting only inflates the clock.

    13 trials in the first full solo sweep sat like this for a quarter of an hour
    each — 3.2 hours, a third of the sweep's agent time — after work that had
    finished in under four minutes. Five had already passed their tests.
    """
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value([]))

    ended = Environment(
        responses={
            "cat ": ExecResult(
                stdout="turn complete for channel c: end_turn\n",
                stderr="",
                return_code=0,
            )
        }
    )
    assert (
        await rt._wait_for_done(ended, orch, trial, [], solo=settle_agent("orch-1"))
        is None
    )


@pytest.mark.parametrize(
    "line",
    [
        "turn hit max_tokens for channel c — session will be rotated",
        "turn hit max_turn_requests for channel c — session will be rotated",
        "turn refused for channel c",
        "turn cancelled for channel c",
    ],
)
async def test_a_turn_that_ended_badly_still_ended(tmp_path, monkeypatch, line):
    """An agent stopped by its own limits is finished, not thinking."""
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value([]))
    environment = Environment(
        responses={"cat ": ExecResult(stdout=line, stderr="", return_code=0)}
    )
    assert (
        await rt._wait_for_done(
            environment, orch, trial_handle((orch,)), [], solo=settle_agent("orch-1")
        )
        is None
    )


async def test_a_working_agent_is_never_mistaken_for_a_finished_one(
    tmp_path, monkeypatch
):
    """No turn-end line means the agent is still mid-turn: keep waiting.

    Stopping early here would throw away a live trial, so the DONE that arrives
    on a later poll has to win.
    """
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    rounds = iter(
        [[], [], [{"id": "9", "pubkey": orch.nostr_pubkey, "content": "DONE: ok"}]]
    )
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value(next(rounds)))
    working = Environment(
        responses={
            "cat ": ExecResult(
                stdout="subscribed to channel c\n", stderr="", return_code=0
            )
        }
    )
    result = await rt._wait_for_done(
        working, orch, trial_handle((orch,)), [], solo=settle_agent("orch-1")
    )
    assert result["content"] == "DONE: ok"


async def test_a_team_keeps_waiting_because_a_worker_can_still_wake_the_lead(
    tmp_path, monkeypatch
):
    """`solo` is the whole guard: with more than one agent, turn-end proves nothing.

    buzz-acp logs turn ends and not turn starts, so a lead between turns looks
    exactly like a lead that has stopped for good.
    """
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    rounds = iter(
        [[], [{"id": "9", "pubkey": orch.nostr_pubkey, "content": "DONE: ok"}]]
    )
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value(next(rounds)))
    ended = Environment(
        responses={
            "cat ": ExecResult(
                stdout="turn complete for channel c: end_turn\n",
                stderr="",
                return_code=0,
            )
        }
    )
    # No solo agent passed: the run() call site only passes one for a lone agent.
    result = await rt._wait_for_done(ended, orch, trial_handle((orch,)), [])
    assert result["content"] == "DONE: ok"


async def test_a_quiet_stop_is_recorded_rather_than_inferred(tmp_path, monkeypatch):
    """The trial completes and is priced; the dropped protocol is a flag, not a zero."""
    manifest = ExperimentManifest.load(
        {
            "condition": "solo",
            "roster": [
                {
                    "id": "solo",
                    "kind": "orchestrator",
                    "role": "lead",
                    "count": 1,
                    "endpoint": "orch-model",
                    "model_revision": "r1",
                    "prompt": {
                        "path": "prompt.md",
                        "sha256": hashlib.sha256(b"prompt").hexdigest(),
                    },
                    "generation": {
                        "max_output_tokens": 100,
                        "context_window_tokens": 1000,
                    },
                }
            ],
            "prices": {
                "orch-model": {
                    "input_per_million_usd": 10,
                    "cached_input_per_million_usd": 1,
                    "output_per_million_usd": 30,
                }
            },
            "trial_budget": {"timeout_seconds": 30},
        }
    )
    (tmp_path / "prompt.md").write_text("prompt", encoding="utf-8")
    solo = credential("solo-1", "orchestrator", "orch-model")
    rt = runtime(tmp_path, poll_seconds=0)

    class QuietStop(Environment):
        async def exec(self, command, env=None, **kwargs):
            self.commands.append((command, env))
            if "buzz-acp" in command:
                return ExecResult(stdout="99\n", stderr="", return_code=0)
            if command.startswith("cat "):
                return ExecResult(
                    stdout=(
                        "subscribed to channel channel\n"
                        "goose usage update input=1000 output=500\n"
                        "turn complete for channel channel: end_turn\n"
                    ),
                    stderr="",
                    return_code=0,
                )
            return ExecResult(stdout="", stderr="", return_code=0)

        async def download_dir(self, source, target):
            Path(target).mkdir(parents=True, exist_ok=True)
            (Path(target) / "solo-1.stdout.log").write_text(
                "goose usage update session_id=s input=1000 output=500\n",
                encoding="utf-8",
            )

    monkeypatch.setattr(rt, "_install_stack", lambda environment: _noop())
    monkeypatch.setattr(rt, "_buzz_json", lambda *a, **k: _value([]))

    result = await rt.run(
        instruction="do the thing",
        environment=QuietStop(),
        manifest=manifest,
        trial=trial_handle((solo,)),
    )

    assert result.metadata["stopped_without_done"] is True
    assert result.metadata["completion_message_id"] == ""
    # Still priced — the tokens were spent whether or not DONE was posted.
    assert result.input_tokens == 1000


async def test_transcript_is_saved_in_author_order_with_names(tmp_path, monkeypatch):
    """The transcript is the only record of who said what to whom.

    Ordered oldest-first and keyed by roster id, because the reason to keep it
    is reading a team's turn-taking — a pubkey-keyed reverse-chronological dump
    answers no question anyone actually asks of it.
    """
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model", "lead")
    worker = credential("worker-1", "worker", "worker-model", "implementer")
    trial = trial_handle((orch, worker))

    async def buzz_json(credential, *args, **kwargs):
        return [
            {
                "id": "3",
                "pubkey": orch.nostr_pubkey,
                "content": "DONE: x",
                "created_at": 3,
            },
            {
                "id": "1",
                "pubkey": trial.user.nostr_pubkey,
                "content": "@orch-1 go",
                "created_at": 1,
            },
            {
                "id": "2",
                "pubkey": worker.nostr_pubkey,
                "content": "@orch-1 done",
                "created_at": 2,
            },
        ]

    monkeypatch.setattr(rt, "_buzz_json", buzz_json)
    await rt._collect_transcript(trial, tmp_path)

    saved = json.loads((tmp_path / "transcript.json").read_text())
    assert saved["channel_id"] == trial.channel_id
    assert saved["message_count"] == 3
    assert saved["truncated"] is False
    assert [m["author"] for m in saved["messages"]] == ["user", "worker-1", "orch-1"]
    assert saved["messages"][0]["content"] == "@orch-1 go"


async def test_a_lost_transcript_never_fails_a_finished_trial(tmp_path, monkeypatch):
    """This runs in a finally; a relay hiccup must not mask the real outcome."""
    rt = runtime(tmp_path, poll_seconds=0)
    orch = credential("orch-1", "orchestrator", "orch-model")
    trial = trial_handle((orch,))

    async def exploding(*args, **kwargs):
        raise RuntimeLaunchError("relay gone")

    monkeypatch.setattr(rt, "_buzz_json", exploding)
    await rt._collect_transcript(trial, tmp_path)
    assert not (tmp_path / "transcript.json").exists()


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


@pytest.mark.parametrize(
    ("overrides", "expected"),
    [
        # Silent manifest: nothing pinned, so the agent's own defaults apply
        # (90%, 272k ceiling) and the container env says so by omission.
        ({}, {}),
        (
            {"compact_at_tokens": 272_000},
            {"BUZZ_AGENT_HANDOFF_AT_TOKENS": "272000"},
        ),
        (
            {"compact_at_percent": 30},
            {"BUZZ_AGENT_HANDOFF_PERCENT": "30"},
        ),
        (
            {"compact_at_percent": 30, "compact_at_tokens": 272_000},
            {
                "BUZZ_AGENT_HANDOFF_PERCENT": "30",
                "BUZZ_AGENT_HANDOFF_AT_TOKENS": "272000",
            },
        ),
    ],
)
async def test_compaction_policy_reaches_the_agent(tmp_path, overrides, expected):
    """The manifest drives buzz-agent's real knobs — no window arithmetic."""
    manifest = write_manifest(tmp_path)
    agent_class = manifest.roster[0]
    agent_class = agent_class.model_copy(
        update={
            "generation": agent_class.generation.model_copy(
                update={"context_window_tokens": 1_000_000, **overrides}
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
    # The real window is always reported honestly, never bent to move the trigger.
    assert env["BUZZ_AGENT_MAX_CONTEXT_TOKENS"] == "1000000"
    for key in ("BUZZ_AGENT_HANDOFF_PERCENT", "BUZZ_AGENT_HANDOFF_AT_TOKENS"):
        assert env.get(key) == expected.get(key)


async def test_an_unpinned_window_sends_no_window_variables(tmp_path):
    """An unset field must be absent from the env, not the string "None".

    ``str(None)`` would reach the container as a literal ``"None"`` where a token
    count belongs — the agent cannot parse that, and the trial bundle would claim
    the condition pinned a value it did not.
    """
    manifest = write_manifest(tmp_path)
    agent_class = manifest.roster[0]
    agent_class = agent_class.model_copy(
        update={
            "generation": agent_class.generation.model_copy(
                update={"max_output_tokens": None, "context_window_tokens": None}
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
    assert "BUZZ_AGENT_MAX_OUTPUT_TOKENS" not in env
    assert "BUZZ_AGENT_MAX_CONTEXT_TOKENS" not in env


async def test_thinking_effort_is_pinned_for_every_agent(tmp_path):
    """An unset effort is the provider's default: unrecorded and not portable.

    Every condition claims the same reasoning effort, so the variable has to be
    present in the container env rather than left to each endpoint's default.
    """
    manifest = write_manifest(tmp_path)
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="4242\n", stderr="", return_code=0)}
    )
    orch = credential("orch-1", "orchestrator", "orch-model")
    await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=manifest.roster[0],
        trial_dir=tmp_path,
    )
    _, env = environment.commands[-1]
    assert env["BUZZ_AGENT_THINKING_EFFORT"] == "medium"


async def test_thinking_effort_can_be_raised_per_condition(tmp_path):
    """The G2 axis: a roster entry may pin an effort above the harness default.

    Guards the A2x / A3x cells specifically. Their whole value is that the only
    difference from A2 / A3 is this one field, so if the manifest value failed
    to reach the container the cells would still run, still score, and quietly
    measure nothing -- two full 89-task sweeps whose null result was an artifact.
    """
    manifest = write_manifest(tmp_path)
    entry = manifest.roster[0].model_copy(
        update={
            "generation": manifest.roster[0].generation.model_copy(
                update={"thinking_effort": "xhigh"}
            )
        }
    )
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="4242\n", stderr="", return_code=0)}
    )
    orch = credential("orch-1", "orchestrator", "orch-model")
    await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=entry,
        trial_dir=tmp_path,
    )
    _, env = environment.commands[-1]
    assert env["BUZZ_AGENT_THINKING_EFFORT"] == "xhigh"


@pytest.mark.parametrize(
    ("harness", "command"),
    [("goose", "goose"), ("codex", "codex-acp")],
)
async def test_external_harness_gets_same_model_and_effort(tmp_path, harness, command):
    manifest = write_manifest(tmp_path)
    entry = manifest.roster[0].model_copy(
        update={
            "harness": harness,
            "generation": manifest.roster[0].generation.model_copy(
                update={"thinking_effort": "high"}
            ),
        }
    )
    orch = credential("orch-1", "orchestrator", "gpt-5.6-luna")
    env = runtime(tmp_path)._agent_env(
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=entry,
        endpoint=EndpointLaunchConfig("openai", "OPENAI_COMPAT_API_KEY"),
        remote_prompt="/opt/buzz/prompts/orch-1.system-prompt.md",
    )
    assert env["BUZZ_ACP_AGENT_COMMAND"] == f"{REMOTE_BIN}/{command}"
    assert env["OPENAI_API_KEY"] == orch.llm_api_key
    assert "BUZZ_AGENT_MODEL" not in env
    if harness == "goose":
        assert env["GOOSE_MODEL"] == "gpt-5.6-luna"
        assert env["GOOSE_THINKING_EFFORT"] == "high"
    else:
        assert env["CODEX_PATH"] == f"{REMOTE_BIN}/codex"
        assert json.loads(env["DEFAULT_AUTH_REQUEST"])["methodId"] == "api-key"
        assert env["INITIAL_AGENT_MODE"] == "agent-full-access"
        assert json.loads(env["CODEX_CONFIG"])["model"] == "gpt-5.6-luna"
        assert json.loads(env["CODEX_CONFIG"])["model_reasoning_effort"] == "high"


async def test_goose_prompt_replacement_mode_reaches_buzz_acp(tmp_path):
    manifest = write_manifest(tmp_path)
    entry = manifest.roster[0].model_copy(
        update={
            "harness": "goose",
            "generation": manifest.roster[0].generation.model_copy(
                update={"extra": {"goose_system_prompt_mode": "set"}}
            ),
        }
    )
    orch = credential("orch-1", "orchestrator", "gpt-5.6-terra")
    env = runtime(tmp_path)._agent_env(
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=entry,
        endpoint=EndpointLaunchConfig("openai", "OPENAI_COMPAT_API_KEY"),
        remote_prompt="/opt/buzz/prompts/orch-1.system-prompt.md",
    )
    assert env["BUZZ_ACP_GOOSE_SYSTEM_PROMPT_MODE"] == "set"


@pytest.mark.parametrize(
    ("include", "expected"),
    [(True, None), (False, "true")],
)
async def test_platform_prompt_can_be_switched_off_per_condition(
    tmp_path, include, expected
):
    """The [Base] section is ~12KB of prompt the condition did not write.

    It is buzz-acp's production-workspace guidance, prepended ahead of the
    pinned persona, and much of it is wrong inside a graded container. Leaving
    it on is the honest default, but the sensitivity run that turns it off is
    the only thing that shows how much of a result it accounts for — so the
    switch has to reach the container, and its absence has to be real absence
    rather than a "0" buzz-acp would read as set.
    """
    manifest = write_manifest(tmp_path, include_platform_prompt=include)
    environment = Environment(
        responses={"buzz-acp": ExecResult(stdout="4242\n", stderr="", return_code=0)}
    )
    orch = credential("orch-1", "orchestrator", "orch-model")
    await runtime(tmp_path)._launch_agent(
        environment=environment,
        trial=trial_handle((orch,)),
        credential=orch,
        agent_class=manifest.roster[0],
        trial_dir=tmp_path,
    )
    _, env = environment.commands[-1]
    assert env.get("BUZZ_ACP_NO_BASE_PROMPT") == expected


def test_team_table_shows_the_manifest_role_not_the_kind(tmp_path):
    """A lead must be able to tell its implementer from its verifier.

    Both are `kind: worker`, so a table rendered from the kind would list two
    identical rows — and every persona addresses teammates by the job in that
    column. Falls back to the kind so a pre-v1.3 provisioner still composes.
    """
    solo = credential("solo-1", "orchestrator", "m")
    impl = credential("impl-1", "worker", "m", manifest_role="implementer")
    critic = credential("critic-1", "worker", "m", manifest_role="critic")
    bare = credential("bare-1", "worker", "m")
    prompt = tmp_path / "persona.md"
    prompt.write_text("persona\n", encoding="utf-8")
    composed = (
        runtime(tmp_path)
        ._compose_system_prompt(
            trial_dir=tmp_path,
            trial=trial_handle((solo, impl, critic, bare)),
            credential=solo,
            persona_path=prompt,
        )
        .read_text(encoding="utf-8")
    )
    assert "| impl-1 | implementer |" in composed
    assert "| critic-1 | critic |" in composed
    assert "| bare-1 | worker |" in composed


def _forwarder_runtime(tmp_path):
    """A runtime wired to a fake forwarder binary and a gateway."""
    binary = tmp_path / "relay-forwarder"
    binary.write_text("ELF")
    return runtime(
        tmp_path,
        relay_gateway="host.docker.internal:3600",
        forwarder_binary=str(binary),
    )


def _forwarder_trial():
    return TrialHandle(
        run_id="run",
        trial_id="trial",
        manifest_hash="hash",
        relay_ws_url="ws://localhost:3600",
        channel_id="channel",
        credentials=(),
        user=user_credential(),
    )


class _BindsAfter(Environment):
    """Reports EADDRINUSE for the first `failures` launches, then succeeds.

    Models the real race: `_stop_agents` TERMs the previous forwarder without
    waiting, so an immediate relaunch loses the socket until the old process
    finishes exiting.
    """

    def __init__(self, failures):
        super().__init__()
        self.remaining = failures
        self.launches = 0
        self._log = ""

    async def exec(self, command, env=None, **kwargs):
        self.commands.append((command, env))
        if command.startswith(": >"):
            self._log = ""
            return ExecResult(stdout="", stderr="", return_code=0)
        if _is_forwarder_launch(command):
            self.launches += 1
            if self.remaining > 0:
                self.remaining -= 1
                self._log = "Error: Os { code: 98, kind: AddrInUse }"
            else:
                self._log = "forwarding 127.0.0.1:3600 -> host.docker.internal:3600"
            return ExecResult(stdout="99\n", stderr="", return_code=0)
        if command.startswith("cat "):
            return ExecResult(stdout=self._log, stderr="", return_code=0)
        return ExecResult(stdout="", stderr="", return_code=0)


async def test_forwarder_retries_when_the_port_is_still_held(tmp_path, monkeypatch):
    rt = _forwarder_runtime(tmp_path)
    monkeypatch.setattr(type(rt), "FORWARDER_BIND_BACKOFF_S", 0.0)
    environment = _BindsAfter(failures=2)

    agent = await rt._start_forwarder(environment, _forwarder_trial())

    assert agent is not None and agent.pid == 99
    assert environment.launches == 3, "should relaunch once per failed bind"


async def test_forwarder_gives_up_after_the_attempt_budget(tmp_path, monkeypatch):
    rt = _forwarder_runtime(tmp_path)
    monkeypatch.setattr(type(rt), "FORWARDER_BIND_BACKOFF_S", 0.0)
    environment = _BindsAfter(failures=999)

    with pytest.raises(RuntimeLaunchError, match="could not bind"):
        await rt._start_forwarder(environment, _forwarder_trial())

    assert environment.launches == type(rt).FORWARDER_BIND_ATTEMPTS


async def test_forwarder_does_not_retry_other_failures(tmp_path, monkeypatch):
    """Only EADDRINUSE is transient; a silent forwarder is a real fault."""
    rt = _forwarder_runtime(tmp_path)
    monkeypatch.setattr(type(rt), "readiness_timeout_seconds", 0.0, raising=False)
    rt.readiness_timeout_seconds = 0.0
    environment = Environment(
        responses={
            FORWARDER_LAUNCH: ExecResult(stdout="99\n", stderr="", return_code=0),
            "cat ": ExecResult(stdout="", stderr="", return_code=0),
        }
    )
    with pytest.raises(RuntimeLaunchError, match="did not report readiness"):
        await rt._start_forwarder(environment, _forwarder_trial())

    launches = sum(1 for cmd, _ in environment.commands if _is_forwarder_launch(cmd))
    assert launches == 1, "a non-AddrInUse failure must stay fatal on attempt 1"


class _HasLiveForwarder(Environment):
    """A container where a forwarder from an earlier phase is still running."""

    def __init__(self, pid=4242):
        super().__init__()
        self.pid = pid

    async def exec(self, command, env=None, **kwargs):
        from harbor_buzz_orchestra.container_runtime import _forwarder_probe

        self.commands.append((command, env))
        if command == _forwarder_probe():
            return ExecResult(stdout=f"{self.pid}\n", stderr="", return_code=0)
        return ExecResult(stdout="", stderr="", return_code=0)


async def test_forwarder_from_an_earlier_phase_is_adopted(tmp_path):
    """continue_until_timeout re-runs the agent; the bridge must be reused.

    Rebinding is not merely wasteful, it is impossible: the forwarder binds
    without SO_REUSEADDR, so its accepted sockets hold the port in TIME_WAIT
    for 60s after it exits.
    """
    rt = _forwarder_runtime(tmp_path)
    environment = _HasLiveForwarder(pid=4242)

    agent = await rt._start_forwarder(environment, _forwarder_trial())

    assert agent is not None and agent.pid == 4242
    launches = sum(1 for cmd, _ in environment.commands if _is_forwarder_launch(cmd))
    assert launches == 0, "must adopt the running forwarder, not launch another"


async def test_teardown_spares_the_forwarder(tmp_path):
    """The per-phase sweep kills the agent stack but leaves the bridge up."""
    rt = _forwarder_runtime(tmp_path)
    environment = Environment()
    agents = [
        _Agent(credential("solo-1", "orchestrator", "m"), 11, "out.log", "err.log")
    ]
    await rt._stop_agents(environment, agents)

    sweeps = [cmd for cmd, _ in environment.commands if "/proc/" in cmd]
    assert sweeps, "expected a teardown sweep"
    for sweep in sweeps:
        assert "! grep -aq relay-forwarder" in sweep, (
            "sweep must exclude the forwarder or the next phase cannot bind"
        )


def test_forwarder_probe_ignores_shells_that_merely_name_the_forwarder(tmp_path):
    """Regression: the probe used to report the PID of the shell running it.

    A substring search over /proc/*/cmdline matches the probe's own `sh -c ...`,
    because the search term is the forwarder path and the probe command contains
    it. `_start_forwarder` then adopts a PID that exits the moment the probe
    returns, and the readiness check reports "agent processes exited early:
    ['relay-forwarder']" on a container that never started one — which is how
    this cost two smoke runs. Process 200 below is that shell.
    """
    import subprocess

    from harbor_buzz_orchestra.container_runtime import (
        FORWARDER,
        _forwarder_probe,
    )

    proc = tmp_path / "proc"
    (proc / "100").mkdir(parents=True)
    (proc / "100" / "cmdline").write_bytes(
        b"\0".join([FORWARDER.encode(), b"127.0.0.1:3600", b"10.0.0.1:3000"]) + b"\0"
    )
    (proc / "200").mkdir()
    (proc / "200" / "cmdline").write_bytes(
        b"\0".join([b"sh", b"-c", _forwarder_probe().encode()]) + b"\0"
    )

    out = subprocess.run(
        ["sh", "-c", _forwarder_probe(str(proc))],
        capture_output=True,
        text=True,
        check=True,
    )

    assert out.stdout.split() == ["100"], (
        f"probe must match argv[0] only, got {out.stdout!r}"
    )
