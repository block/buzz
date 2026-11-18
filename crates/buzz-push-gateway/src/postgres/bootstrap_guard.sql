-- This release supports fresh gateway initialization, not populated legacy
-- development databases. Run before migrations that could alter legacy data.
DO $$
DECLARE
    initialized boolean := false;
    populated boolean;
    candidate record;
BEGIN
    IF to_regclass('_sqlx_migrations') IS NOT NULL THEN
        EXECUTE 'SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 5 AND success)'
            INTO initialized;
    END IF;
    -- Ordinary subsequent deployments of the initialized gateway retain data.
    IF initialized THEN
        RETURN;
    END IF;

    FOR candidate IN
        SELECT schemaname, tablename FROM pg_tables
        WHERE schemaname = current_schema() AND tablename <> '_sqlx_migrations'
    LOOP
        EXECUTE format('SELECT EXISTS (SELECT 1 FROM %I.%I)',
            candidate.schemaname, candidate.tablename) INTO populated;
        IF populated THEN
            RAISE EXCEPTION 'Cannot initialize push gateway: non-empty pre-launch database (table %). Legacy upgrades are unsupported. Stop the development gateway and provision a fresh dedicated database. No migrations have run.', candidate.tablename;
        END IF;
    END LOOP;
END
$$;
