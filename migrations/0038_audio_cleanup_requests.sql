-- Durable disconnect/cancellation intent for live audio attempts. A request is
-- witnessed before ephemeral cleanup begins, allowing another relay to finish
-- compensation immediately when the initiating process loses the race.

ALTER TABLE audio_session_admissions
    ADD COLUMN cleanup_requested_at TIMESTAMPTZ;

CREATE INDEX idx_audio_session_admissions_cleanup_requested
    ON audio_session_admissions (community_id, cleanup_requested_at)
    WHERE cleanup_requested_at IS NOT NULL
      AND state IN ('reserved', 'active');
