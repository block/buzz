SELECT EXISTS(SELECT 1 FROM reactions WHERE community_id=$1 AND
    (event_id=decode($2->>'id','hex') OR reaction_event_id=decode($2->>'id','hex')))
OR EXISTS(SELECT 1 FROM event_mentions WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM push_match_queue WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM push_wake_outbox WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM operator_listener_outbox WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM delivery_log WHERE community_id=$1 AND event_id=decode($2->>'id','hex'))
OR EXISTS(SELECT 1 FROM workflow_runs WHERE community_id=$1 AND
    (trigger_event_id=decode($2->>'id','hex') OR lower(trigger_context->>'message_id')=$2->>'id'))
OR EXISTS(SELECT 1 FROM events e CROSS JOIN LATERAL (
    SELECT decode(t->>1,'hex') AS id FROM jsonb_array_elements(e.tags) WITH ORDINALITY a(t,n)
    WHERE t->>0='e' AND t->>1 ~ '^[0-9a-fA-F]{64}$'
    ORDER BY CASE WHEN e.kind IN (40003,45002) THEN n ELSE -n END LIMIT 1) target
    WHERE e.community_id=$1 AND e.kind IN (7,40003,45002) AND target.id=decode($2->>'id','hex')
      AND e.channel_id IS NOT DISTINCT FROM
        (SELECT channel_id FROM events WHERE community_id=$1 AND id=decode($2->>'id','hex')
          AND created_at=($2->>'created_at')::timestamptz))
