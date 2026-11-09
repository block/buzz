-- Operator identity retirement (docs/operator-identity-retirement.md).
-- Expand only: the deployed relay never writes or reads the new tables, and
-- the widened consent CHECK admits every row it can write. Validated in 0088
-- so the scan does not retain this transaction's DDL lock.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- NIP-IA `consent=relay`: archived by an operator action, not a signed request.
-- Such rows store '' in request_event_id; the column stays NOT NULL because
-- the deployed relay decodes it as a non-null string.
ALTER TABLE archived_identities
    DROP CONSTRAINT archived_identities_consent_path_check,
    ADD CONSTRAINT archived_identities_consent_path_check
        CHECK (consent_path IN ('self', 'owner', 'admin', 'relay')) NOT VALID;

-- One row per accepted operator request: audit, idempotency and result.
-- All writers use community admission; no fence trigger.
CREATE TABLE identity_retirement_actions (
    community_id    UUID NOT NULL,
    id              UUID NOT NULL,
    request_id      UUID NOT NULL,
    operation       TEXT NOT NULL CHECK (operation IN ('archive', 'unarchive', 'release')),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    actor_pubkey    BYTEA NOT NULL CHECK (length(actor_pubkey) = 32),
    request         JSONB NOT NULL,
    result          JSONB NOT NULL,
    published_at    TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, id),
    UNIQUE (community_id, request_id)
);

-- Durable execution hold, independent of NIP-IA visibility. Run admission
-- refuses a workflow whose owner, or whose owner's owner, has a row here.
CREATE TABLE identity_retirement_holds (
    community_id    UUID NOT NULL,
    pubkey          BYTEA NOT NULL CHECK (length(pubkey) = 32),
    action_id       UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, pubkey)
);

-- Workflows a retirement moved from active to archived, with the updated_at it
-- wrote, so release restores only workflows unchanged since then.
CREATE TABLE identity_retirement_workflows (
    community_id    UUID NOT NULL,
    workflow_id     UUID NOT NULL,
    owner_pubkey    BYTEA NOT NULL CHECK (length(owner_pubkey) = 32),
    action_id       UUID NOT NULL,
    archived_at     TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (community_id, workflow_id)
);

CREATE INDEX idx_identity_retirement_workflows_owner
    ON identity_retirement_workflows (community_id, owner_pubkey);
