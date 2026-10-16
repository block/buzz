-- Serialize invitation admission with restriction and ownership changes per tenant.
CREATE FUNCTION invite_admission_lock(community UUID) RETURNS VOID
LANGUAGE sql AS $$
    SELECT pg_advisory_xact_lock(hashtextextended('invite-admission:' || community::text, 0));
$$;
