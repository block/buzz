-- Add optional per-channel avatar URL. The relay emits this as a `picture`
-- tag on kind:39000 group metadata so clients can render channel icons.
ALTER TABLE channels ADD COLUMN avatar_url TEXT;
