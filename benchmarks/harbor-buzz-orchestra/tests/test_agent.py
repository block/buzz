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

    def create_trial(self, run_id, trial_id, manifest):
        self.created = (run_id, trial_id, manifest)
        return TrialHandle(
            run_id, trial_id, manifest.sha256, "ws://relay", "channel-1", ()
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
    environment = SimpleNamespace(context_id=context_id)
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
