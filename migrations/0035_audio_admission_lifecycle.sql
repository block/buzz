-- Durable lifecycle for PostgreSQL-authorized audio attachment attempts.
--
-- These rows authorize bounded attempts; they never assert live presence or
-- create membership. Existing one-shot receipts are closed during upgrade.

ALTER TABLE audio_session_admissions
    ADD COLUMN state TEXT,
    ADD COLUMN state_version BIGINT,
    ADD COLUMN updated_at TIMESTAMPTZ,
    ADD COLUMN activated_at TIMESTAMPTZ,
    ADD COLUMN aborted_at TIMESTAMPTZ,
    ADD COLUMN finished_at TIMESTAMPTZ,
    ADD COLUMN failure_code TEXT;

UPDATE audio_session_admissions
SET state = 'aborted',
    state_version = 1,
    updated_at = admitted_at,
    aborted_at = admitted_at,
    failure_code = 'upgrade_closed';

ALTER TABLE audio_session_admissions
    ALTER COLUMN state SET DEFAULT 'reserved',
    ALTER COLUMN state SET NOT NULL,
    ALTER COLUMN state_version SET DEFAULT 1,
    ALTER COLUMN state_version SET NOT NULL,
    ALTER COLUMN updated_at SET DEFAULT clock_timestamp(),
    ALTER COLUMN updated_at SET NOT NULL,
    ADD CONSTRAINT audio_session_admissions_state
        CHECK (state IN ('reserved', 'active', 'aborted', 'finished')),
    ADD CONSTRAINT audio_session_admissions_state_version
        CHECK (state_version > 0),
    ADD CONSTRAINT audio_session_admissions_failure_code
        CHECK (failure_code IS NULL OR
               (length(failure_code) BETWEEN 1 AND 64 AND
                failure_code ~ '^[a-z0-9_]+$'));

CREATE INDEX idx_audio_session_admissions_reconcile
    ON audio_session_admissions (state, updated_at, lease_expires_at);
