-- Collapse loopback community hosts to canonical 127.0.0.1.
--
-- Before normalize_host collapsed loopback variants, a relay reachable via
-- both `localhost:3000` and `127.0.0.1:3000` could seed two separate community
-- rows. After the fix, all inbound Host headers normalize to `127.0.0.1:3000`,
-- so any `localhost:3000`-keyed row becomes an unreachable orphan.
--
-- This migration merges `localhost` and `[::1]` community rows into their
-- `127.0.0.1` counterpart (if one exists) or renames them in place (if not).
-- Relay members, channels, and other community-scoped data are reparented via
-- the community_id foreign key.

-- Step 1: For communities where a 127.0.0.1 counterpart already exists,
-- reparent all relay_members from the localhost row to the 127.0.0.1 row.
UPDATE relay_members rm
SET community_id = target.id
FROM communities target
JOIN communities source
  ON lower(source.host) IN ('localhost', 'localhost:3000', '[::1]', '[::1]:3000')
  AND lower(target.host) = replace(
        replace(lower(source.host), 'localhost', '127.0.0.1'),
        '::1', '127.0.0.1')
WHERE rm.community_id = source.id
  AND target.id != source.id;

-- Step 2: Rename remaining localhost/[::1] community hosts to 127.0.0.1
-- (no counterpart exists — rename in place).
UPDATE communities
SET host = replace(
      replace(host, 'localhost', '127.0.0.1'),
      '::1', '127.0.0.1')
WHERE lower(host) IN ('localhost', 'localhost:3000', '[::1]', '[::1]:3000')
  AND host NOT LIKE '127.0.0.1%';
