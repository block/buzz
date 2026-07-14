from types import SimpleNamespace
from uuid import uuid4
import pytest
from harbor.models.agent.context import AgentContext
from harbor_buzz_orchestra import (
    AgentCredential,
    BuzzOrchestraAgent,
    RuntimeResult,
    TrialHandle,
)

pytestmark = pytest.mark.asyncio


async def test_agent_credential_carries_closed_relay_attestation():
    credential = AgentCredential(
        agent_id="orchestrator-1",
        role="orchestrator",
        nostr_secret_key="11" * 32,
        nostr_pubkey="22" * 32,
        nostr_auth_tag='["auth","owner","conditions","signature"]',
        llm_endpoint="https://example.databricks.com/serving-endpoints/opus",
        llm_api_key="attributed-key",
    )

    assert credential.nostr_auth_tag.startswith('["auth"')


class Provisioner:
    def __init__(self):
        self.healthchecked = False
        self.created = None
        self.torn_down = None

    def healthcheck(self):
        self.healthchecked = True

    def create_trial(self, run_id, trial_id, manifest, channel_label=None):
        self.created = (run_id, trial_id, manifest, channel_label)
        return TrialHandle(
            run_id,
            trial_id,
            manifest.sha256,
            "ws://relay",
            "channel-1",
            (),
            user=AgentCredential(
                agent_id="user",
                role="user",
                nostr_secret_key="s",
                nostr_pubkey="p",
                nostr_auth_tag="[]",
                llm_endpoint="",
                llm_api_key="",
            ),
        )

    def teardown(self, handle):
        self.torn_down = handle


class Runtime:
    def __init__(self, error=None):
        self.called = None
        self.error = error

    async def run(self, **kwargs):
        self.called = kwargs
        if self.error:
            raise self.error
        return RuntimeResult(10, 2, 3, 0.25, {"receipt_status": "pending"})


async def test_agent_lifecycle_and_context(tmp_path, manifest_data):
    provisioner, runtime, context_id = Provisioner(), Runtime(), uuid4()
    environment = SimpleNamespace(
        context_id=context_id, environment_name="hello-world"
    )
    agent = BuzzOrchestraAgent(
        logs_dir=tmp_path,
        manifest=manifest_data,
        provisioner=provisioner,
        runtime=runtime,
        run_id="run-1",
    )
    agent.context_id = context_id
    context = AgentContext()
    await agent.setup(environment)
    await agent.run("solve it", environment, context)
    assert provisioner.healthchecked
    assert provisioner.created[:2] == ("run-1", str(context_id))
    # The task short name labels the trial channel for spectator GUIs.
    assert provisioner.created[3] == "hello-world"
    assert provisioner.torn_down.channel_id == "channel-1"
    assert runtime.called["instruction"] == "solve it"
    assert (
        context.n_input_tokens,
        context.n_cache_tokens,
        context.n_output_tokens,
        context.cost_usd,
    ) == (10, 2, 3, 0.25)
    assert context.metadata["manifest_sha256"] == agent.manifest.sha256
    assert context.metadata["trial_id"] == str(context_id)


async def test_teardown_runs_when_runtime_fails(tmp_path, manifest_data):
    provisioner, runtime, context_id = (
        Provisioner(),
        Runtime(RuntimeError("runtime failed")),
        uuid4(),
    )
    environment = SimpleNamespace(context_id=context_id)
    agent = BuzzOrchestraAgent(
        logs_dir=tmp_path,
        manifest=manifest_data,
        provisioner=provisioner,
        runtime=runtime,
    )
    agent.context_id = context_id
    with pytest.raises(RuntimeError, match="runtime failed"):
        await agent.run("solve it", environment, AgentContext())
    assert provisioner.torn_down.channel_id == "channel-1"


async def test_missing_integrations_fail_explicitly(tmp_path, manifest_data):
    agent = BuzzOrchestraAgent(logs_dir=tmp_path, manifest=manifest_data)
    with pytest.raises(RuntimeError, match="M1 wiring is incomplete"):
        await agent.run("solve it", SimpleNamespace(context_id=uuid4()), AgentContext())


async def test_cli_runtime_construction_from_json(tmp_path, manifest_data):
    endpoint_path = tmp_path / "endpoints.json"
    endpoint_path.write_text(
        '{"frontier/rev":{"provider":"anthropic",'
        '"api_key_env":"ANTHROPIC_API_KEY"},'
        '"worker/rev":{"provider":"openai",'
        '"api_key_env":"OPENAI_API_KEY"}}'
    )
    agent = BuzzOrchestraAgent(
        logs_dir=tmp_path / "logs",
        manifest=manifest_data,
        artifact_root=tmp_path,
        endpoint_config=endpoint_path,
        buzz_acp_binary="/pinned/buzz-acp",
        buzz_agent_binary="/pinned/buzz-agent",
        buzz_dev_mcp_binary="/pinned/buzz-dev-mcp",
        buzz_cli_binary="/pinned/buzz",
    )
    assert agent.runtime.artifact_root == tmp_path
    assert agent.runtime.endpoints["frontier/rev"].provider == "anthropic"
    assert agent.runtime.buzz_acp_binary == "/pinned/buzz-acp"
    assert agent.runtime.buzz_agent_binary == "/pinned/buzz-agent"
    assert agent.runtime.buzz_dev_mcp_binary == "/pinned/buzz-dev-mcp"
    assert agent.runtime.buzz_cli_binary == "/pinned/buzz"


async def test_cli_construction_requires_complete_pairs(tmp_path, manifest_data):
    with pytest.raises(ValueError, match="artifact_root"):
        BuzzOrchestraAgent(
            logs_dir=tmp_path,
            manifest=manifest_data,
            endpoint_config={},
        )
    with pytest.raises(ValueError, match="provisioner_factory"):
        BuzzOrchestraAgent(
            logs_dir=tmp_path,
            manifest=manifest_data,
            provisioner_config={},
        )
