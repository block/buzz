-- Retain the exact current revision superseded by a withdrawal so every
-- authenticated connection that displayed that author can receive a
-- strictly newer opaque withdrawal. This remains server-side reconciliation
-- state and is never serialized into the client projection.

ALTER TABLE client_status_revisions
    ADD COLUMN supersedes_revision BIGINT;

UPDATE client_status_revisions
SET supersedes_revision = revision - 1
WHERE disposition = 'withdrawn';

ALTER TABLE client_status_revisions
    ADD CONSTRAINT client_status_revisions_withdrawal CHECK (
        (disposition = 'current' AND supersedes_revision IS NULL)
        OR
        (disposition = 'withdrawn'
            AND supersedes_revision IS NOT NULL
            AND supersedes_revision > 0
            AND revision > supersedes_revision)
    );
