-- PostgreSQL restore sessions deliberately clear search_path. Buzz trigger
-- functions call other Buzz functions and tables by their unqualified names,
-- so data-only restores must not inherit name resolution from the invoker.
-- Pin every application-owned public function while leaving extension-owned
-- and provider-owned functions untouched.

SET LOCAL search_path TO public, pg_catalog;

DO $$
DECLARE
    function_identity REGPROCEDURE;
BEGIN
    FOR function_identity IN
        SELECT procedure.oid::REGPROCEDURE
          FROM pg_proc AS procedure
          JOIN pg_namespace AS namespace
            ON namespace.oid = procedure.pronamespace
         WHERE namespace.nspname = 'public'
           AND procedure.prokind = 'f'
           AND procedure.proowner = (SELECT oid FROM pg_roles WHERE rolname = current_user)
           AND NOT EXISTS (
               SELECT 1
                 FROM pg_depend AS dependency
                WHERE dependency.classid = 'pg_proc'::REGCLASS
                  AND dependency.objid = procedure.oid
                  AND dependency.deptype = 'e'
           )
         ORDER BY procedure.oid
    LOOP
        EXECUTE format(
            'ALTER FUNCTION %s SET search_path TO public, pg_catalog',
            function_identity
        );
    END LOOP;
END
$$;
