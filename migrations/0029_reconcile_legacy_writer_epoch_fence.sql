-- Reconcile deployments that recorded the pre-config writer-fence migration.
--
-- Older deployments recorded migration 0027 before the server-side config row
-- was added.  They have the same epoch/lease authority and trigger shape, but
-- their guard still reads buzz.writer_fence_required from the session.  This
-- additive migration makes the deployed catalog match the current 0027
-- contract.  It is intentionally idempotent so fresh installs also pass
-- through the same canonical definition.

-- A legacy v27 guard reads this session setting.  The migration-admin session
-- is allowed to disable that legacy check only for this transaction; the
-- canonical guard below ignores this setting and reads the server-side row.
SET LOCAL buzz.writer_fence_required = 'off';

CREATE TABLE IF NOT EXISTS buzz_writer_fence_config (
    singleton   BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    required    BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ NOT NULL
);

INSERT INTO _operator_global_tables (table_name, reason)
SELECT 'buzz_writer_fence_config', 'deployment-global writer-fence enforcement mode'
 WHERE NOT EXISTS (
    SELECT 1 FROM _operator_global_tables
     WHERE table_name = 'buzz_writer_fence_config'
 );

INSERT INTO buzz_writer_fence_config (singleton, required, updated_at)
VALUES (TRUE, FALSE, clock_timestamp())
ON CONFLICT (singleton) DO NOTHING;

REVOKE ALL ON buzz_writer_fence_config FROM PUBLIC;

CREATE OR REPLACE FUNCTION buzz_writer_fence_guard() RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    required BOOLEAN;
    resource TEXT := NULLIF(current_setting('buzz.writer_fence_resource', true), '');
    epoch_text TEXT := NULLIF(current_setting('buzz.writer_fence_epoch', true), '');
    holder_id TEXT := NULLIF(current_setting('buzz.writer_fence_holder', true), '');
BEGIN
    SELECT COALESCE(
        (SELECT c.required
           FROM buzz_writer_fence_config AS c
          WHERE c.singleton),
        TRUE
    )
    INTO required;

    IF NOT required THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        ELSIF TG_OP = 'TRUNCATE' THEN
            RETURN NULL;
        ELSE
            RETURN NEW;
        END IF;
    END IF;

    IF resource IS NULL OR epoch_text IS NULL OR holder_id IS NULL
       OR epoch_text !~ '^[0-9]+$'
       OR NOT buzz_writer_fence_check(resource, epoch_text::BIGINT, holder_id)
    THEN
        RAISE EXCEPTION
            'writer fence denied for table %, operation %, resource %, epoch %',
            TG_TABLE_NAME, TG_OP, resource, epoch_text
            USING ERRCODE = '42501';
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    ELSIF TG_OP = 'TRUNCATE' THEN
        RETURN NULL;
    ELSE
        RETURN NEW;
    END IF;
END
$$;

REVOKE ALL ON FUNCTION buzz_writer_fence_guard() FROM PUBLIC;
