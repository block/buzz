-- Fenced outbox delivery: add a per-claim opaque token to relay_admin_outbox.
--
-- Without this, mark_outbox_delivered and fail_outbox_row only fence on row ID,
-- which allows a stale worker (whose lease already expired) to overwrite a newer
-- worker's terminal update. The claim token is a real ownership fence: completion
-- and failure updates require the token written at claim time.

ALTER TABLE relay_admin_outbox
    ADD COLUMN outbox_claim_token UUID;
