\set ON_ERROR_STOP on

BEGIN;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_roles WHERE rolname = 'buzz_writer_fence_owner'
    ) THEN
        CREATE ROLE buzz_writer_fence_owner
            NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE;
    END IF;
END
$$;

ALTER ROLE buzz_writer_fence_owner
    NOLOGIN NOINHERIT NOSUPERUSER NOCREATEDB NOCREATEROLE;

-- Drain every existing runtime session before renaming the role. PostgreSQL
-- keeps the old role identity and privileges on already-open sessions; role
-- rotation without this step therefore does not rotate authority at all.
DO $$
DECLARE
    session_record RECORD;
    attempt INTEGER;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'buzz') THEN
        RAISE EXCEPTION 'expected pre-rotation runtime role buzz is missing';
    END IF;

    -- Prevent an old relay from opening a replacement session while the
    -- existing sessions are being terminated. The current administrator
    -- session, if it is using buzz, is unaffected by changing the role limit.
    ALTER ROLE buzz CONNECTION LIMIT 0;

    -- pg_terminate_backend signals a backend; it does not promise that the
    -- row disappears before the function returns. Re-scan briefly so the
    -- rename below cannot race a still-alive old privileged session.
    FOR attempt IN 1..100 LOOP
        PERFORM pg_stat_clear_snapshot();
        FOR session_record IN
            SELECT pid
            FROM pg_stat_activity
            WHERE usename = 'buzz'
              AND pid <> pg_backend_pid()
        LOOP
            PERFORM pg_terminate_backend(session_record.pid, 5000);
        END LOOP;

        PERFORM pg_stat_clear_snapshot();
        EXIT WHEN NOT EXISTS (
            SELECT 1
            FROM pg_stat_activity
            WHERE usename = 'buzz'
              AND pid <> pg_backend_pid()
        );
        PERFORM pg_sleep(0.1);
    END LOOP;

    IF EXISTS (
        SELECT 1
        FROM pg_stat_activity
        WHERE usename = 'buzz'
          AND pid <> pg_backend_pid()
    ) THEN
        RAISE EXCEPTION 'runtime role buzz still has sessions after drain';
    END IF;
END
$$;

-- The initial PostgreSQL role remains an operator-only bootstrap superuser.
-- It is never used for the relay runtime, and the audit below proves the new
-- runtime role owns neither tables nor authority state.
ALTER ROLE buzz RENAME TO buzz_bootstrap_20260804;
ALTER ROLE buzz_bootstrap_20260804
    LOGIN PASSWORD :'bootstrap_password' CONNECTION LIMIT -1;
CREATE ROLE buzz
    LOGIN PASSWORD :'runtime_password'
    NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT
    CONNECTION LIMIT -1;

ALTER TABLE public.buzz_writer_fence OWNER TO buzz_writer_fence_owner;
ALTER TABLE public.buzz_writer_fence_config OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_acquire(text, text, integer)
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer)
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_state(text)
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_check(text, bigint, text)
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_guard()
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_effect_check(text, bigint, text)
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_commit_guard()
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_truncate_guard()
    OWNER TO buzz_writer_fence_owner;
ALTER FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text)
    OWNER TO buzz_writer_fence_owner;

REVOKE ALL ON public.buzz_writer_fence FROM PUBLIC;
REVOKE ALL ON public.buzz_writer_fence FROM buzz;
REVOKE ALL ON public.buzz_writer_fence_config FROM PUBLIC, buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_acquire(text, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_state(text) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text) TO buzz;

GRANT CONNECT ON DATABASE buzz TO buzz;
GRANT USAGE ON SCHEMA public TO buzz;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO buzz;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO buzz;
REVOKE ALL ON public.buzz_writer_fence FROM buzz;
REVOKE ALL ON public._sqlx_migrations FROM buzz;
REVOKE CREATE ON SCHEMA public FROM PUBLIC;

ALTER DEFAULT PRIVILEGES FOR ROLE buzz_bootstrap_20260804 IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO buzz;
ALTER DEFAULT PRIVILEGES FOR ROLE buzz_bootstrap_20260804 IN SCHEMA public
    GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO buzz;

REVOKE ALL ON FUNCTION public.buzz_writer_fence_acquire(text, text, integer) FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer) FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_state(text) FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_check(text, bigint, text) FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_guard() FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_effect_check(text, bigint, text) FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_commit_guard() FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_truncate_guard() FROM PUBLIC, buzz;
REVOKE ALL ON FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text) FROM PUBLIC, buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_acquire(text, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_renew(text, bigint, text, integer) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_state(text) TO buzz;
GRANT EXECUTE ON FUNCTION public.buzz_writer_fence_begin_effect(text, bigint, text, text) TO buzz;

-- Requiredness is stored in a server-side control row. A database/session GUC
-- is intentionally not used as the security boundary because a session can
-- override a database default with SET or SET LOCAL.
UPDATE public.buzz_writer_fence_config
   SET required = TRUE, updated_at = clock_timestamp()
 WHERE singleton;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM pg_stat_activity
        WHERE usename IN ('buzz', 'buzz_bootstrap_20260804')
          AND pid <> pg_backend_pid()
    ) THEN
        RAISE EXCEPTION 'writer-role rotation left an old runtime session active';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_roles r ON r.oid = c.relowner
        WHERE n.nspname = 'public'
          AND c.relkind IN ('r', 'p', 'S')
          AND r.rolname = 'buzz'
    ) THEN
        RAISE EXCEPTION 'runtime role buzz owns a public relation after rotation';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM pg_roles
        WHERE rolname = 'buzz' AND rolsuper
    ) THEN
        RAISE EXCEPTION 'runtime role buzz is still superuser after rotation';
    END IF;
END
$$;

COMMIT;
