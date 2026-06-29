-- Upsert local dev community hosts for row-zero host binding.
--
-- :hosts is a newline-separated list of authorities derived by seed-hosts.py
-- (e.g. "localhost:3000", "localhost", "127.0.0.1", "127.0.0.1:3000"). Each
-- distinct authority becomes a communities row; existing rows are left
-- untouched (ON CONFLICT against the unique lower(host) index).
INSERT INTO communities (host)
SELECT btrim(h)
FROM unnest(string_to_array(:'hosts', E'\n')) AS h
WHERE btrim(h) <> ''
ON CONFLICT (lower(host)) DO NOTHING;
