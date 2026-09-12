ALTER TABLE communities
ADD COLUMN thread_replies_in_channel BOOLEAN NOT NULL DEFAULT FALSE;
