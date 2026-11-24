-- A delegated stage can advance while its external request is outstanding.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';
ALTER TABLE entity_deletion_stage_history
    DROP CONSTRAINT entity_deletion_stage_history_status_check,
    ADD CONSTRAINT entity_deletion_stage_history_status_check
        CHECK (status IN ('pending', 'running', 'failed', 'submitted', 'completed'));
