-- Keep owner quota lookups bounded: incomplete owner deletions reserve an
-- active slot, and every non-aborted owner deletion counts toward the
-- lifetime cap after relay membership is purged.
SET LOCAL lock_timeout = '5s';

CREATE INDEX community_deletion_requests_owner_quota_reservations
    ON community_deletion_requests (owner_pubkey)
    INCLUDE (community_id, completed_at)
    WHERE request_origin = 'owner'
      AND stage <> 'aborted';
