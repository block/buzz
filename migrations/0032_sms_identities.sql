-- Twilio SMS allow-list + phone->project routing default. `allowed` gates
-- whether an inbound SMS is admitted at all (closes the anonymous-spam /
-- oracle vector); `default_project` names a NIP-MP project `d`-tag the
-- sms-operator persona dispatches into when the number has no ambiguity.
CREATE TABLE sms_identities (
    phone_number    TEXT PRIMARY KEY,
    community_id    UUID NOT NULL REFERENCES communities(id),
    allowed         BOOLEAN NOT NULL DEFAULT false,
    linked_pubkey   BYTEA,
    default_project TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_sms_identities_phone_number_e164
        CHECK (phone_number ~ '^\+[1-9][0-9]{1,14}$'),
    CONSTRAINT chk_sms_identities_linked_pubkey_len
        CHECK (linked_pubkey IS NULL OR octet_length(linked_pubkey) = 32)
);

CREATE INDEX idx_sms_identities_community ON sms_identities (community_id);
