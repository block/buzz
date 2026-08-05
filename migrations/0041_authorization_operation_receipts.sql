-- Transaction-owned protected-operation idempotency.
--
-- These receipts are part of the mutation commit protocol. They are not an
-- authorization decision log or operator audit trail: a receipt exists only
-- when the protected mutation committed in the same transaction.

CREATE TABLE authorization_operation_receipts (
    community_id        UUID NOT NULL REFERENCES communities(id),
    operation_id        UUID NOT NULL,
    operation_kind      TEXT NOT NULL CHECK (
        length(operation_kind) > 0 AND length(operation_kind) <= 128
    ),
    request_fingerprint BYTEA NOT NULL CHECK (length(request_fingerprint) = 32),
    result_version      SMALLINT NOT NULL DEFAULT 1 CHECK (result_version > 0),
    result_payload      BYTEA NOT NULL CHECK (octet_length(result_payload) <= 65536),
    lease_expires_at    TIMESTAMPTZ NOT NULL,
    committed_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, operation_id)
);

CREATE INDEX idx_authorization_operation_receipts_committed_at
    ON authorization_operation_receipts (community_id, committed_at);

-- The transaction rechecks expiry before inserting its receipt, and this
-- deferred trigger closes the final interval between that check and COMMIT.
CREATE FUNCTION authorization_operation_expiry_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.lease_expires_at <= clock_timestamp() THEN
        RAISE EXCEPTION 'protected operation authorization expired before commit'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER authorization_operation_expiry
    AFTER INSERT OR UPDATE OF lease_expires_at
    ON authorization_operation_receipts
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION authorization_operation_expiry_guard();
