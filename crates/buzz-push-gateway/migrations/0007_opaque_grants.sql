SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';

-- Permission rows, counters and revocation tombstones are retained.
ALTER TABLE push_gateway_delegations ADD COLUMN grant_hash BYTEA;
