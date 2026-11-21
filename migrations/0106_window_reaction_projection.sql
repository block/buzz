-- Old writers and legacy signed events use the auxiliary reference index.
SET LOCAL lock_timeout = '1s';
SET LOCAL statement_timeout = '5s';

ALTER TABLE public.events ADD COLUMN reaction_read_projected BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE public.reactions ADD COLUMN reaction_event_created_at TIMESTAMPTZ;
