"""Provisioner unit tests — no relay or Postgres required."""

from __future__ import annotations

import hashlib
import json

import coincurve
import pytest
from harbor_buzz_testbed.provisioner import (
    BuzzTrialProvisioner,
    ProvisioningError,
    TestbedConfig,
)

OWNER_SECRET = "0" * 63 + "3"


def config(**overrides) -> TestbedConfig:
    defaults = dict(
        relay_http_url="http://localhost:3000",
        relay_ws_url="ws://host.docker.internal:3000",
        owner_secret_key=OWNER_SECRET,
        postgres_dsn="postgresql://unused",
        llm_api_keys={"databricks/glm": "glm-key", "databricks/opus": "opus-key"},
    )
    defaults.update(overrides)
    return TestbedConfig(**defaults)


def test_mint_credentials_expands_roster(manifest):
    credentials = BuzzTrialProvisioner(config())._mint_credentials(manifest)
    assert [c.agent_id for c in credentials] == [
        "orch-opus-1",
        "worker-glm-1",
        "worker-glm-2",
    ]
    assert credentials[0].role == "orchestrator"
    assert credentials[0].llm_api_key == "opus-key"
    assert {c.llm_api_key for c in credentials[1:]} == {"glm-key"}


def test_mint_credentials_keys_are_fresh_and_attested(manifest):
    provisioner = BuzzTrialProvisioner(config())
    first = provisioner._mint_credentials(manifest)
    second = provisioner._mint_credentials(manifest)
    all_secrets = [c.nostr_secret_key for c in first + second]
    assert len(all_secrets) == len(set(all_secrets)), "keys must never be reused"
    owner_pubkey = coincurve.PrivateKey(bytes.fromhex(OWNER_SECRET)).public_key_xonly
    for credential in first:
        tag = json.loads(credential.nostr_auth_tag)
        assert tag[:3] == ["auth", owner_pubkey.format().hex(), ""]
        digest = hashlib.sha256(
            f"nostr:agent-auth:{credential.nostr_pubkey}:".encode()
        ).digest()
        assert owner_pubkey.verify(bytes.fromhex(tag[3]), digest)


def test_mint_user_is_attested_and_not_an_agent():
    provisioner = BuzzTrialProvisioner(config())
    user = provisioner._mint_user()
    assert user.agent_id == "user"
    assert user.role == "user"
    assert user.llm_endpoint == "" and user.llm_api_key == ""
    owner_pubkey = coincurve.PrivateKey(bytes.fromhex(OWNER_SECRET)).public_key_xonly
    tag = json.loads(user.nostr_auth_tag)
    assert tag[:3] == ["auth", owner_pubkey.format().hex(), ""]


def test_pinned_user_secret_reuses_one_identity():
    pinned = "7" * 64
    provisioner = BuzzTrialProvisioner(config(user_secret_key=pinned))
    first = provisioner._mint_user()
    second = provisioner._mint_user()
    assert first.nostr_secret_key == pinned
    assert first.nostr_pubkey == second.nostr_pubkey
    expected = coincurve.PrivateKey(bytes.fromhex(pinned)).public_key_xonly
    assert first.nostr_pubkey == expected.format().hex()
    # Still a user, still attested by the owner.
    assert first.role == "user"
    owner_pubkey = coincurve.PrivateKey(bytes.fromhex(OWNER_SECRET)).public_key_xonly
    assert json.loads(first.nostr_auth_tag)[1] == owner_pubkey.format().hex()


def test_teardown_skips_archiving_when_disabled():
    provisioner = BuzzTrialProvisioner(config(archive_on_teardown=False))
    # Any attribute access would fail on this handle — teardown must return
    # before touching the CLI or Postgres.
    provisioner.teardown(handle=None)


def test_mint_credentials_missing_api_key_is_explicit(manifest):
    provisioner = BuzzTrialProvisioner(config(llm_api_keys={}))
    with pytest.raises(ProvisioningError, match="databricks/"):
        provisioner._mint_credentials(manifest)


def test_lock_key_is_deterministic_and_distinct():
    calls: list[int] = []

    class FakeConn:
        def execute(self, _query, params):
            calls.append(params[0])

    BuzzTrialProvisioner._lock_trial(FakeConn(), "run-a", "trial-1")
    BuzzTrialProvisioner._lock_trial(FakeConn(), "run-a", "trial-1")
    BuzzTrialProvisioner._lock_trial(FakeConn(), "run-a", "trial-2")
    assert calls[0] == calls[1]
    assert calls[0] != calls[2]
    assert all(-(2**63) <= key < 2**63 for key in calls)


def test_healthcheck_fails_fast_when_relay_down():
    provisioner = BuzzTrialProvisioner(
        config(relay_http_url="http://localhost:1", postgres_dsn="postgresql://unused")
    )
    with pytest.raises(ProvisioningError, match="relay unreachable"):
        provisioner.healthcheck()
