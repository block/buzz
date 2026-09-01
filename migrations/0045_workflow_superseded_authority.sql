-- Supersession retains captured workflow authority; explicit deletion revokes it.
-- Do not infer a deletion reason for historical rows: unknown stays fail-closed.
ALTER TABLE events ADD COLUMN workflow_revision_superseded BOOLEAN NOT NULL DEFAULT false;
