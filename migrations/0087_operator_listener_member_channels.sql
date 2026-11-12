-- A registration's target-signed scope: false delivers only events that
-- p-tag the target; true also delivers channel events for channels the target
-- is a current member of. The constant default fills existing rows without a
-- table rewrite and keeps them mentions-only.
SET LOCAL lock_timeout = '5s';

ALTER TABLE operator_listener_pubkeys
    ADD COLUMN member_channels BOOLEAN NOT NULL DEFAULT false;

-- Member-channel enqueue starts from the few opted-in registrations and probes
-- channel_members by primary key, instead of walking every member of a busy
-- channel. Every existing row is false, so the index builds empty.
SET LOCAL statement_timeout = '5s';
DO $migration$
BEGIN
    IF to_regclass('public.operator_listener_pubkeys_member_channels') IS NULL THEN
        CREATE INDEX operator_listener_pubkeys_member_channels
            ON public.operator_listener_pubkeys (target_pubkey, listener_pubkey)
            WHERE member_channels;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        WHERE i.indexrelid = 'public.operator_listener_pubkeys_member_channels'::regclass
          AND i.indrelid = 'public.operator_listener_pubkeys'::regclass
          AND i.indisvalid AND i.indisready AND i.indislive
          AND NOT i.indisunique AND i.indnkeyatts = 2 AND i.indnatts = 2
          AND pg_get_indexdef(i.indexrelid,1,true) = 'target_pubkey'
          AND pg_get_indexdef(i.indexrelid,2,true) = 'listener_pubkey'
          AND i.indoption::text = '0 0'
          AND pg_get_expr(i.indpred, i.indrelid) = 'member_channels'
          AND (SELECT amname FROM pg_am WHERE oid=(SELECT relam FROM pg_class WHERE oid=i.indexrelid))='btree'
    ) THEN
        RAISE EXCEPTION 'operator_listener_pubkeys_member_channels invalid or wrong definition';
    END IF;
END $migration$;
