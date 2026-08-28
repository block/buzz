-- Bind durable human approvals to exactly one workflow task.
-- Legacy approvals remain valid with task_id NULL.
ALTER TABLE workflow_approvals
    ADD COLUMN task_id UUID,
    ADD COLUMN request_message TEXT;

ALTER TABLE workflow_approvals
    ADD CONSTRAINT fk_workflow_approval_task
    FOREIGN KEY (community_id, run_id, task_id)
    REFERENCES workflow_run_tasks (community_id, run_id, id)
    ON DELETE CASCADE;

CREATE UNIQUE INDEX idx_workflow_approvals_task
    ON workflow_approvals (community_id, run_id, task_id)
    WHERE task_id IS NOT NULL;
