-- Name-only profile search: extract names without letting one stored profile
-- fail the query. See schema/functions/profile_name_fields.sql.
CREATE FUNCTION profile_name_fields(content TEXT, OUT name TEXT, OUT display_name TEXT)
LANGUAGE plpgsql IMMUTABLE STRICT AS $$
DECLARE
    doc JSONB;
BEGIN
    doc := content::jsonb;
    IF jsonb_typeof(doc) = 'object' THEN
        IF jsonb_typeof(doc -> 'name') = 'string' THEN
            name := doc ->> 'name';
        END IF;
        IF jsonb_typeof(doc -> 'display_name') = 'string' THEN
            display_name := doc ->> 'display_name';
        END IF;
    END IF;
EXCEPTION WHEN OTHERS THEN
    name := NULL;
    display_name := NULL;
END
$$;
