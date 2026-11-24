-- Nullable fields preserve compatibility with older serving writers.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
ALTER TABLE community_serving_write_leases
    ADD COLUMN actor_pubkey BYTEA CHECK (actor_pubkey IS NULL OR octet_length(actor_pubkey) = 32),
    ADD COLUMN actor_owner_pubkey BYTEA CHECK (actor_owner_pubkey IS NULL OR octet_length(actor_owner_pubkey) = 32),
    ADD COLUMN media_lease_until TIMESTAMPTZ,
    ADD COLUMN media_write_started BOOLEAN NOT NULL DEFAULT false,
    ADD CHECK ((actor_pubkey IS NULL) = (media_lease_until IS NULL));
