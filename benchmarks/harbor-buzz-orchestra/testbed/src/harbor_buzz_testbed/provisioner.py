"""Channel-per-trial Buzz provisioning against a local benchmark stack."""

from __future__ import annotations

import dataclasses
import hashlib
import json
import urllib.error
import urllib.request
from dataclasses import dataclass

import psycopg
from harbor_buzz_orchestra.manifest import ExperimentManifest
from harbor_buzz_orchestra.provisioning import AgentCredential, TrialHandle

from .buzz_cli import BuzzCli
from .keys import compute_auth_tag, generate_keypair, keypair_from_secret


class ProvisioningError(RuntimeError):
    """Trial provisioning failed or was invoked inconsistently."""


@dataclass(frozen=True, slots=True)
class TestbedConfig:
    """Connection settings for one local benchmark stack."""

    __test__ = False  # not a pytest class, despite the name
    relay_http_url: str  # e.g. http://localhost:3000 — CLI + healthcheck
    relay_ws_url: str  # as reachable FROM the agents' runtime
    owner_secret_key: str  # relay owner key; signs NIP-OA attestations
    postgres_dsn: str  # benchmark schema lives here
    llm_api_keys: dict[str, str] = dataclasses.field(default_factory=dict)
    # endpoint -> API key. v1: per-endpoint resolution; true per-agent
    # Databricks attribution is pending the AI Gateway field verification.
    # Pinned user identity (hex secret key). When set, every trial's user —
    # the human analogue that owns the channel and posts the task — is this
    # one identity instead of a fresh key per trial, so a GUI logged in as
    # that user sees every trial channel accumulate. None preserves the
    # fresh-user-per-trial behaviour.
    user_secret_key: str | None = None
    # When False, teardown leaves the trial channel unarchived (and its
    # archived_at stamp NULL) so finished trials stay visible in a GUI.
    archive_on_teardown: bool = True


def provisioner_from_dict(config: dict[str, object]) -> "BuzzTrialProvisioner":
    """Harbor CLI factory for a JSON-decoded testbed configuration."""
    return BuzzTrialProvisioner(TestbedConfig(**config))


class BuzzTrialProvisioner:
    """Implements the TrialHandle v1.1 contract against a live Buzz relay.

    Guarantees (contract PLANS/HARBOR_BUZZ_TRIALHANDLE_CONTRACT.md):
    - create_trial is synchronous and idempotent per (run_id, trial_id);
      concurrency-safe via a Postgres advisory lock on the trial key.
    - One private channel per trial; membership is exactly the trial's
      credentials, so cross-trial reads are blocked by construction.
    - Fresh keys per trial; never reused.
    - teardown archives the channel and stamps archived_at; events are
      never deleted.
    """

    def __init__(self, config: TestbedConfig) -> None:
        self._config = config

    # -- contract surface ---------------------------------------------------

    def create_trial(
        self,
        run_id: str,
        trial_id: str,
        manifest: ExperimentManifest,
        channel_label: str | None = None,
    ) -> TrialHandle:
        manifest_hash = manifest.sha256
        with psycopg.connect(self._config.postgres_dsn) as conn:
            self._lock_trial(conn, run_id, trial_id)
            existing = self._load_trial(conn, run_id, trial_id)
            if existing is not None:
                if existing.manifest_hash != manifest_hash:
                    raise ProvisioningError(
                        f"trial ({run_id}, {trial_id}) already provisioned with "
                        f"manifest {existing.manifest_hash}, got {manifest_hash}"
                    )
                return existing

            handle = self._provision(
                run_id, trial_id, manifest, manifest_hash, channel_label
            )
            self._store_trial(conn, handle)
            conn.commit()
            return handle

    def teardown(self, handle: TrialHandle) -> None:
        if not self._config.archive_on_teardown:
            # GUI/spectator mode: finished trial channels stay visible.
            return
        cli = self._cli_for(handle.credentials[0])
        try:
            cli.archive_channel(handle.channel_id)
        except Exception as error:  # noqa: BLE001 — idempotent re-teardown
            if "archived" not in str(error).lower():
                raise
        with psycopg.connect(self._config.postgres_dsn) as conn:
            conn.execute(
                "UPDATE benchmark.trial_manifest"
                " SET archived_at = COALESCE(archived_at, now())"
                " WHERE run_id = %s AND trial_id = %s",
                (handle.run_id, handle.trial_id),
            )
            conn.commit()

    def healthcheck(self) -> None:
        url = f"{self._config.relay_http_url.rstrip('/')}/_readiness"
        try:
            with urllib.request.urlopen(url, timeout=5) as response:
                if response.status != 200:
                    raise ProvisioningError(
                        f"relay readiness returned {response.status}"
                    )
        except (urllib.error.URLError, OSError) as error:
            raise ProvisioningError(f"relay unreachable at {url}: {error}") from error
        try:
            with psycopg.connect(self._config.postgres_dsn, connect_timeout=5) as conn:
                conn.execute("SELECT 1")
        except psycopg.Error as error:
            raise ProvisioningError(
                f"benchmark postgres unreachable: {error}"
            ) from error

    # -- internals ----------------------------------------------------------

    def _provision(
        self,
        run_id: str,
        trial_id: str,
        manifest: ExperimentManifest,
        manifest_hash: str,
        channel_label: str | None,
    ) -> TrialHandle:
        credentials = self._mint_credentials(manifest)
        user = self._mint_user()
        # The user identity creates the channel and invites the agents —
        # mirroring production Buzz, where a human owns the channel their
        # agents work in.
        cli = self._cli_for(user)
        name = (
            f"{channel_label}-{trial_id[:8]}"
            if channel_label
            else f"trial-{trial_id[:8]}-{manifest_hash[:8]}"
        )
        channel_id = cli.create_private_channel(
            name=name,
            description=f"run={run_id} trial={trial_id} manifest={manifest_hash}",
        )
        for credential in credentials:
            cli.add_member(channel_id, credential.nostr_pubkey)
        return TrialHandle(
            run_id=run_id,
            trial_id=trial_id,
            manifest_hash=manifest_hash,
            relay_ws_url=self._config.relay_ws_url,
            channel_id=channel_id,
            credentials=credentials,
            user=user,
            user_relay_url=self._config.relay_http_url,
        )

    def _mint_credentials(
        self, manifest: ExperimentManifest
    ) -> tuple[AgentCredential, ...]:
        roster = sorted(manifest.roster, key=lambda e: e.kind != "orchestrator")
        credentials: list[AgentCredential] = []
        for entry in roster:
            api_key = self._config.llm_api_keys.get(entry.endpoint)
            if api_key is None:
                raise ProvisioningError(
                    f"no LLM API key configured for endpoint {entry.endpoint!r}"
                )
            for index in range(1, entry.count + 1):
                keypair = generate_keypair()
                credentials.append(
                    AgentCredential(
                        agent_id=f"{entry.id}-{index}",
                        role=entry.kind,
                        nostr_secret_key=keypair.secret_key,
                        nostr_pubkey=keypair.pubkey,
                        nostr_auth_tag=compute_auth_tag(
                            self._config.owner_secret_key, keypair.pubkey
                        ),
                        llm_endpoint=entry.endpoint,
                        llm_api_key=api_key,
                    )
                )
        return tuple(credentials)

    def _mint_user(self) -> AgentCredential:
        """Mint the trial's user identity — the human analogue, not an agent.

        With a pinned ``user_secret_key`` the same identity fronts every
        trial, like one human running many teams; otherwise each trial gets
        a fresh user key.
        """
        keypair = (
            keypair_from_secret(self._config.user_secret_key)
            if self._config.user_secret_key
            else generate_keypair()
        )
        return AgentCredential(
            agent_id="user",
            role="user",
            nostr_secret_key=keypair.secret_key,
            nostr_pubkey=keypair.pubkey,
            nostr_auth_tag=compute_auth_tag(
                self._config.owner_secret_key, keypair.pubkey
            ),
            llm_endpoint="",
            llm_api_key="",
        )

    def _cli_for(self, credential: AgentCredential) -> BuzzCli:
        return BuzzCli(
            relay_url=self._config.relay_http_url,
            secret_key=credential.nostr_secret_key,
            auth_tag=credential.nostr_auth_tag,
        )

    @staticmethod
    def _lock_trial(conn: psycopg.Connection, run_id: str, trial_id: str) -> None:
        digest = hashlib.sha256(f"{run_id}\x00{trial_id}".encode()).digest()
        lock_key = int.from_bytes(digest[:8], "big", signed=True)
        conn.execute("SELECT pg_advisory_xact_lock(%s)", (lock_key,))

    @staticmethod
    def _load_trial(
        conn: psycopg.Connection, run_id: str, trial_id: str
    ) -> TrialHandle | None:
        row = conn.execute(
            "SELECT handle FROM benchmark.trial_manifest"
            " WHERE run_id = %s AND trial_id = %s",
            (run_id, trial_id),
        ).fetchone()
        if row is None:
            return None
        stored = row[0]
        return TrialHandle(
            run_id=stored["run_id"],
            trial_id=stored["trial_id"],
            manifest_hash=stored["manifest_hash"],
            relay_ws_url=stored["relay_ws_url"],
            channel_id=stored["channel_id"],
            credentials=tuple(
                AgentCredential(**credential) for credential in stored["credentials"]
            ),
            user=AgentCredential(**stored["user"]),
            user_relay_url=stored.get("user_relay_url", ""),
        )

    @staticmethod
    def _store_trial(conn: psycopg.Connection, handle: TrialHandle) -> None:
        conn.execute(
            "INSERT INTO benchmark.trial_manifest"
            " (run_id, trial_id, manifest_hash, channel_id, handle)"
            " VALUES (%s, %s, %s, %s, %s)",
            (
                handle.run_id,
                handle.trial_id,
                handle.manifest_hash,
                handle.channel_id,
                json.dumps(dataclasses.asdict(handle)),
            ),
        )
