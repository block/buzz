-- Profile name search reads `name` and `display_name` from stored kind:0
-- content. Content is any text the signer chose, and some valid JSON cannot
-- become jsonb (for example `\u0000`, a lone surrogate, a huge number or very
-- deep nesting). Such content has no searchable names; it must never make the
-- whole search query fail. Only string values count as names.
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
