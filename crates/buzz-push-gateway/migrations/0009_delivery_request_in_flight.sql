-- Tell a replay of a request id that is still being delivered apart from one
-- whose delivery finished. The gateway inserts new request ids with
-- finished=false and sets it true on a terminal outcome; a replay of an
-- unfinished id answers 503 retry instead of 404, so the relay retries rather
-- than treating its own timed-out first attempt as delivered. Existing rows
-- and rows written by an older gateway default to finished, which keeps the
-- old 404 behavior for them. A constant default adds the column without a
-- table rewrite.
SET LOCAL lock_timeout = '5s';
ALTER TABLE push_gateway_delivery_request_replays
    ADD COLUMN finished BOOLEAN NOT NULL DEFAULT true;
