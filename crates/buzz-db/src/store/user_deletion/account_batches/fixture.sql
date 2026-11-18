-- Test-only data: identical row identifiers in two communities, three users,
-- and enough UUID/composite keys to cross every multirow pagination boundary.
BEGIN;
CREATE TEMP TABLE account_fixture_keys AS
SELECT community, pubkey,
       CASE WHEN pubkey = repeat('33',32) THEN pubkey ELSE '__TARGET__' END AS actor
FROM (VALUES ('__COMMUNITY__'::uuid), ('__OTHER__'::uuid)) c(community)
CROSS JOIN (VALUES ('__TARGET__'), (repeat('22',32)), (repeat('33',32))) p(pubkey);
INSERT INTO communities(id,host)
SELECT DISTINCT community, community::text || '.example.org' FROM account_fixture_keys;
INSERT INTO users(community_id,pubkey)
SELECT community,decode(pubkey,'hex') FROM account_fixture_keys;
INSERT INTO relay_members(community_id,pubkey,role,added_by)
SELECT community,pubkey,'member',actor FROM account_fixture_keys;
INSERT INTO pubkey_allowlist(community_id,pubkey,added_by)
SELECT community,decode(pubkey,'hex'),decode(actor,'hex') FROM account_fixture_keys;
INSERT INTO channels(community_id,id,name,created_by)
SELECT DISTINCT community,md5(n::text)::uuid,'fixture-'||n,decode(repeat('22',32),'hex')
FROM account_fixture_keys CROSS JOIN generate_series(1,3) n;
INSERT INTO channel_members(community_id,channel_id,pubkey,invited_by,removed_by,removed_at)
SELECT community,md5(n::text)::uuid,decode(pubkey,'hex'),decode(actor,'hex'),
       CASE WHEN n > 1 THEN decode(actor,'hex') END,
       CASE WHEN n > 1 THEN now() END
FROM account_fixture_keys CROSS JOIN generate_series(1,3) n;
INSERT INTO api_tokens(community_id,id,owner_pubkey,token_hash,name,scopes,revoked_by,revoked_at,expires_at)
SELECT community,md5(pubkey||n)::uuid,decode(pubkey,'hex'),
       decode(md5(pubkey||n)||md5(pubkey||n),'hex'),'fixture','[]',
       CASE WHEN n > 1 THEN decode(actor,'hex') END,
       CASE WHEN n > 1 THEN now() END, CASE WHEN n = 3 THEN now() - interval '1 day' END
FROM account_fixture_keys CROSS JOIN generate_series(1,3) n;
INSERT INTO relay_invites(community_id,id,created_by,token_hash,expires_at)
SELECT community,md5(pubkey||n)::uuid,pubkey,decode(md5(pubkey||n)||md5(pubkey||n),'hex'),now()
FROM account_fixture_keys CROSS JOIN generate_series(1,3) n;
INSERT INTO join_policy_acceptances(community_id,pubkey,policy_version)
SELECT community,pubkey,lpad(n::text,64,'0') FROM account_fixture_keys CROSS JOIN generate_series(1,3) n;
INSERT INTO personal_read_accounts(community_id,actor)
SELECT community,decode(pubkey,'hex') FROM account_fixture_keys;
INSERT INTO personal_read_frontiers(community_id,actor,channel_id,root_id,through_timestamp)
SELECT community,decode(pubkey,'hex'),md5(n::text)::uuid,decode(lpad(r::text,64,'0'),'hex'),now()
FROM account_fixture_keys CROSS JOIN generate_series(1,3) n CROSS JOIN generate_series(1,3) r;
INSERT INTO archived_identities(community_id,pubkey,actor,consent_path,replaced_by,request_event_id)
SELECT community,pubkey,pubkey,'self',actor,'fixture' FROM account_fixture_keys;
INSERT INTO parameterized_event_watermarks(community_id,pubkey,kind,d_tag,created_at,event_id)
SELECT community,decode(pubkey,'hex'),30078,'fixture',now(),decode(pubkey,'hex') FROM account_fixture_keys;
UPDATE relay_members SET updated_at='2000-01-01T00:00:00Z'
WHERE community_id IN ('__COMMUNITY__','__OTHER__');
UPDATE channel_members SET sync_modified_at='2000-01-01T00:00:00Z'
WHERE community_id IN ('__COMMUNITY__','__OTHER__');
DROP TABLE account_fixture_keys;
COMMIT;
