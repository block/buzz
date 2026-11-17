SET LOCAL lock_timeout = '5s';

-- The relay builds the community deletion lock key in Rust (#9396) and reads
-- profile names with inline SQL (#9397). No supported relay calls either
-- function. Migrations 0029 and 0064 stay frozen. Neither drop uses CASCADE:
-- if anything still depends on a function, the migration fails instead.
DROP FUNCTION community_deletion_lock_key(UUID);
DROP FUNCTION profile_name_fields(TEXT);
