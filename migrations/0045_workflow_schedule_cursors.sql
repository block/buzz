-- Durable scheduler progress, separate from fire claims. Existing workflows
-- start at deployment time so rollout never replays historical schedules.
SET LOCAL lock_timeout = '5s';

CREATE TABLE workflow_schedule_cursors (
    community_id      UUID NOT NULL,
    workflow_id       UUID NOT NULL,
    evaluated_through TIMESTAMPTZ NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (community_id, workflow_id),
    FOREIGN KEY (community_id, workflow_id)
        REFERENCES workflows (community_id, id) ON DELETE CASCADE
);

INSERT INTO workflow_schedule_cursors (community_id, workflow_id, evaluated_through)
SELECT community_id, id, NOW()
FROM workflows
WHERE definition->'trigger'->>'on' = 'schedule';

CREATE OR REPLACE FUNCTION reset_workflow_schedule_cursor()
RETURNS TRIGGER AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND OLD.definition IS NOT DISTINCT FROM NEW.definition
       AND OLD.enabled IS NOT DISTINCT FROM NEW.enabled
       AND OLD.status IS NOT DISTINCT FROM NEW.status THEN
        RETURN NEW;
    END IF;

    IF NEW.definition->'trigger'->>'on' = 'schedule' THEN
        INSERT INTO workflow_schedule_cursors
            (community_id, workflow_id, evaluated_through, updated_at)
        VALUES (NEW.community_id, NEW.id, NOW(), NOW())
        ON CONFLICT (community_id, workflow_id) DO UPDATE
        SET evaluated_through = EXCLUDED.evaluated_through,
            updated_at = EXCLUDED.updated_at;
    ELSE
        DELETE FROM workflow_schedule_cursors
        WHERE community_id = NEW.community_id AND workflow_id = NEW.id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_workflows_reset_schedule_cursor
AFTER INSERT OR UPDATE OF definition, enabled, status ON workflows
FOR EACH ROW EXECUTE FUNCTION reset_workflow_schedule_cursor();

SELECT attach_community_write_fence('workflow_schedule_cursors');
