-- Review-feedback wake ledger. One row per accepted wake, never more than one
-- per (author, feedback event): a retry of the same feedback revision, even with
-- a fresh timestamp after the relay freshness window refused the first attempt,
-- is acknowledged as a duplicate instead of waking the agent twice.
--
-- Rows exist independent of event retention, like artifact_revisions, so a
-- redacted or expired wake can never be re-minted. They are NOT time-pruned
-- (pruning would reopen duplicate wakes). Growth is bounded 1:1 by accepted
-- feedback events: each wake requires an accepted synaxis.artifact-feedback
-- revision signed by the same author. The table is purged with its community.
CREATE TABLE artifact_feedback_wakes (
    community_id UUID NOT NULL REFERENCES communities(id),
    author_pubkey BYTEA NOT NULL CHECK (length(author_pubkey) = 32),
    feedback_event_id BYTEA NOT NULL CHECK (length(feedback_event_id) = 32),
    wake_event_id BYTEA NOT NULL CHECK (length(wake_event_id) = 32),
    channel_id UUID NOT NULL,
    feedback_artifact_id UUID NOT NULL,
    reviewed_artifact_id UUID NOT NULL,
    reviewed_event_id BYTEA NOT NULL CHECK (length(reviewed_event_id) = 32),
    root_event_id BYTEA NOT NULL CHECK (length(root_event_id) = 32),
    parent_event_id BYTEA NOT NULL CHECK (length(parent_event_id) = 32),
    agent_pubkey BYTEA NOT NULL CHECK (length(agent_pubkey) = 32),
    accepted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (community_id, author_pubkey, feedback_event_id)
);
CREATE UNIQUE INDEX artifact_feedback_wakes_wake ON artifact_feedback_wakes (community_id, wake_event_id);

SELECT attach_community_write_fence('artifact_feedback_wakes');
