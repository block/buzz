-- Durable authority boundary for protected audio sessions.
--
-- The row records a bounded existing-member admission. It does not create
-- membership and cannot outlive the finalized access lease.

CREATE TABLE audio_session_admissions (
    community_id UUID NOT NULL REFERENCES communities(id) ON DELETE CASCADE,
    admission_id UUID NOT NULL,
    channel_id UUID NOT NULL,
    pubkey BYTEA NOT NULL CHECK (octet_length(pubkey) = 32),
    lease_expires_at TIMESTAMPTZ NOT NULL,
    admitted_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (community_id, admission_id),
    CONSTRAINT audio_session_admissions_channel_fk
        FOREIGN KEY (community_id, channel_id)
        REFERENCES channels(community_id, id) ON DELETE CASCADE,
    CONSTRAINT audio_session_admissions_positive_lease
        CHECK (lease_expires_at > admitted_at)
);

CREATE INDEX idx_audio_session_admissions_active
    ON audio_session_admissions (community_id, channel_id, lease_expires_at);
