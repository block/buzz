# Operator-listener thread watches

A configured operator listener may receive plain replies to an explicitly watched
thread without requiring every reply to address its target with a `p` tag. There
is no channel-wide or community-wide subscription. Existing mention delivery is
unchanged, and an event matching both paths produces one outbox delivery per
listener/target/community/event.

This uses the existing deployment control plane because listener registration
belongs to a configured delivery endpoint, outside normal community event ingest.
The target still provides fresh signed consent, and the relay remains delivery
infrastructure: this adds no agent scheduler or publisher.

## Register or renew

Send `POST /operator/listener/threads` with the existing configured listener's
NIP-98 authentication, signed for the exact
`RELAY_OPERATOR_API_ORIGIN + /operator/listener/threads`, including the exact
request-body hash. The existing replay guard applies. The body's `registrations` array contains exactly **one full signed Nostr event object**.
Use the existing `KIND_OPERATOR_LISTENER_REGISTRATION` kind. The event is signed
by the target agent and has exactly one `p` tag naming the configured listener
and one `u` tag naming that same canonical endpoint. Its timestamp must be within
60 seconds. Its signed `content` is JSON with exactly these fields:

```json
{
  "community_id": "community UUID",
  "channel_id": "channel UUID",
  "root_event_id": "64 lowercase hexadecimal characters"
}
```

The target must already have a current listener-pubkey registration. The target
must be a current, active channel member with an active user record. The root
must be an undeleted top-level message in that exact community and channel;
registering a reply as a root is refused. Scope checks occur in a community-
admitted transaction. Nil/foreign identifiers do not grant access.

Success returns `{"expires_after_seconds":2592000}`: a fixed **30-day** lifetime,
matching the existing target registration. At most 128 bindings are retained per
listener/target/community; renewal of an existing binding is allowed at capacity.
An unknown request outcome is retried with fresh NIP-98/proof events for the same
tuple. One tuple per request makes persistence atomic.

Renew owned active conversation bindings through the client's existing listener
registration maintenance; no additional poller is required. Expiry is real:
a plain reply after both the watch/target lease have lapsed cannot reactivate
its own delivery. The client must renew before expiry or establish a fresh
explicitly addressed conversation. Before a reviewed ownership cutover, known
legacy thread bindings may be explicitly seeded from the reconciled operator
inventory through this same target-signed API. Seeding changes no event,
identity or history and does not infer employee authority.

`DELETE /operator/listener/threads` uses the same listener NIP-98 authentication
and body `{community_id, channel_id, root_event_id, pubkey}`. `pubkey` is the target
agent. Removal relinquishes only that listener's exact tuple; it cannot add
subscriptions or remove another listener's binding. Stop renewal on conversation
closure, agent deletion or ownership loss. Expired rows are cleaned in bounded
community-admitted batches on the existing outbox-reaper cadence; community
purge owns rows belonging to a deleting community.

## Delivery and recovery

Only a committed reply whose canonical thread metadata matches the exact
community/channel/root may enqueue. The target's own output, root messages, other roots, channel chatter
and other communities do not match. Current target membership, active user/root
and registration/watch expiry are checked again before delivery. Removing a
member after enqueue suppresses delivery, including from an open channel.

Thread-only delivery uses the existing durable outbox, event uniqueness,
visibility claims, retry policy, NIP-98 callback signing and signed v2 `event`
payload. Its signed payload adds optional `thread_root_event_id` (lowercase hex).
An explicit `p` match takes priority and retains its original payload shape.
The consumer must independently verify the event signature, channel/root and its
own durable watch/requester binding. A watch grants no employee/tool authority.

This does not backfill events predating registration and does not extend the
outbox's 15-minute retention or nine-attempt retry policy. Clients needing outage
recovery must issue bounded standard signed queries for their watched thread and
feed the same durable intake path. Do not widen webhook freshness as recovery.

## Rollout and validation

Migration 0080 is an **expand**: one tenant-owned table/index set and one nullable
outbox metadata column; no trigger, function, foreign key or data backfill. Apply
schema through the deployment's reviewed workflow before new code. The older
relay ignores the new table/column, but its exact-set community-deletion catalog
will refuse deletion until all new code is deployed. Avoid community deletion
during that short expand/deploy gap. Auto-migration rollback to an older binary
requires `BUZZ_AUTO_MIGRATE=false`, as for any unknown newer migration.

Deploy support on all delivery workers and the compatible consumer **before**
registering watches. Old workers omit the new classification and old consumers
may reject plain replies; mixed-version delivery is not a supported activation.
No existing watches or registrations are enrolled by this change. Verify schema
parity, positive plain-thread delivery, wrong root/channel/community/nonmember,
expiry/deactivation/deletion, duplicate mention+watch, retry and removal against
an isolated local relay before rollout. A human must also test the flow before
marking the draft ready, per the repository's contributor checklist.
