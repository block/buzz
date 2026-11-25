SELECT community_id, false AS via_agent
FROM relay_members
WHERE pubkey = $1 AND role = 'owner'
UNION ALL
SELECT m.community_id, true AS via_agent
FROM users a
JOIN relay_members m ON m.community_id = a.community_id AND m.pubkey = encode(a.pubkey, 'hex')
WHERE a.agent_owner_pubkey = decode($1, 'hex') AND m.role = 'owner' AND m.pubkey <> $1
