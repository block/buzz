-- Prevent a workflow from starting more than one run for the same persisted
-- reply parent. This is deliberately general rather than outreach-specific:
-- reply-triggered workflows must not repeat their side effects when the same
-- event is replayed or reprocessed.

DROP INDEX IF EXISTS uq_outreach_stage_b_reply_parent;

CREATE UNIQUE INDEX IF NOT EXISTS uq_workflow_runs_reply_parent
    ON workflow_runs (
        community_id,
        workflow_id,
        ((trigger_context ->> 'reply_to_message_id'))
    )
    WHERE NULLIF(trigger_context ->> 'reply_to_message_id', '') IS NOT NULL;
