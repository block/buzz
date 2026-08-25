-- Deduplicate workflow webhook retries without retaining caller-provided keys
-- or request bodies. Existing and non-webhook runs keep these columns NULL.
SET LOCAL lock_timeout = '5s';

ALTER TABLE workflow_runs
    ADD COLUMN webhook_idempotency_key_hash BYTEA,
    ADD COLUMN webhook_payload_hash BYTEA,
    ADD COLUMN webhook_execution_claimed_at TIMESTAMPTZ,
    ADD COLUMN webhook_execution_claim_token UUID,
    ADD CONSTRAINT workflow_runs_webhook_idempotency_hashes
        CHECK (
            (
                webhook_idempotency_key_hash IS NULL
                AND webhook_payload_hash IS NULL
                AND webhook_execution_claimed_at IS NULL
                AND webhook_execution_claim_token IS NULL
            )
            OR (
                webhook_idempotency_key_hash IS NOT NULL
                AND webhook_payload_hash IS NOT NULL
                AND octet_length(webhook_idempotency_key_hash) = 32
                AND octet_length(webhook_payload_hash) = 32
                AND (
                    (
                        webhook_execution_claimed_at IS NULL
                        AND webhook_execution_claim_token IS NULL
                    )
                    OR (
                        webhook_execution_claimed_at IS NOT NULL
                        AND webhook_execution_claim_token IS NOT NULL
                    )
                )
            )
        );

CREATE UNIQUE INDEX idx_workflow_runs_webhook_idempotency
    ON workflow_runs (community_id, workflow_id, webhook_idempotency_key_hash)
    WHERE webhook_idempotency_key_hash IS NOT NULL;
