# Multiverse M02: authenticated intent, not execution

Stacks on M01 (`docs/multiverse-placement.md`). This is an inert codec for
`kind:50003`, `L=buzz.placement.v1`, with owner-self NIP-44 v2 ciphertext.
The relay's `required_scope_for_kind` still rejects this kind. No producer,
receiver, new login, key distribution, runtime effect or configuration is enabled.

## Representation and trust

Reuse the **audience model**, not the replaceable semantics, of
`crates/buzz-core/src/private_managed_agent.rs::build_event` and
`validate_and_decrypt`: the owner encrypts to itself and signs once. Authorized
Desktop instances already holding that owner's keys can decrypt the same event,
including Start Y observed by X. Executor keys alone cannot read it. Neither a
profile nor possession of a host key grants owner credentials. There is no
blanket group key, per-recipient re-signing, generic signer or runtime key export.
An executor without an existing authenticated owner Desktop context is unsupported;
this work does not invent credentials for it.

The encrypted payload is exactly version, canonical community, owner, agent,
target host, request UUID and Start/Stop action. Only author, kind, namespace and
signed timestamp are exposed outside ciphertext. Host means stable executor,
not process/run. Unknown fields, duplicate struct fields, invalid/noncanonical
community, nil request, unknown action/version and legacy exact-run objects fail
closed. Restart is not accepted as Start: it remains a separately deduplicated
current-host operation. No Move future-Start template is serialized.

`decode_event` checks event hash **and** signature before decryption and binds
owner/community/agent/target to caller-supplied authoritative scope. It returns
request metadata separately from an M01 contribution whose host, action and
order come from that single verified event. Caller-supplied bindings are not
proof created by the codec. M01 remains explicitly non-authorizing.

Newer signed sender seconds wins, lower event ID breaks ties. There is no
expiry or skew check on historical desired state, no relay order, lease or
logical clock. Rebuilding randomizes ciphertext and changes event identity:
persist exact signed bytes before publication, retry those bytes only. A shared
request UUID does not make two differently signed events the same ordering fact;
future admission must reject request collisions, not pick by arrival.

## Concrete integration path (not implemented here)

1. Native producer: reuse the captured owner/relay pattern in
   `desktop/src-tauri/src/managed_agents/retention.rs::active_retention_scope`
   and `persona_events.rs::flush_pending_events_at`. Canonicalize with
   `buzz_core::relay::normalize_relay_url`, validate authoritative agent ownership
   and executor binding, build once, persist exact bytes. Do **not** reuse persona
   NIP-33 upserts or redating: immutable commands are not replaceable definitions.
2. Relay: extend existing author-only filter/result/live/COUNT gates, storage
   search exclusion and owner/global ingest validation together, before admitting
   50003. `handlers/req.rs::{author_only_filters_authorized,is_author_only_event}`
   already implement the audience shape. Ciphertext is not metadata privacy.
   Transport owner authentication and community isolation remain unchanged.
3. Receiver/history: capture the same native owner context, not host-key login.
   `relay.rs::query_relay_at_with_keys` is the existing explicit-key query seam;
   add bounded no-redirect private queries and honest errors. Query explicit kind
   + owner across **all** relevant targets, not destination X only. Agent/host
   fields are encrypted: paginate owner history before local scoped decoding,
   never treat a page without matching agent rows as exhaustion.
   `buzz-db/src/store/event.rs::EventQuery` already has `until` + `before_id`
   (`created_at DESC, id ASC`); complete dense-tie paging and overlapping live
   intake remain required. No new history database or relay sequencer is needed.
4. Authoritative bindings must distinguish valid historical intent from current
   effect authorization. Revocation/compaction must not drop an old stop or
   selection and revive a prior host. Preserve authenticated projection fences;
   never turn read/binding errors into absence. Current bindings are rechecked
   before effects. Scope switches discard decrypted state and fence in-flight work.
5. Durable admission/retention and ordinary Desktop controls precede enablement:
   history projects state, never replays Start/Stop/Restart. Reconnect can stop a
   superseded copy, not resume interrupted operations. Move waits for ordinary
   Stop success; failed/unconfirmed/late results never release automatic Start Y.

Next executable slice is relay owner-private admission/read coverage plus its
schema search exclusion, retaining disabled runtime consumers. Then combine
native scoped history + binding adapter where the measured diff permits, before
journal/control integration. The earlier M01–M17 outline is not a fixed PR count;
profile/inventory and sender/result splits should be remeasured, not scaffolded
in advance. Preserve the existing `remote-start-preview` convention when native
wiring first appears; keyless lifecycle compatibility and native acceptance are
release gates, not claims of this codec PR.
