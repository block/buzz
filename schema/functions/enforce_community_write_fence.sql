CREATE FUNCTION enforce_community_write_fence() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSIF TG_OP = 'DELETE' THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id IS NOT DISTINCT FROM NEW.community_id THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id IS NULL THEN
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSIF NEW.community_id IS NULL THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
    ELSIF OLD.community_id < NEW.community_id THEN
        PERFORM assert_community_write_allowed(OLD.community_id);
        PERFORM assert_community_write_allowed(NEW.community_id);
    ELSE
        PERFORM assert_community_write_allowed(NEW.community_id);
        PERFORM assert_community_write_allowed(OLD.community_id);
    END IF;

    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;
