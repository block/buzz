-- Distinguish a legacy reservation whose pointer must exist from an Enforce
-- announcement that intentionally starts unpublished and takes its first push
-- only after PostgreSQL cutover. Existing reservations are conservatively
-- classified as legacy; migration must fail on a missing legacy pointer.
ALTER TABLE git_repo_names
    ADD COLUMN publication_origin TEXT NOT NULL DEFAULT 'legacy'
    CHECK (publication_origin IN ('legacy', 'protected_unpublished'));
