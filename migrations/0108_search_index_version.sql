-- The search indexer's external version; see
-- schema/sequences/search_index_version.sql. A new sequence: it locks no
-- existing object.
SET LOCAL lock_timeout = '5s';
CREATE SEQUENCE search_index_version AS BIGINT START WITH 10000000000000000;
