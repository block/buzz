-- Kind-0 `name` projected separately from `display_name`.
--
-- The kind-0 side effect used to collapse `display_name` and `name` into the
-- single `users.display_name` column, so a profile search backed by `users`
-- could not tell which field matched and lost `name` whenever `display_name`
-- was also set. `users.name` holds the raw kind-0 `name`; `display_name` now
-- holds `display_name` (falling back to `displayName`) only.
--
-- Metadata-only ALTER on a small table: adding a nullable column without a
-- default rewrites nothing. Existing rows are populated by
-- `buzz-admin backfill-user-profiles`; see docs/profile-search-users-table.md.

ALTER TABLE users ADD COLUMN name VARCHAR(255);
