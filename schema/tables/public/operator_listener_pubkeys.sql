-- ── Operator-listener mention delivery ──────────────────────────────────────
-- Listener registrations are deployment-global. The outbox records community
-- provenance for the event, but is intentionally not tenant-owned.

CREATE TABLE operator_listener_pubkeys (
    listener_pubkey BYTEA NOT NULL CHECK (length(listener_pubkey) = 32),
    target_pubkey   BYTEA NOT NULL CHECK (length(target_pubkey) = 32),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (listener_pubkey, target_pubkey)
);
CREATE INDEX operator_listener_pubkeys_target
    ON operator_listener_pubkeys (target_pubkey, listener_pubkey);
CREATE INDEX operator_listener_pubkeys_created_at
    ON operator_listener_pubkeys (created_at);
