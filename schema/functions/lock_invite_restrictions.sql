CREATE FUNCTION lock_invite_restrictions() RETURNS TRIGGER
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM invite_admission_lock(NEW.community_id);
    RETURN NEW;
END
$$;
