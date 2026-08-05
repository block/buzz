-- Protected-domain activation is a one-way V1 cutover. Removing the marker
-- would disable database lifecycle fencing and let an unaware release treat
-- the domain as legacy. O4 has no witnessed downgrade operation, so direct
-- marker removal remains unavailable.

CREATE FUNCTION deny_protected_domain_marker_delete() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'protected domain activation cannot be removed without a separately witnessed authority model'
        USING ERRCODE = 'check_violation';
END
$$;

CREATE TRIGGER protected_domain_marker_delete_guard
    BEFORE DELETE ON authorization_invalidation_domains
    FOR EACH ROW EXECUTE FUNCTION deny_protected_domain_marker_delete();
