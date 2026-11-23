CREATE SCHEMA media_access_plan;
SET LOCAL search_path TO media_access_plan,public;
CREATE TABLE media_access_plan.channels(community_id uuid,id uuid,deleted_at timestamptz,visibility text);
CREATE TABLE media_access_plan.channel_members(community_id uuid,channel_id uuid,pubkey bytea,removed_at timestamptz);
CREATE INDEX plan_members ON media_access_plan.channel_members(community_id,channel_id,pubkey);
CREATE INDEX plan_channels ON media_access_plan.channels(community_id,id);
INSERT INTO media_access_plan.channels SELECT '00000000-0000-0000-0000-000000000001',md5(g::text)::uuid,NULL,CASE WHEN g%2=0 THEN 'open' ELSE 'private' END FROM generate_series(0,255) g;
INSERT INTO media_access_plan.channel_members SELECT community_id,id,decode(repeat('1',64),'hex'),NULL FROM media_access_plan.channels WHERE visibility='private';
CREATE TABLE media_access_plan.events (LIKE public.events INCLUDING DEFAULTS INCLUDING GENERATED) PARTITION BY RANGE (created_at);
CREATE TABLE media_access_plan.events_2025_01 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-01-01') TO ('2025-02-01');
CREATE TABLE media_access_plan.events_2025_02 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-02-01') TO ('2025-03-01');
CREATE TABLE media_access_plan.events_2025_03 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-03-01') TO ('2025-04-01');
CREATE TABLE media_access_plan.events_2025_04 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-04-01') TO ('2025-05-01');
CREATE TABLE media_access_plan.events_2025_05 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-05-01') TO ('2025-06-01');
CREATE TABLE media_access_plan.events_2025_06 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-06-01') TO ('2025-07-01');
CREATE TABLE media_access_plan.events_2025_07 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-07-01') TO ('2025-08-01');
CREATE TABLE media_access_plan.events_2025_08 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-08-01') TO ('2025-09-01');
CREATE TABLE media_access_plan.events_2025_09 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-09-01') TO ('2025-10-01');
CREATE TABLE media_access_plan.events_2025_10 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-10-01') TO ('2025-11-01');
CREATE TABLE media_access_plan.events_2025_11 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-11-01') TO ('2025-12-01');
CREATE TABLE media_access_plan.events_2025_12 PARTITION OF media_access_plan.events FOR VALUES FROM ('2025-12-01') TO ('2026-01-01');
CREATE TABLE media_access_plan.events_2026_01 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-01-01') TO ('2026-02-01');
CREATE TABLE media_access_plan.events_2026_02 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-02-01') TO ('2026-03-01');
CREATE TABLE media_access_plan.events_2026_03 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-03-01') TO ('2026-04-01');
CREATE TABLE media_access_plan.events_2026_04 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-04-01') TO ('2026-05-01');
CREATE TABLE media_access_plan.events_2026_05 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE media_access_plan.events_2026_06 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE media_access_plan.events_2026_07 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
CREATE TABLE media_access_plan.events_2026_08 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-08-01') TO ('2026-09-01');
CREATE TABLE media_access_plan.events_2026_09 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-09-01') TO ('2026-10-01');
CREATE TABLE media_access_plan.events_2026_10 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-10-01') TO ('2026-11-01');
CREATE TABLE media_access_plan.events_2026_11 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-11-01') TO ('2026-12-01');
CREATE TABLE media_access_plan.events_2026_12 PARTITION OF media_access_plan.events FOR VALUES FROM ('2026-12-01') TO ('2027-01-01');
CREATE INDEX media_access_plan_tags ON media_access_plan.events USING gin(tags jsonb_path_ops);
CREATE INDEX media_access_plan_id ON media_access_plan.events(community_id,id,created_at DESC);
CREATE INDEX media_access_plan_channel ON media_access_plan.events(community_id,channel_id,created_at DESC,id);
CREATE INDEX media_access_plan_kind ON media_access_plan.events(community_id,kind,created_at DESC,id);
CREATE INDEX media_access_plan_addressable ON media_access_plan.events(community_id,kind,pubkey,channel_id,deleted_at);
INSERT INTO media_access_plan.events(community_id,id,pubkey,created_at,kind,tags,content,sig,channel_id)
SELECT '00000000-0000-0000-0000-000000000001', decode(md5(g::text)||md5(g::text),'hex'), decode(repeat('1',64),'hex'),
 '2025-01-01'::timestamptz + (g/50000) * interval '1 month', CASE WHEN g%100=2 THEN 0 WHEN g%4=3 THEN 40003 ELSE 9 END,
 CASE WHEN g%50000=0 THEN jsonb_build_array(jsonb_build_array('imeta','x '||repeat('a',64),'url https://plan.example/media/'||repeat('a',64)||'.bin'))
 WHEN g%4=3 THEN jsonb_build_array(jsonb_build_array('e',upper(md5((g-2)::text)||md5((g-2)::text))),jsonb_build_array('imeta','x '||CASE WHEN g%50000=3 THEN repeat('a',64) ELSE md5(g::text)||md5(g::text) END,'url https://plan.example/media/'||CASE WHEN g%50000=7 THEN repeat('a',64) ELSE md5(g::text)||md5(g::text) END||'.bin'))
 WHEN g%50000=1 THEN jsonb_build_array(jsonb_build_array('imeta','url https://plan.example/media/'||repeat('a',64)||'.bin'))
 ELSE jsonb_build_array(jsonb_build_array('imeta','x '||encode(decode(md5(g::text)||md5(g::text),'hex'),'hex'))) END,
 CASE WHEN g=2 THEN '{"picture":"https://plan.example/media/'||repeat('a',64)||'.bin"}' ELSE 'ordinary content' END,
 decode(repeat('2',128),'hex'), md5(((g/4)%256)::text)::uuid FROM generate_series(0,1199999) g;
-- Finish bulk-load GIN maintenance as autovacuum does in steady state.
-- Otherwise every URL alternative repeatedly scans the bulk insert pending list.
SELECT gin_clean_pending_list(inhrelid) FROM pg_inherits
WHERE inhparent='media_access_plan.media_access_plan_tags'::regclass;
ANALYZE media_access_plan.events;
