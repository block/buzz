-- Operator-only scan: all communities, private and historical records included.
-- Select media-bearing payloads explicitly; do not copy credentials or signing keys.
SELECT 'events'::text AS source_table, e.community_id,
 jsonb_build_array(e.created_at,encode(e.id,'hex')) AS source_key, CASE WHEN e.kind=30382 AND e.d_tag ~ '^[0-9a-f]{64}$' THEN e.d_tag ELSE encode(e.pubkey,'hex') END AS subject,
 jsonb_build_object('content',e.content,'tags',e.tags) AS payload,
 NULL::text AS direct_hash, false AS upload_claim, true AS subject_owned
FROM events e
UNION ALL SELECT 'users',community_id,to_jsonb(encode(pubkey,'hex')),encode(pubkey,'hex'),
 jsonb_build_object('avatar_url',avatar_url,'about',about,'capabilities',capabilities),NULL,false,true FROM users
UNION ALL SELECT 'nip_fi_profile_heads',community_id,to_jsonb(pubkey),pubkey,event,NULL,false,true FROM nip_fi_profile_heads
UNION ALL SELECT 'workflows',community_id,to_jsonb(id),encode(owner_pubkey,'hex'),definition,NULL,false,true FROM workflows
UNION ALL SELECT 'workflow_runs',r.community_id,to_jsonb(r.id),encode(w.owner_pubkey,'hex'),
 jsonb_build_object('trigger_context',r.trigger_context,'execution_trace',r.execution_trace,'error_message',r.error_message),NULL,false,true
 FROM workflow_runs r LEFT JOIN workflows w ON (w.community_id,w.id)=(r.community_id,r.workflow_id)
UNION ALL SELECT 'workflow_approvals',a.community_id,to_jsonb(encode(a.token,'hex')),encode(w.owner_pubkey,'hex'),
 jsonb_build_object('note',a.note,'approver_spec',a.approver_spec),NULL,false,true
 FROM workflow_approvals a LEFT JOIN workflows w ON (w.community_id,w.id)=(a.community_id,a.workflow_id)
UNION ALL SELECT 'subscriptions',community_id,to_jsonb(id),encode(owner_pubkey,'hex'),jsonb_build_object('delivery_url',delivery_url),NULL,false,true FROM subscriptions
UNION ALL SELECT 'delivery_log',l.community_id,jsonb_build_array(l.delivered_at,l.id),encode(s.owner_pubkey,'hex'),jsonb_build_object('error_message',l.error_message),NULL,false,true
 FROM delivery_log l LEFT JOIN subscriptions s ON (s.community_id,s.id)=(l.community_id,l.subscription_id)
UNION ALL SELECT 'product_feedback',community_id,to_jsonb(id),encode(submitter_pubkey,'hex'),jsonb_build_object('body',body,'tags',tags),NULL,false,false FROM product_feedback
UNION ALL SELECT 'channels',community_id,to_jsonb(id),NULL,jsonb_build_object('canvas',canvas,'description',description,'topic',topic,'purpose',purpose),NULL,false,false FROM channels
UNION ALL SELECT 'communities',id,to_jsonb(id),NULL,jsonb_build_object('icon',icon),NULL,false,false FROM communities
UNION ALL SELECT 'moderation_reports',community_id,to_jsonb(id),encode(reporter_pubkey,'hex'),jsonb_build_object('note',note),encode(target_blob_sha256,'hex'),false,false FROM moderation_reports
UNION ALL SELECT 'moderation_report_events',r.community_id,to_jsonb(r.id),encode(r.reporter_pubkey,'hex'),jsonb_build_object('content',e.content,'tags',e.tags),NULL,false,false
 FROM moderation_reports r JOIN events e ON e.community_id=r.community_id AND e.id=r.target_event_id
UNION ALL SELECT 'moderation_actions',community_id,to_jsonb(id),encode(actor_pubkey,'hex'),jsonb_build_object('public_reason',public_reason,'private_reason',private_reason),NULL,false,false FROM moderation_actions
UNION ALL SELECT 'relay_admin_actions',report_community_id,to_jsonb(id),encode(actor_pubkey,'hex'),jsonb_build_object('reason',reason,'error_message',error_message),NULL,false,false FROM relay_admin_actions
UNION ALL SELECT 'relay_admin_outbox',a.report_community_id,to_jsonb(o.id),encode(a.actor_pubkey,'hex'),jsonb_build_object('payload',o.payload,'error_message',o.error_message),NULL,false,false
 FROM relay_admin_outbox o LEFT JOIN relay_admin_actions a ON a.id=o.action_id
UNION ALL SELECT 'artifact_heads',h.community_id,to_jsonb(h.artifact_id),NULL,jsonb_build_object('content',e.content,'tags',e.tags),NULL,false,false
 FROM artifact_heads h JOIN events e ON e.community_id=h.community_id AND e.id=h.event_id WHERE NOT h.deleted
UNION ALL SELECT 'media_uploaders',community_id,jsonb_build_array(sha256,encode(pubkey,'hex')),encode(pubkey,'hex'),'{}'::jsonb,sha256,true,true FROM media_uploaders
UNION ALL SELECT 'audit_log.media_uploaded',community_id,to_jsonb(seq),encode(actor_pubkey,'hex'),jsonb_build_object('sha256',detail->'sha256'),object_id,true,true FROM audit_log WHERE action='media_uploaded'

UNION ALL SELECT 'workflow_trigger_runs',r.community_id,to_jsonb(r.id),r.trigger_context->>'author',jsonb_build_object('trigger_context',r.trigger_context,'execution_trace',r.execution_trace,'error_message',r.error_message),NULL,false,false FROM workflow_runs r WHERE r.trigger_context->>'author' IS NOT NULL
