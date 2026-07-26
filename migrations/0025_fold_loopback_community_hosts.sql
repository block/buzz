-- Backfill for the loopback fold in `buzz_core::tenant::normalize_host`.
--
-- `normalize_host` now folds every loopback spelling (`localhost`,
-- 127.0.0.0/8, `::1`) to `127.0.0.1`, so that a host header and a
-- client-canonicalized relay URL for the same loopback deployment agree.
-- `communities.host` stores the already-normalized key, which means rows
-- written under the previous rule are now keyed by a host no request will
-- ever resolve to.
--
-- Without this backfill the upgrade silently strands data. `communities.host`
-- is unique on `lower(host)`, and `lower('localhost:3000')` does not conflict
-- with `lower('127.0.0.1:3000')`, so `Db::ensure_configured_community` would
-- INSERT a *second* community with a fresh UUID at startup. Every existing
-- channel, member, and event stays attached to the old id while all
-- post-upgrade requests bind to the new, empty one: the deployment comes back
-- up looking wiped, with the original data intact but unreachable.
--
-- Collisions fail the migration rather than guessing. If a deployment already
-- has two loopback communities that fold onto the same key (say `localhost`
-- and `127.0.0.1`), silently keeping one would strand the other's data. That
-- is a pre-existing misconfiguration on a single-machine host, and the
-- operator has to decide which community survives.

DO $$
DECLARE
    conflict_report text;
BEGIN
    -- Any two rows whose folded hosts are equal would violate
    -- idx_communities_host once rewritten. Report every clashing group.
    SELECT string_agg(detail, '; ' ORDER BY detail)
      INTO conflict_report
      FROM (
          SELECT format('[%s] all fold to %L', string_agg(host, ', ' ORDER BY host), fold)
                 AS detail
            FROM (
                SELECT
                    host,
                    CASE
                        WHEN host ~ '^(localhost|127\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|\[::1\])(:[0-9]+)?$'
                            THEN '127.0.0.1' || COALESCE(substring(host from ':[0-9]+$'), '')
                        ELSE host
                    END AS fold
                  FROM communities
            ) folded
           GROUP BY fold
          HAVING count(*) > 1
      ) collisions;

    IF conflict_report IS NOT NULL THEN
        RAISE EXCEPTION
            'cannot fold loopback community hosts: % . Merge or remove the duplicate communities so each folded host is unique, then re-run the migration.',
            conflict_report
            USING HINT = 'Loopback spellings (localhost, 127.0.0.0/8, ::1) now share one community key.';
    END IF;

    UPDATE communities
       SET host = '127.0.0.1' || COALESCE(substring(host from ':[0-9]+$'), '')
     WHERE host ~ '^(localhost|127\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|\[::1\])(:[0-9]+)?$'
       AND host <> '127.0.0.1' || COALESCE(substring(host from ':[0-9]+$'), '');
END $$;
