-- started_at is the actor's first read intent. NULL until then: an account
-- can exist before its actor has started. Read positions are floored at it,
-- so a started account begins caught up and an unstarted one counts nothing.
ALTER TABLE personal_read_accounts ADD COLUMN started_at TIMESTAMPTZ;
