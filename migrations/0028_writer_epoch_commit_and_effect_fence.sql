-- Close the two remaining writer-fence windows:
--
-- 1. Row-level BEFORE triggers reject stale writes early, but a transaction
--    can otherwise pass that check and commit after another process takes the
--    epoch. A deferred constraint trigger re-checks the lease at COMMIT.
-- 2. Redis and HTTP effects need a linearization point with epoch takeover.
--    The effect-begin function takes a shared lock on the live fence row and
--    the caller holds that transaction until the external operation returns.
--    Epoch acquisition takes FOR UPDATE on the same row, so takeover waits
--    for an in-flight effect and a stale process cannot start a new one.

CREATE OR REPLACE FUNCTION buzz_writer_fence_effect_check(
    p_resource TEXT,
    p_epoch BIGINT,
    p_holder_id TEXT
) RETURNS BOOLEAN
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    required BOOLEAN;
    current_epoch BIGINT;
    current_holder_id TEXT;
    current_mode TEXT;
    current_lease_until TIMESTAMPTZ;
BEGIN
    SELECT COALESCE(
        (SELECT c.required
           FROM buzz_writer_fence_config AS c
          WHERE c.singleton),
        TRUE
    )
    INTO required;

    IF NOT required THEN
        RETURN TRUE;
    END IF;

    -- This lock is held until the surrounding transaction ends. It is the
    -- serialization point with buzz_writer_fence_acquire's FOR UPDATE.
    SELECT f.epoch, f.holder_id, f.mode, f.lease_until
      INTO current_epoch, current_holder_id, current_mode, current_lease_until
      FROM buzz_writer_fence AS f
     WHERE f.resource = p_resource
     FOR SHARE;

    RETURN FOUND
       AND current_epoch = p_epoch
       AND current_holder_id = p_holder_id
       AND current_mode = 'active'
       AND current_lease_until > clock_timestamp();
END
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_commit_guard() RETURNS trigger
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
        ELSE
            RETURN NEW;
        END IF;
    END IF;

    IF resource IS NULL OR epoch_text IS NULL OR holder_id IS NULL
       OR epoch_text !~ '^[0-9]+$'
       OR NOT buzz_writer_fence_effect_check(resource, epoch_text::BIGINT, holder_id)
    THEN
        RAISE EXCEPTION
            'writer fence denied at commit for table %, operation %, resource %, epoch %',
            TG_TABLE_NAME, TG_OP, resource, epoch_text
            USING ERRCODE = '42501';
    END IF;

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    ELSE
        RETURN NEW;
    END IF;
END
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_truncate_guard() RETURNS trigger
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
        RETURN NULL;
    END IF;

    IF resource IS NULL OR epoch_text IS NULL OR holder_id IS NULL
       OR epoch_text !~ '^[0-9]+$'
       OR NOT buzz_writer_fence_effect_check(resource, epoch_text::BIGINT, holder_id)
    THEN
        RAISE EXCEPTION
            'writer fence denied for table %, operation TRUNCATE, resource %, epoch %',
            TG_TABLE_NAME, resource, epoch_text
            USING ERRCODE = '42501';
    END IF;

    RETURN NULL;
END
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_begin_effect(
    p_resource TEXT,
    p_epoch BIGINT,
    p_holder_id TEXT,
    p_effect_key TEXT
) RETURNS BOOLEAN
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = public
AS $$
BEGIN
    IF p_effect_key IS NULL OR length(btrim(p_effect_key)) NOT BETWEEN 1 AND 256 THEN
        RAISE EXCEPTION 'invalid writer-fence effect key'
            USING ERRCODE = '22023';
    END IF;

    IF p_resource IS NULL OR p_epoch IS NULL OR p_holder_id IS NULL
       OR NOT buzz_writer_fence_effect_check(p_resource, p_epoch, p_holder_id)
    THEN
        RAISE EXCEPTION 'writer fence denied before external effect %', p_effect_key
            USING ERRCODE = '42501';
    END IF;
    RETURN TRUE;
END
$$;

REVOKE ALL ON FUNCTION buzz_writer_fence_effect_check(TEXT, BIGINT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_commit_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_truncate_guard() FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_begin_effect(TEXT, BIGINT, TEXT, TEXT) FROM PUBLIC;

DO $$
DECLARE
    relation RECORD;
BEGIN
    FOR relation IN
        SELECT c.relname
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname = 'public'
          AND c.relkind IN ('r', 'p')
          AND NOT EXISTS (
              SELECT 1 FROM pg_inherits i WHERE i.inhrelid = c.oid
          )
          AND c.relname NOT IN (
              'buzz_writer_fence',
              'buzz_writer_fence_config',
              '_sqlx_migrations'
          )
    LOOP
        EXECUTE format(
            'DROP TRIGGER IF EXISTS buzz_writer_fence_commit ON public.%I',
            relation.relname
        );
        EXECUTE format(
            'CREATE CONSTRAINT TRIGGER buzz_writer_fence_commit
             AFTER INSERT OR UPDATE OR DELETE ON public.%I
             DEFERRABLE INITIALLY DEFERRED
             FOR EACH ROW EXECUTE FUNCTION public.buzz_writer_fence_commit_guard()',
            relation.relname
        );
        EXECUTE format(
            'ALTER TABLE public.%I ENABLE ALWAYS TRIGGER buzz_writer_fence_commit',
            relation.relname
        );

        EXECUTE format(
            'DROP TRIGGER IF EXISTS buzz_writer_fence_truncate ON public.%I',
            relation.relname
        );
        EXECUTE format(
            'CREATE TRIGGER buzz_writer_fence_truncate
             BEFORE TRUNCATE ON public.%I
             FOR EACH STATEMENT EXECUTE FUNCTION public.buzz_writer_fence_truncate_guard()',
            relation.relname
        );
        EXECUTE format(
            'ALTER TABLE public.%I ENABLE ALWAYS TRIGGER buzz_writer_fence_truncate',
            relation.relname
        );
    END LOOP;
END
$$;
