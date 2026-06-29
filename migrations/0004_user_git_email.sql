-- Add git_email column to users table for agent commit trailer injection.
--
-- Populated from the standard NIP-01 kind:0 `email` field when a user
-- publishes a profile event. Agents read this from `[Context]` as
-- `Requester-Git-Email:` and use it for Co-authored-by / Signed-off-by
-- trailers instead of falling back to `git config user.email`.
--
-- Nullable: users who have not set an email in their profile leave this
-- NULL, and the harness omits the `Requester-Git-Email:` line entirely.

ALTER TABLE users ADD COLUMN git_email TEXT;
