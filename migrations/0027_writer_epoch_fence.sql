-- Cross-process writer epoch/lease fence.
--
-- The tables are the authoritative control plane. The server-side config row
-- controls whether the trigger is required; session GUCs can carry only the
-- resource/epoch/holder tuple and cannot disable enforcement.
--
-- The relay writer pool stamps every connection with resource/epoch/holder
-- GUCs. ENABLE ALWAYS triggers then reject durable mutations from an expired,
-- replaced, missing, or partitioned writer. The runtime database role must not
-- own these tables and must not be superuser; that is a separate cutover gate.

CREATE TABLE buzz_writer_fence (
    resource     TEXT        PRIMARY KEY CHECK (length(resource) BETWEEN 1 AND 128),
    epoch        BIGINT      NOT NULL CHECK (epoch >= 1),
    holder_id    TEXT        NOT NULL CHECK (length(holder_id) BETWEEN 1 AND 128),
    mode         TEXT        NOT NULL CHECK (mode IN ('active', 'draining', 'fenced')),
    lease_until  TIMESTAMPTZ NOT NULL,
    updated_at   TIMESTAMPTZ NOT NULL
);

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('buzz_writer_fence', 'deployment-global writer epoch/lease authority');

CREATE TABLE buzz_writer_fence_config (
    singleton   BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    required    BOOLEAN     NOT NULL DEFAULT FALSE,
    updated_at  TIMESTAMPTZ NOT NULL
);

INSERT INTO buzz_writer_fence_config (singleton, required, updated_at)
VALUES (TRUE, FALSE, clock_timestamp());

INSERT INTO _operator_global_tables (table_name, reason) VALUES
    ('buzz_writer_fence_config', 'deployment-global writer-fence enforcement mode');

REVOKE ALL ON buzz_writer_fence_config FROM PUBLIC;

-- The relay calls these functions instead of receiving DML privileges on the
-- authority table. Keep the table itself private; the deployment cutover must
-- additionally make the function owner a dedicated migration/control role
-- and the relay role a non-owner, non-superuser.
REVOKE ALL ON buzz_writer_fence FROM PUBLIC;

CREATE OR REPLACE FUNCTION buzz_writer_fence_acquire(
    p_resource TEXT,
    p_holder_id TEXT,
    p_lease_seconds INTEGER
) RETURNS TABLE(epoch BIGINT, lease_until TIMESTAMPTZ)
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    v_current_epoch BIGINT;
    v_current_mode TEXT;
    v_current_lease_until TIMESTAMPTZ;
    v_epoch BIGINT;
    v_lease_until TIMESTAMPTZ;
BEGIN
    IF p_resource IS NULL OR length(btrim(p_resource)) NOT BETWEEN 1 AND 128
       OR p_holder_id IS NULL OR length(btrim(p_holder_id)) NOT BETWEEN 1 AND 128
       OR p_lease_seconds < 5 OR p_lease_seconds > 86400
    THEN
        RAISE EXCEPTION 'invalid writer fence acquisition parameters'
            USING ERRCODE = '22023';
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended(p_resource, 0));

    SELECT f.epoch, f.mode, f.lease_until
      INTO v_current_epoch, v_current_mode, v_current_lease_until
      FROM buzz_writer_fence AS f
     WHERE f.resource = p_resource
       FOR UPDATE;

    IF FOUND THEN
        IF v_current_mode = 'active' AND v_current_lease_until > clock_timestamp() THEN
            RAISE EXCEPTION 'writer lease held for resource % until %',
                p_resource, v_current_lease_until
                USING ERRCODE = '55P03';
        END IF;

        v_epoch := v_current_epoch + 1;
        v_lease_until := clock_timestamp() + make_interval(secs => p_lease_seconds::double precision);
        UPDATE buzz_writer_fence
           SET epoch = v_epoch,
               holder_id = p_holder_id,
               mode = 'active',
               lease_until = v_lease_until,
               updated_at = clock_timestamp()
         WHERE resource = p_resource;
    ELSE
        v_epoch := 1;
        v_lease_until := clock_timestamp() + make_interval(secs => p_lease_seconds::double precision);
        INSERT INTO buzz_writer_fence (resource, epoch, holder_id, mode, lease_until, updated_at)
        VALUES (p_resource, v_epoch, p_holder_id, 'active', v_lease_until, clock_timestamp());
    END IF;

    RETURN QUERY SELECT v_epoch, v_lease_until;
END
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_renew(
    p_resource TEXT,
    p_epoch BIGINT,
    p_holder_id TEXT,
    p_lease_seconds INTEGER
) RETURNS BOOLEAN
LANGUAGE plpgsql
VOLATILE
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    v_rows BIGINT;
BEGIN
    IF p_lease_seconds < 5 OR p_lease_seconds > 86400 THEN
        RAISE EXCEPTION 'invalid writer fence renewal parameters'
            USING ERRCODE = '22023';
    END IF;

    UPDATE buzz_writer_fence
       SET lease_until = clock_timestamp() + make_interval(secs => p_lease_seconds::double precision),
           updated_at = clock_timestamp()
     WHERE resource = p_resource
       AND epoch = p_epoch
       AND holder_id = p_holder_id
       AND mode = 'active'
       AND lease_until > clock_timestamp();
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    RETURN v_rows = 1;
END
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_state(
    p_resource TEXT
) RETURNS TABLE(
    epoch BIGINT,
    holder_id TEXT,
    mode TEXT,
    lease_until TIMESTAMPTZ,
    updated_at TIMESTAMPTZ
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT f.epoch, f.holder_id, f.mode, f.lease_until, f.updated_at
      FROM buzz_writer_fence AS f
     WHERE f.resource = p_resource
$$;

CREATE OR REPLACE FUNCTION buzz_writer_fence_check(
    p_resource TEXT,
    p_epoch BIGINT,
    p_holder_id TEXT
) RETURNS BOOLEAN
LANGUAGE sql
VOLATILE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT EXISTS (
        SELECT 1
        FROM buzz_writer_fence
        WHERE resource = p_resource
          AND epoch = p_epoch
          AND holder_id = p_holder_id
          AND mode = 'active'
          AND lease_until > clock_timestamp()
    )
$$;

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

-- Do not leave lease acquisition as a database-wide capability. The cutover
-- operator grants these three exact signatures to the dedicated non-owner
-- relay role after the role/ownership audit has passed.
REVOKE ALL ON FUNCTION buzz_writer_fence_acquire(TEXT, TEXT, INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_renew(TEXT, BIGINT, TEXT, INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_state(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_check(TEXT, BIGINT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION buzz_writer_fence_guard() FROM PUBLIC;

-- Attach to every current application table and to the partitioned events
-- parent. Child partitions inherit these triggers; future partition creation
-- therefore remains fenced. _sqlx_migrations is excluded so migrations can be
-- recorded during a controlled cutover. The two writer-fence control tables
-- are excluded because their security-definer functions and operator update
-- are the control plane itself.
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
            'DROP TRIGGER IF EXISTS buzz_writer_fence_dml ON public.%I',
            relation.relname
        );
        EXECUTE format(
            'CREATE TRIGGER buzz_writer_fence_dml
             BEFORE INSERT OR UPDATE OR DELETE ON public.%I
             FOR EACH ROW EXECUTE FUNCTION public.buzz_writer_fence_guard()',
            relation.relname
        );
        EXECUTE format(
            'ALTER TABLE public.%I ENABLE ALWAYS TRIGGER buzz_writer_fence_dml',
            relation.relname
        );

        EXECUTE format(
            'DROP TRIGGER IF EXISTS buzz_writer_fence_truncate ON public.%I',
            relation.relname
        );
        EXECUTE format(
            'CREATE TRIGGER buzz_writer_fence_truncate
             BEFORE TRUNCATE ON public.%I
             FOR EACH STATEMENT EXECUTE FUNCTION public.buzz_writer_fence_guard()',
            relation.relname
        );
        EXECUTE format(
            'ALTER TABLE public.%I ENABLE ALWAYS TRIGGER buzz_writer_fence_truncate',
            relation.relname
        );
    END LOOP;
END
$$;
