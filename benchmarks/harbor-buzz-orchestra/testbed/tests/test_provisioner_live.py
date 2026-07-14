"""Live provisioning tests against a running benchmark stack.

Gated by BUZZ_TESTBED_LIVE=1 with stack coordinates in the environment:
  BUZZ_TESTBED_RELAY_HTTP   (default http://localhost:3000)
  BUZZ_TESTBED_RELAY_WS     (default ws://host.docker.internal:3000)
  BUZZ_TESTBED_OWNER_KEY    relay owner secret key (hex)
  BUZZ_TESTBED_PG_DSN       benchmark Postgres DSN
"""

from __future__ import annotations

import os
import uuid

import psycopg
import pytest
from harbor_buzz_testbed.buzz_cli import BuzzCli, BuzzCliError
from harbor_buzz_testbed.provisioner import (
    BuzzTrialProvisioner,
    ProvisioningError,
    TestbedConfig,
)

pytestmark = pytest.mark.skipif(
    os.environ.get("BUZZ_TESTBED_LIVE") != "1",
    reason="live testbed suite; set BUZZ_TESTBED_LIVE=1 with a running stack",
)


@pytest.fixture()
def provisioner() -> BuzzTrialProvisioner:
    owner_key = os.environ.get("BUZZ_TESTBED_OWNER_KEY")
    dsn = os.environ.get("BUZZ_TESTBED_PG_DSN")
    if not owner_key or not dsn:
        pytest.fail("BUZZ_TESTBED_OWNER_KEY and BUZZ_TESTBED_PG_DSN are required")
    return BuzzTrialProvisioner(
        TestbedConfig(
            relay_http_url=os.environ.get(
                "BUZZ_TESTBED_RELAY_HTTP", "http://localhost:3000"
            ),
            relay_ws_url=os.environ.get(
                "BUZZ_TESTBED_RELAY_WS", "ws://host.docker.internal:3000"
            ),
            owner_secret_key=owner_key,
            postgres_dsn=dsn,
            llm_api_keys={
                "databricks/glm": "test-glm-key",
                "databricks/opus": "test-opus-key",
            },
        )
    )


def cli_for(provisioner: BuzzTrialProvisioner, credential) -> BuzzCli:
    return provisioner._cli_for(credential)


def test_create_is_idempotent_and_isolated(provisioner, manifest):
    run_id = f"live-{uuid.uuid4().hex[:8]}"
    trial_a = str(uuid.uuid4())
    trial_b = str(uuid.uuid4())

    provisioner.healthcheck()
    handle_a = provisioner.create_trial(run_id, trial_a, manifest)
    handle_b = provisioner.create_trial(run_id, trial_b, manifest)
    try:
        # Shape: one credential per roster slot, orchestrator first.
        assert len(handle_a.credentials) == 3
        assert handle_a.credentials[0].role == "orchestrator"
        assert handle_a.manifest_hash == manifest.sha256

        # Idempotency: same key returns the stored handle, keys included.
        again = provisioner.create_trial(run_id, trial_a, manifest)
        assert again == handle_a

        # Isolation: trials get distinct channels and fresh keys...
        assert handle_a.channel_id != handle_b.channel_id
        keys_a = {c.nostr_secret_key for c in handle_a.credentials}
        keys_b = {c.nostr_secret_key for c in handle_b.credentials}
        assert not keys_a & keys_b

        # ...and trial B's orchestrator cannot see trial A's channel.
        cli_a = cli_for(provisioner, handle_a.credentials[0])
        cli_b = cli_for(provisioner, handle_b.credentials[0])
        cli_a.run(
            "messages",
            "send",
            "--channel",
            handle_a.channel_id,
            "--content",
            "trial A secret",
        )
        foreign_read = cli_b.run(
            "messages", "get", "--channel", handle_a.channel_id, "--limit", "10"
        )
        assert foreign_read == [], "cross-trial read must return nothing"
        with pytest.raises(BuzzCliError, match="private"):
            cli_b.run("channels", "join", "--channel", handle_a.channel_id)

        # Members can read their own channel.
        own_read = cli_a.run(
            "messages", "get", "--channel", handle_a.channel_id, "--limit", "10"
        )
        assert [m["content"] for m in own_read] == ["trial A secret"]

        # Same trial key + different manifest must be rejected, not silently
        # reprovisioned.
        altered = manifest.model_copy(update={"condition": "O-altered"})
        with pytest.raises(ProvisioningError, match="already provisioned"):
            provisioner.create_trial(run_id, trial_a, altered)
    finally:
        provisioner.teardown(handle_a)
        provisioner.teardown(handle_b)

    # Teardown is idempotent and stamps archived_at.
    provisioner.teardown(handle_a)
    with psycopg.connect(os.environ["BUZZ_TESTBED_PG_DSN"]) as conn:
        row = conn.execute(
            "SELECT archived_at FROM benchmark.trial_manifest"
            " WHERE run_id = %s AND trial_id = %s",
            (run_id, trial_a),
        ).fetchone()
    assert row is not None and row[0] is not None
