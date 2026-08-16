-- Twilio SMS allow-list + phone->project routing default. `allowed` gates
-- whether an inbound SMS is admitted at all (closes the anonymous-spam /
-- oracle vector); `default_project` names a NIP-MP project `d`-tag the
-- sms-operator persona dispatches into when the number has no ambiguity.
-- The primary key leads with `community_id` because this is a tenant-scoped
-- table (it is not in `_operator_global_tables`), and the repo enforces that
-- shape. Keying on `phone_number` alone would also have made the number
-- globally unique across every community on the relay, so one community
-- registering a number would silently deny it to all the others.
CREATE TABLE sms_identities (
    community_id    UUID NOT NULL REFERENCES communities(id),
    phone_number    TEXT NOT NULL,
    allowed         BOOLEAN NOT NULL DEFAULT false,
    linked_pubkey   BYTEA,
    default_project TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, phone_number),
    CONSTRAINT chk_sms_identities_phone_number_e164
        CHECK (phone_number ~ '^\+[1-9][0-9]{1,14}$'),
    CONSTRAINT chk_sms_identities_linked_pubkey_len
        CHECK (linked_pubkey IS NULL OR octet_length(linked_pubkey) = 32)
);

-- The inbound webhook resolves a number before it knows the community (the
-- community comes *from* the matched row), so it queries on phone_number
-- alone and the composite PK's leading column cannot serve it.
CREATE INDEX idx_sms_identities_phone_number ON sms_identities (phone_number);
