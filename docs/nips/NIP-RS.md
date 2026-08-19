NIP-RS
======

Cross-Device Read State Sync
-----------------------------

`draft` `optional`

## Abstract

This NIP defines a scheme for synchronizing a user's own per-context read state
(e.g., "I have read this channel up to timestamp T") across multiple client
instances belonging to that same user, using encrypted `kind:30078` events.

This NIP is not a read-receipt protocol. It does not expose what another user
has read, and it does not tell other users what messages the current user has
read.

## Motivation

A user running Nostr clients on multiple devices (phone, desktop, web) has no way to share read position across those clients. Each instance independently tracks what has been read, causing already-read content to appear unread on other devices.

This NIP defines a minimal, privacy-preserving protocol for propagating read state across client instances without requiring a new event kind, a new wire message, relay-stored read-state logic, or coordination between different client implementations. It is not free of relay obligations: a relay serving the manual-unread override layer's full-state load must satisfy the ordering, capacity, floor, push, and barrier contract that section enumerates.

## Non-Goals

This NIP does not define a durable log of all read messages — frontier blobs are best-effort recent activity hints bounded by a time horizon. Exception: `ov_*` override entries, including tombstone floors, are durable state — they are exempt from age pruning, budget eviction, and horizon-bounded fetching, they live in a single coordinate per installation, and they MUST be carried forward before that coordinate is deleted or abandoned (see Manual-Unread Override Layer — Override State Durability).
This NIP does not define cross-client interoperability on context ID format — context identifiers are opaque by default and meaningful only within a single client family, except for OPTIONAL well-known schemes defined in this NIP (`thread:<root-event-id>` and `msg:<event-id>`, defined under Read Context Schemes), which are provided for cross-client thread/message-read interoperability.
This NIP does not guarantee ordering of read events across devices.
This NIP does not require relay-stored read-state logic: no new event kind, no new wire message, and nothing a relay must interpret about read state. Clients implementing the manual-unread override layer do depend on relay behaviour their full-state load cannot verify (see Full-State Load).
This NIP does not define read receipts, seen-by lists, or any mechanism for
tracking what other users have read.

## Specification

### Event Structure

Clients publish a `kind:30078` addressable event (per [NIP-78](78.md)) with the following structure:

```json
{
  "kind": 30078,
  "pubkey": "<user-pubkey>",
  "created_at": 1700000000,
  "tags": [
    ["d", "read-state:<slot-id>"],
    ["t", "read-state"]
  ],
  "content": "<nip44-encrypted-json>"
}
```

#### `d` Tag

The `d` tag MUST be `read-state:<slot-id>`, where `<slot-id>` is exactly 32 lowercase hexadecimal characters (`[0-9a-f]{32}`), generated randomly by the client on first launch and persisted locally. The `<slot-id>` has no relationship to the `client_id` — it is solely a unique key for NIP-33 addressable event semantics. The shape is fixed rather than opaque so that a relay can recognize a read-state coordinate structurally, from the `d` tag alone and without decrypting anything, and apply per-coordinate protections to it; a client that picks some other shape is not merely stylistically different, it forfeits those protections silently.

**Primary coordinate:** a client MUST designate one coordinate as its **primary** and MUST use a single stable, unique `<slot-id>` for it for the lifetime of that installation. The primary `<slot-id>` changes only on a `client_id` conflict (below) or rotation (see Client-ID Rotation).

**Additional frontier-only coordinates:** a client MAY publish additional coordinates under distinct `<slot-id>` values when its primary blob would otherwise exceed the size budget. Additional coordinates MUST NOT contain `ov_*` entries — they carry frontier entries only, and are therefore freely rewritable and freely deletable (see Orphaned Blob Deletion). A client MUST persist the `<slot-id>` values of its additional coordinates locally so that it can rewrite and delete them.

**All `ov_*` entries, and the frontier entries of the contexts they belong to, MUST live in the primary coordinate.** A client implementing the manual-unread override layer MUST NOT distribute `ov_*` entries across coordinates and MUST NOT move them between coordinates: there is exactly one override-bearing coordinate per installation.

If a client fetches its own `d` tag coordinate and the decrypted `client_id` does not match its local `client_id`, the coordinate is conflicted. The client MUST NOT publish to that coordinate and MUST generate a new random `<slot-id>` before the next publish.

Events with zero `d` tags MUST be ignored.
Events whose `d` tag value does not begin with `read-state:` MUST be ignored.
Events with more than one `d` tag MUST be ignored.
Events whose `<slot-id>` is not exactly 32 lowercase hexadecimal characters MUST be ignored.

Recognizable coordinates also serve the accumulation discipline this NIP depends on: a relay that can identify a read-state coordinate structurally can replace superseded versions outright instead of retaining a tombstone row per publish, which keeps the coordinate count a full-state load must enumerate near one per live installation (see Full-State Load).

#### `t` Tag

Events MUST include exactly one `["t", "read-state"]` tag. The tag is a discoverability marker: it lets a client express "read-state events only" in a single filter. It is not a guarantee of relay-side selectivity — a relay MAY apply tag constraints after its result cap, and `kind:30078` is shared with unrelated application data — so clients MUST apply the tag as a correctness filter locally on everything they receive, and MUST NOT infer from a short result that no further coordinates exist. A client performing a full-state load MUST omit the tag from its filter entirely (see Full-State Load).

Events with zero `t` tags with value `read-state`, or more than one `t` tag with value `read-state`, MUST be ignored.

#### Content

The `content` field MUST be a [NIP-44](44.md) ciphertext. The NIP-44 conversation key MUST be computed as `nip44_conversation_key(user_privkey, user_pubkey)` — the user's private key as the local party and their own public key as the remote party.

The plaintext MUST be a JSON object of the following form:

```json
{
  "v": 1,
  "client_id": "<client-id>",
  "contexts": {
    "<context-id>": <unix-timestamp>
  }
}
```

- `v` is an integer schema version. Clients MUST ignore blobs with unknown `v` values.
- `client_id` is a non-empty UTF-8 string of 1–64 characters identifying this client instance. It is generated on first launch and persisted locally. Each client instance MUST use a stable, unique `client_id`. This field is the only link between a blob and the device that owns it; it is never visible to relay operators.
- Keys under `contexts` are arbitrary UTF-8 strings identifying a readable context (e.g., a channel, group, or conversation). This NIP does not prescribe context identifier format.
- Values are unix timestamps (integer seconds) representing "all messages in this context at or before this time have been read."

Unknown top-level keys in the JSON object SHOULD be ignored for forward compatibility.

#### Content Validation

After decryption, clients MUST apply the following validation rules:

- Events whose `content` does not decrypt to valid JSON MUST be discarded.
- Events with a missing or non-integer `v` field MUST be discarded.
- Events with an unknown `v` value MUST be ignored.
- Events with a missing `client_id` field MUST be discarded.
- Events with a `client_id` that is not a non-empty string of 1–64 UTF-8 characters MUST be discarded.
- Events with a missing `contexts` field MUST be discarded.
- Events whose `contexts` field is not a JSON object MUST be discarded.
- Individual context entries whose timestamp is not an integer in the range 0–4294967295 MUST be discarded (the entry is dropped; the rest of the blob is still processed).
- Individual context entries whose context ID exceeds 256 bytes MUST be discarded.
- Override counter entries (keys beginning with `ov_s:`, `ov_c:`, or `ov_b:`) MUST be validated as a complete logical group BEFORE any decoding, zero-filling, merging, or canonicalizing. Clients MUST collect all `ov_s:`, `ov_c:`, and `ov_b:` entries for the same `<ctx>` suffix together before processing them. The only accepted wire shapes for an override group are: (a) a complete live group containing exactly the three keys `ov_s:<ctx>`, `ov_c:<ctx>`, and `ov_b:<ctx>` with valid uint32 values, or (b) a tombstone floor containing only `ov_c:<ctx>` with a valid uint32 value. Any other shape (partial group, extra keys, or invalid value in any sibling) MUST cause the entire override group to be rejected; the corresponding frontier entry for `<ctx>` MUST be retained. Applying the generic per-entry discard rule before group collection is prohibited for override entries.
- Blobs containing more than 10,000 context entries MUST be rejected.
- If a blob contains duplicate context keys, clients SHOULD use the last value encountered (consistent with RFC 8259 §4).
- Clients SHOULD ensure the total serialized event does not exceed the relay's maximum event size (commonly 64 KB per NIP-01). Clients receiving events that exceed their configured size limit SHOULD discard them.

#### Context Identifiers

Context identifier format is not prescribed by this NIP. Clients choose identifiers appropriate to their context type (e.g., a NIP-28 channel event ID, a NIP-29 group address, a pubkey for DMs). Interoperability between different client implementations on context ID conventions is outside the scope of this NIP.

#### Reserved Namespace

The key prefix stem `ov_` (3 bytes) and the escape marker `esc:` (4 bytes) are reserved for the manual-unread override layer defined below. Clients MUST escape any raw context ID that begins with `ov_` or `esc:` when using it as a frontier key in the `contexts` map:

- **On publish:** prepend `esc:` to any raw context ID beginning with `ov_` or `esc:` before writing it as a frontier key (e.g., raw `ov_s:evil` → wire key `esc:ov_s:evil`; raw `esc:foo` → wire key `esc:esc:foo`).
- **On receive:** strip exactly one leading `esc:` from any frontier wire key beginning with `esc:` to recover the raw context ID (e.g., wire key `esc:ov_s:evil` → raw `ov_s:evil`; wire key `esc:esc:foo` → raw `esc:foo`). This is a bijection — applying escape then unescape is the identity function. Clients MUST NOT strip more than one `esc:` prefix per receive.

**Backward-compatibility limitation:** a context published *unescaped* by a client predating this amendment, whose raw ID happens to start with `ov_` or `esc:`, is not safely migrated. The scheme protects contexts generated by amendment-aware clients going forward; it does not retroactively rewrite history. This residual hazard is documented as a known limitation. Buzz's own context ID shapes (channel UUID, `msg:hex64`, `thread:hex64`) cannot trigger it.

#### Read Context Schemes (Optional)

This subsection defines OPTIONAL well-known context schemes for tracking read
state below a channel: thread-level read state for reply chains and per-message
read state for individual events. These schemes are pure interpretation layers
over the flat `contexts` map — they introduce no new fields, no nesting, and no
change to the merge rule, event structure, validation, or fetching. The schema
version `v` remains `1`. Clients that do not implement this subsection remain
fully interoperable (see Backwards Compatibility below).

A client implementing thread read contexts MUST use the context key
`thread:<root-event-id>` for a thread, where `<root-event-id>` is the
64-character lowercase hex event ID of the thread's root event.

A client implementing per-message read contexts MUST use the context key
`msg:<event-id>` for a message, where `<event-id>` is the 64-character lowercase
hex event ID of that message. Per-message contexts are useful for clients that
reveal only part of a thread (for example, a thread panel with collapsed nested
branches): a client can mark the revealed reply events read without marking the
whole thread read.

A bare channel identifier (e.g., the NIP-28 channel event ID) remains the channel
context, exactly as before — this is grandfathered existing behavior and is
unchanged.

Keys beginning with `thread:` whose remainder does not match `^[0-9a-f]{64}$`
MUST be treated as ordinary opaque contexts, not as thread contexts. Keys
beginning with `msg:` whose remainder does not match `^[0-9a-f]{64}$` MUST be
treated as ordinary opaque contexts, not as per-message contexts. This protects
an existing client family that may already use one of these prefixes from being
misinterpreted under this scheme.

The relationship between a thread or message and its parent channel is DERIVED
from the Nostr event graph at evaluation time (the root/message event's channel
reference, e.g. its `h` tag) and MUST NOT be serialized into the blob. The blob
remains a flat `{<context-id>: <unix-timestamp>}` map.

##### Hierarchical Frontier Rule

The effective read frontier of a context is the maximum of its own merged
timestamp and the effective frontier of its parent:

```
effective(ctx) = max(merged[ctx], effective(parent(ctx)))
```

A channel has no parent, so its effective frontier is simply its own merged
value. For a thread, the parent is its channel:

```
effective(thread:<root>) = max(merged[thread:<root>], merged[<channelId>])
```

For a per-message context, the parent is also its channel (not its thread or
parent message):

```
effective(msg:<event-id>) = max(merged[msg:<event-id>], merged[<channelId>])
```

When a surface evaluates a reply inside a known thread, it MAY additionally fold
in the thread frontier:

```
effective(reply) = max(effective(msg:<reply-id>), effective(thread:<root>))
```

A thread is unread iff at least one reply is unread. A reply is unread iff
`reply.created_at > effective(reply)` for clients that implement per-message
contexts; clients that only implement thread contexts MAY instead use
`latestReplyAt > effective(thread:<root>)`. Because both rules are `max()` over
the same grow-only registers defined in the Merge Rule, they remain monotone
state-based CvRDT interpretations — no change to the merge rule is required.
Marking a channel read clears unread state on any thread/message whose relevant
event predates the channel frontier, since each child context inherits the
channel term; replies newer than the channel frontier remain unread until their
own message marker or thread marker is advanced.

If the thread root or message event (and therefore its parent channel) cannot be
resolved from the event graph, `effective(thread:<root>)` or
`effective(msg:<event-id>)` degrades to its own merged value alone.

##### Write Discipline

Marking a thread read MUST advance only its own `thread:<root>` context.
Marking an individual message read MUST advance only its own `msg:<event-id>`
context. Neither operation may advance the parent channel context. Otherwise,
reading a single thread or reply would silently mark later top-level channel
messages as read. Marking a channel read advances only the channel context
(which the hierarchical rule then propagates to child contexts at read time).
The channel context SHOULD advance to the maximum `created_at` across the
channel's top-level messages only, NOT including thread replies. This keeps a
thread unread when its replies exceed the newest top-level message: opening a
channel clears the channel timeline but leaves its threads/replies unread until
each thread or message is read.

##### Eviction

A `thread:<root>` or `msg:<event-id>` entry whose value is
`<= effective(parent)` is semantically inert: the parent (channel) frontier
already covers it, so its presence or absence does not change the result of the
child context's effective frontier. Clients MAY drop such dominated entries
before publishing to bound blob size, consistent with the Debounce and Pruning
section.

This eviction is bounded best-effort, NOT a guaranteed garbage-collection or
per-key tombstone mechanism. Because the merge rule re-merges any context
present in another instance's blob (see Merge Rule and Live Subscription and
Convergence), a dropped `thread:<root>` or `msg:<event-id>` key MAY be
re-added by a peer instance that still carries it. A dropped key stays gone only
once it is dominated on every instance or has aged past the time horizon
everywhere. Clients SHOULD treat child-context eviction as a companion to the existing time-horizon
pruning, not as a standalone guarantee that the context count or blob size will
shrink immediately.

To avoid a re-publish loop with peers that still carry an evicted key, an
incoming context entry whose value is `<= effective(parent(ctx))` MUST NOT by
itself trigger a re-publish. Clients SHOULD evaluate the Live Subscription
re-publish trigger and the suppression comparison (Live Subscription and
Convergence rules 2–3) AFTER applying their eviction policy, so that re-merging
a dominated key a peer still carries does not force a write that changes nothing
semantically. This is backwards-safe: it only suppresses writes with no semantic
effect.

##### Backwards Compatibility

A client that does not implement this scheme treats `thread:<root>` and
`msg:<event-id>` keys as ordinary opaque contexts. It carries the keys through
the merge unchanged (already required by the Merge Rule) and simply computes no
thread/message-level unread state. There is no validation change and no interop
break: an unaware client and an aware client can share a blob and both produce
correct results for the contexts they understand.

##### Example

Two blobs merge to the following effective state for a symbolically named thread
`X` and its parent channel (real `thread:` keys use 64-character lowercase hex
event IDs):

```json
{
  "thread:X": 100,
  "<channelId>": 150
}
```

The thread's effective frontier is computed through the channel parent term:

```
effective(thread:X) = max(merged[thread:X], merged[<channelId>])
                    = max(100, 150) = 150
```

A thread reply with `created_at = 140` is `<= 150`, so it reads as **read** (the
channel frontier already covers it). A reply with `created_at = 160` is `> 150`,
so the thread reads as **unread**. The thread's own entry (`100`) is dominated by
the channel frontier (`150`) and is therefore inert — a client MAY evict it
before publishing. The same rule applies to `msg:<event-id>` entries for
individual replies.

#### Timestamp Accuracy

Clients SHOULD use the `created_at` of the message being marked as read as the context timestamp — not the local wall clock and not the relay receive time.
Clients SHOULD ensure timestamps within a context are monotonically non-decreasing.

Because context timestamps are derived from message `created_at` values — which are author-controlled in Nostr — a message with a future-dated or skewed `created_at` can advance the read frontier beyond the actual read position. This is an accepted limitation of timestamp-based read state. Clients MAY implement local safeguards such as capping context timestamps at the current wall clock time, but this NIP does not mandate such behavior.

### Fetching

To load read state, a client MUST fetch all `kind:30078` events for the user. Unless it is performing a full-state load (below), it SHOULD narrow the fetch with the `#t` filter:

```json
{"kinds": [30078], "authors": ["<user-pubkey>"], "#t": ["read-state"]}
```

Clients that neither read nor write `ov_*` override state SHOULD limit the fetch to events with `created_at` within a configurable time horizon (default: 7 days) by adding `"since": <now - horizon>`, accepting that frontiers older than the horizon become unknown. Clients that implement the manual-unread override layer MUST NOT filter the fetch by age or by tag, and MUST establish completeness (see Full-State Load below).

After fetching, clients MUST:

1. Decrypt each blob.
2. Discard blobs that fail validation (see Content Validation).
3. Identify the blob whose decrypted `client_id` matches the client's own `client_id` — this is the client's own blob.

If multiple blobs decrypt to the same `client_id` (e.g., due to a prior rotation that left an orphaned blob, or a backup/restore that duplicated identifiers), the client MUST treat the blob at its own primary coordinate as its own and merge all others into the read state as if they were from other instances. If none of them is at the client's own primary coordinate, the blob with the highest `created_at` is its current reference. Deletion of such a stale duplicate is governed by Orphaned Blob Deletion — a duplicate carrying `ov_*` entries MUST NOT be deleted until its override state has been carried forward.

4. Merge all valid blobs (including the client's own) using the merge rule.

Absence of a context in all fetched blobs means the read state for that context is **unknown** — clients SHOULD treat unknown contexts as unread (conservative default). For clients using a finite horizon, the horizon is a storage and fetch optimization, not a semantic claim about read status: contexts that were read but have aged out of the time horizon are indistinguishable from never-read contexts. Clients MAY extend the horizon or maintain a local cache to mitigate this. Clients implementing the override layer do not filter the fetch by age at all (see above), so for them this ambiguity arises only from write-time frontier pruning.

#### Full-State Load

Clients that implement the manual-unread override layer MUST perform a **full-state load**: they MUST NOT apply a finite `since` filter, and they MUST establish that every one of the user's `read-state` coordinates has been retrieved. Because the payload is encrypted, a relay filter cannot select for override-bearing events: any event-level window can exclude the only coordinate carrying a tombstone floor, which reopens the resurrection witness in Override State Durability regardless of any per-entry exemption. For these clients the time horizon is a *write-time* frontier pruning policy only (see Debounce and Pruning), never a fetch filter.

Removing `since` does not by itself make the result complete. Relays MAY cap the number of events returned for a historical query, MAY cap below the client's requested `limit`, and emit end-of-stored-events after the capped query — so **a single query proves nothing.** End-of-stored-events marks the end of the capped result, not the end of the matching set, and a short result does not establish that no further coordinates exist. Caps typically retain the newest events and drop the oldest, which are precisely the rotation-predecessor and orphaned coordinates whose tombstone floors this layer depends on. A silently truncated load that omits the sole carrier of a floor merges a stale live register unopposed and reports a manually-unread context as read, permanently.

A full-state load MUST therefore be enumerated with no tag constraint in the filter:

```json
{"kinds": [30078], "authors": ["<user-pubkey>"], "limit": <n>}
```

A relay MAY deliver fewer events than its result cap selected — for example, by applying tag constraints only after the cap and withholding the events that fail them. Under a tag-constrained filter the number of events the client receives is therefore not the number the cap selected: a delivered page can be short, or empty, while older matching coordinates still exist below it, and no observation the client can make distinguishes the two. `kind:30078` is arbitrary application data whose `d` tag namespace is open to every application that has ever written under the user's key, so this is not a hypothetical — a page can be filled entirely by coordinates unrelated to read state. With the tag constraint omitted, the client asks for exactly what it will accept, and the events the cap selects are the events it receives. Selection moves client-side, which is where the validation rules already place it: collect coordinates only from `d` tags of the form `read-state:<slot-id>` and ignore every other event. The cost is that the client fetches its own application data at that kind rather than a relay-selected subset of it.

A client MUST NOT test completeness by comparing the number of events returned against the `limit` it requested: the effective cap is the relay's, a relay MAY cap below the requested value, and a relay's advertised maximum limit is not necessarily the limit it enforces — so no comparison against the requested `limit` is a valid truncation test. What the client MAY compare is one delivery against another. Let `C` be the largest number of events the relay delivered for any single **preceding** query in this load. The relay demonstrably delivered `C` events at once, so its cap is at least `C`, and a query that delivers fewer than `C` events was not cut short by that cap. Completeness is established by continuation on a strictly decreasing cursor, with each band discharged by that comparison.

`C` yields nothing at the start of a load, and yields nothing for the whole of a load whose entire history at this kind is a single event: one delivery of one event bounds the cap below by one, and no delivery can be smaller than that. The procedure therefore also fixes a floor, `L = 2`, required of relays below. A delivery is bounded by the requested `limit` as well as by the relay's cap, so the floor licenses a conclusion about a delivery only together with step 1's requirement that the requested `limit` be at least `L`: what the client may conclude is that a query it issued for at least `L` events, whose matching set holds at least `L`, delivers at least `L`. A threshold above the requested `limit` would be unreachable by construction, and a test that can never be met declares a truncated page exhausted. It is stated at the smallest value that admits a second event, because a larger floor is a stronger claim about relays that buys nothing further: a relay that will deliver only one event per query cannot express `limit` semantics and cannot serve a user who has two coordinates at all, whereas any floor above two would begin excluding relays this procedure does not need to exclude.

Enumeration descends, so it cannot see a coordinate that moves *up* while it runs. Because these are addressable events, a republish replaces the previous version rather than appending: a coordinate the client already collected at a low `created_at` can be replaced, during the load, by a version above the cursor the client has already passed — and the old version stops existing, so no continuation and no pinned window will ever return either one. If that new version carries a tombstone floor the old one lacked, a load that reported *complete* merged without it.

A full-state load therefore MUST be fenced by a live subscription on the same tag-free filter, established **before** the first enumeration query and held unbroken for the duration of the load. The fence is *established* when the client has received end-of-stored-events for that subscription, not when it sent the request: sending a request is not an observation, and the relay's answer to it is the first point at which the client knows the subscription is registered and that what the relay accepts from then on will be pushed to it. The fence and every enumeration query MUST be issued on the same connection.

Every event the fence delivers is collected exactly as an enumerated event is (step 2), which is what repairs the moved coordinate: the replacing event is itself what the relay pushes. Delivery is necessary but not sufficient — it must be delivery *before the verdict*, and those are different properties. Under push delivery alone, a relay that accepts a replacement, removes the version sitting below the cursor, and pushes the replacement some time later has violated nothing: the enumeration in between finds neither version, the pinned window and the continuation both come back empty, and the load reports *complete* moments before the fence delivers the floor it was missing. Ordering that push ahead of the verdict is what the delivery barrier below requires of the relay. On the client side, the verdict MUST NOT be rendered until end-of-stored-events for the final continuation has been received and every fence delivery received before it has been collected.

If the client did not hold such a subscription for the whole load, or it lapsed or reconnected at any point during it, the load is potentially incomplete regardless of what the enumeration returned. A client MUST NOT publish to its own coordinates while its own load is in progress; a self-inflicted replacement is the same defect with the client on both ends of it.

1. Every query MUST carry the same explicit `limit` `n`, `n` MUST be at least `L`, and no query MUST constrain tags. `C` and the floor are only meaningful across queries that differ solely in their time bounds. `n` SHOULD be substantially larger than `L`: `n` bounds how many events a single band can retrieve, so a small `n` costs round trips without making any verdict safer.
2. From each delivered event, collect the read-state coordinates — those whose `d` tag has the form `read-state:<slot-id>` — deduplicating by `d` tag value and retaining, of the entries sharing a `d` tag value, the one with the greatest `created_at`, and on equal `created_at` the one with the lexicographically lowest event id. That is the addressable ordering NIP-01 defines, and both halves of it are load-bearing here: a replacement published in the same second as the version it replaces is legal and is the one the relay retains, so a retention rule that only compares `created_at` may keep the superseded version even when the fence delivered its successor perfectly. Ignore the other events, but count them: they are part of what the cap returned. Events the fence delivers are collected the same way, but MUST NOT contribute to `T` or to `C`: they are not a query result, and an event arriving below the cursor would otherwise move it down and skip the band between. Collection is what recovers a moved coordinate; the cursor descends on query results alone.
3. Let `T` be the lowest `created_at` across **all** events delivered by the queries in this load, not only the read-state ones. The cursor therefore advances on every non-empty page, including one that yielded no coordinate.
4. Before advancing past `T`, the client MUST query the pinned window `{"since": T, "until": T}` and merge the result. A cap can cut mid-second, leaving events at `T` that a continuation at `"until": T - 1` would skip forever; pinning both bounds to one second removes every event outside that second from contention for the cap.
5. Second `T` is exhausted only if that pinned query delivered fewer than `max(C, L)` events. If it delivered `max(C, L)` or more, the cap may have bound inside the second, no finer cursor exists on the standard filter surface, and the load is **potentially incomplete** and MUST be reported as such. This verdict is terminal for the load: the client MUST NOT continue to step 6, and no later observation upgrades it. A continuation past an undischarged second can deliver nothing simply because that second was the oldest, so an empty continuation is not evidence that the second above it was exhausted.
6. Otherwise continue with `"until": T - 1`. Because a bare `until` is inclusive, decrementing guarantees each continuation covers a strictly older band, so the loop makes progress regardless of how the relay caps.
7. The load is **complete** when a continuation delivers no events at all, every preceding second having been discharged by step 5, the fence having been established before the first query and held unbroken since, and every fence delivery received up to that continuation's end-of-stored-events having been collected. Because the filter constrains nothing the relay applies after its cap, an empty delivery is an empty result — a cap that returns nothing is not a cap.
8. A load that failed, or whose fence lapsed, on any relay the client publishes to is potentially incomplete (see Read-Before-Write).

This layer places five requirements on every relay a client performs a full-state load against. They are stated normatively, not as background assumptions, because a *complete* verdict rests on them and none of them is verifiable from the responses a client receives:

- **Newest-first prefix delivery.** A capped result MUST consist of the newest events by `created_at` for the filter, ties broken by lowest id — the delivery NIP-01 already specifies for `limit`. A relay that caps by returning some other subset can omit an event lying *above* the cursor the client derives from that same delivery, so the omitted event is never queried at all. Repeating a query cannot recover it, because the same filter with the same bounds is the same request.
- **Non-decreasing effective cap within a load.** A relay MUST NOT reduce, within a single load, the number of events it will deliver for queries that differ solely in their time bounds. A cap that shrinks between the query establishing `C` and a later pinned window makes that window's short delivery indistinguishable from exhaustion, which converts a truncated second into a discharged one.
- **The floor `L`.** A relay MUST deliver at least `L` events for a query whose matching set holds at least `L` and whose requested `limit` is at least `L`. `L = 2`, fixed by this NIP; a client MUST NOT derive it from relay-advertised discovery, because an advertised maximum is not necessarily the limit a relay enforces.
- **Push delivery on an open subscription.** A relay MUST deliver every event it accepts that matches an open subscription's filter to that subscription. The mutation fence in step 2 is exactly this delivery; a relay that accepts a replacement without pushing it gives the client no way to observe a coordinate that moved above the cursor.
- **The delivery barrier.** Before a relay sends end-of-stored-events for a query, every event it accepted before that query read its stored events, and which matches an open subscription on the same connection, MUST already have been delivered to that subscription. Push delivery alone promises only that the replacement arrives eventually; the barrier is what places it before the verdict that depends on it. Without it, a relay whose accept path and query path proceed independently can answer a query from storage the replacement has already changed while the corresponding push is still pending, and the client discharges the load in the interval between the two.

A client cannot distinguish a relay that violates any of these from one that simply had fewer events to return, so these are conformance preconditions of this layer rather than properties a load establishes. A client MUST NOT perform a full-state load against a relay it knows, or has evidence, to violate them, and MUST treat any load against such a relay as potentially incomplete. Conditioning *complete* on positive proof of these properties instead would be equivalent to never issuing it — no such proof exists on the standard filter surface — which would withdraw the override layer from every client rather than from the non-conforming relays.

The comparison in step 5 fails safe: a pinned window is reported potentially incomplete unless the relay has already shown it will deliver at least that many at once, so an inconclusive result is never mistaken for an exhaustive one. A plateau of more events at a single `created_at` than the relay will deliver for a window pinned to that second is therefore unenumerable, because this NIP defines no finer cursor, and it resolves to *cannot prove complete* rather than to a false *complete*. The comparison is a lower bound on the cap rather than the cap itself, so it is also conservative in the other direction: a load whose oldest second holds as many events as the largest delivery observed so far resolves to *cannot prove complete* even where the relay would have delivered more. Where more than one coordinate exists this is transient, because any later publish moves that coordinate to a different second and separates the two.

The floor is the narrowest of the five requirements and the one that makes the ordinary case reachable at all. A coordinate at a replaceable kind contributes exactly one event no matter how many times it is republished, because a republish replaces the previous version rather than appending to it; a client's event count at this kind therefore does not grow over time, and a single-installation client publishing under one coordinate has one event at one second permanently. Without a floor, `C` for such a client is one, its pinned window delivers one, and step 5 can never be discharged — mark-unread would be permanently unavailable to the most common conforming deployment, and no amount of waiting or republishing would change the observation. `L = 2` discharges it: the pinned window delivers one event, `max(C, L)` is two, `1 < 2`, the second is exhausted, and the continuation below it is empty. That same replacement behaviour is what the fence exists for: the one event a coordinate contributes can move, and it moves by being replaced.

A **potentially incomplete** load MUST NOT be the basis for any of the following, each of which either destroys override state or asserts authority over it:

- canonical compaction of an override register (see Mandatory Canonical Publication),
- publishing a canonicalized override blob,
- deleting or abandoning any coordinate (see Orphaned Blob Deletion),
- reporting an explicit mark-read as successful (see Actions).

Until a complete load succeeds, the client MUST evaluate unread state from its own locally persisted state and MUST report override actions as failed rather than acting on a partial view. The honest terminal states are *complete* and *cannot prove complete*; a client MUST NOT treat the second as the first.

The number of coordinates a full-state load must retrieve is bounded by the number of installations that have ever used the override layer, plus their not-yet-deleted rotation predecessors. It grows with the user's device history, not with elapsed time, and — because a coordinate carrying `ov_*` entries may not be deleted until it has been carried forward (see Client-ID Rotation) — it does not shrink on its own. Clients SHOULD carry forward and delete rotation predecessors promptly so the count stays near one coordinate per live installation.

### Merge Rule

After decrypting all fetched blobs, the effective read timestamp for each context is:

```
effective[context] = max(timestamp) across all blobs
```

This is a grow-only max-register state-based CvRDT with an associative, commutative, idempotent join. Clients MUST NOT lower a read timestamp — only advance it.

The manual-unread override layer (see Manual-Unread Override Layer below) adds per-context set/clear counters merged by the same componentwise `max()` rule. The frontier merge rule is unchanged.

### Writing

Clients MAY publish read state automatically when read-position sync is part of
the client's default account state model. This NIP is explicitly not a
read-receipt protocol; any protocol or feature that exposes what a user has read
to other users MUST require explicit user consent.

Clients SHOULD publish read state blobs to the same relays they use for general event storage. Clients that implement NIP-65 (relay list metadata) SHOULD publish to their write relays and fetch from their read relays.

Each client instance maintains its own primary blob (one `kind:30078` event at its primary coordinate), plus one event per additional frontier-only coordinate if it uses any. Writing replaces the previous blob at each coordinate via parameterized replaceable event semantics ([NIP-33](33.md)).

Clients MUST only update blobs whose decrypted `client_id` matches their own `client_id`. Clients MUST NOT overwrite another instance's blob.

If the client discovers same-`client_id` blobs at coordinates that are neither its primary nor one of its known additional coordinates (e.g., rotation orphans or backup/restore duplicates), it MUST merge them into its own state and MUST NOT delete them until their override state has been carried forward (see Orphaned Blob Deletion). It MUST NOT publish to them: its own writes go to its primary and its known additional coordinates only.

#### Read-Before-Write

Before publishing, a client MUST:

1. Fetch its own current blob(s) from each relay it intends to publish to, and merge all fetched versions.

A client fetches its own coordinates using their known `d` tag values — its primary, plus its additional frontier-only coordinates if any — and unions them componentwise:

```json
{"kinds": [30078], "authors": ["<user-pubkey>"], "#d": ["read-state:<own-primary-slot-id>", "read-state:<own-additional-slot-id>", ...]}
```

A read-before-write fetch of the client's own coordinates is not a full-state load: it cannot discover rotation orphans or duplicates. Before canonicalizing override state or publishing a canonicalized override blob, the client MUST have a complete full-state load (see Full-State Load).

2. Decrypt and merge the fetched blob(s) with local state using `max()` per context.
3. Publish the merged result.

If a relay is unreachable during the fetch step, the client SHOULD proceed with the data available from reachable relays. The merge rule ensures that data from the unreachable relay will be incorporated on the next successful fetch, provided the relay retains the event. Permanent relay loss or event expiry may result in loss of frontier state — this is an accepted property of the best-effort model (see Non-Goals).

That accepted loss does not extend to override state. A client MUST NOT treat a fetch that failed on any relay it publishes to as a complete view of its own override state, and MUST NOT canonicalize, publish canonicalized override state, or delete or abandon any of its own coordinates on the basis of such a partial fetch (see Full-State Load). Clients implementing the override layer SHOULD publish override state to more than one relay so that the loss of a single relay does not erase a tombstone floor.

This read-before-write requirement also applies to re-publishes triggered by incoming blobs from other instances (see Live Subscription and Convergence).

The `created_at` monotonicity rule applies relative to the maximum `created_at` seen across all fetched blobs. Combined with the max-merge rule, this reduces the risk of state loss when two instances write concurrently. Full consistency is achieved once all instances complete a subsequent fetch-merge-publish cycle.

#### Live Subscription and Convergence

Clients SHOULD subscribe to `kind:30078` events for their own pubkey with `#t: ["read-state"]` for live updates:

```json
{"kinds": [30078], "authors": ["<user-pubkey>"], "#t": ["read-state"]}
```

When a blob from another client instance arrives (i.e., its decrypted `client_id` does not match the client's own `client_id`):

1. Merge it into local state using `max()` per context.
2. Canonicalize the merged override state against the client's own effective frontier (applying the tombstone floor and live/dead/virgin rules from Mandatory Canonical Publication). If any context entry in the canonical merged result differs from the corresponding entry in the client's last-published canonical blob (or the context is absent from the last-published blob), perform a read-before-write and re-publish the client's own blob after a debounce delay.
3. Clients MUST suppress the re-publish if the canonical merged result is identical to the canonical form of the client's last-published blob. Comparing canonical-to-canonical prevents a retained live peer blob (which the client has already tombstoned) from triggering an identical write on every replay. A client that has never published treats its last-published blob as empty.
4. Clients SHOULD limit re-publishes triggered by incoming blobs to at most one per debounce window, regardless of how many blobs arrive during that window.

This drives convergence without a coordination round-trip, assuming eventual relay reachability and event retention.

A live subscription is not a full-state load. A relay MAY return a capped set of stored events before end-of-stored-events on this filter, so a client implementing the override layer MUST NOT treat what the subscription delivers as a complete view of its coordinates (see Full-State Load). Merging an incoming blob into local state (step 1) is always safe, because merge is componentwise `max()`; the canonicalize-and-re-publish in steps 2–3 is a canonical publication and therefore requires a complete full-state load. A client that does not have one MUST defer the re-publish rather than publish a canonical blob derived from a partial view. A subscription is nonetheless a required *component* of a full-state load, serving as its mutation fence, and the fence MUST use the tag-free filter rather than the `#t`-narrowed one above — a replacement it fails to deliver is a replacement the descending enumeration cannot recover.

#### Clock Skew

When publishing, if the client's local clock produces a `created_at` value less than or equal to the maximum `created_at` seen across all fetched blobs for the same `d` tag, the client MUST use `max_fetched_created_at + 1` instead.

#### Debounce and Pruning

Clients SHOULD debounce writes to avoid excessive relay traffic (e.g., flush 5–10 seconds after the last local read-state change, or on app close/background transition). Clients MUST NOT write on every individual read action.

The blob SHOULD contain only contexts the client has explicitly interacted with. Clients SHOULD prune aggressively, prioritizing recently-active contexts, and MAY drop frontier entries older than the time horizon before writing. Clients MUST NOT drop `ov_*` override entries (including tombstone floors) based on age or budget pressure; see Override State Durability in the Manual-Unread Override Layer section. Clients MUST ensure the published event does not exceed relay event size limits (typically 64 KB content).

#### Client-ID Rotation

Clients MAY rotate their `client_id` by generating a new one, generating a new random `<slot-id>` for the primary coordinate, and publishing a new blob. Rotation adds one extra blob temporarily. Clients SHOULD keep their `client_id` stable for as long as possible to minimize blob proliferation.

Rotation is the only event that changes a client's override-bearing coordinate, and it carries the layer's single durability obligation:

**Carry-forward rule.** Before deleting or abandoning its previous primary, a rotating client MUST publish the componentwise `max()` of every override register the old primary holds — every tombstone ceiling included — under its new primary, and MUST confirm acceptance **on every relay from which the old primary will be deleted or allowed to lapse**. If any such relay rejects the publish or is unreachable, the client MUST retain the old primary on that relay and MUST NOT delete it there. Acceptance on one relay does not authorize deletion on another: a relay that never received the replacement would otherwise be left with no local carrier of the floor. The old primary MUST NOT be left to age out while it is the only carrier of an override floor on any relay.

Additional frontier-only coordinates carry no override state, so rotation may abandon or delete them freely.

If a device backup or clone results in two installations sharing the same `client_id` and primary `<slot-id>`, both will write to the same coordinate. This is operationally equivalent to a single client and does not corrupt state, but the two installations will overwrite each other's context entries. Clients that detect this condition (e.g., by observing unexpected context changes in their own blob) SHOULD generate a new `client_id` and a fresh primary `<slot-id>`, again carrying override state forward per the carry-forward rule.

#### Orphaned Blob Deletion

Clients MAY delete blobs from decommissioned client instances by publishing a `kind:5` deletion event per [NIP-09](09.md) targeting the orphaned event's `a` tag coordinate (`30078:<pubkey>:<d-tag-value>`). For blobs carrying no `ov_*` entries — including a client's own additional frontier-only coordinates — this is optional and unconditional: such blobs are harmless and age out naturally.

A blob carrying `ov_*` entries MUST NOT be deleted or abandoned until its override state has been carried forward per the carry-forward rule in Client-ID Rotation. This applies to the client's own previous primary and to same-`client_id` orphans discovered from a prior rotation or a backup/restore. A client's record of its own coordinates MAY be stale — for example restored from a backup taken before a rotation — so an unknown same-`client_id` coordinate MUST be treated as a live carrier of override state, not as a deletable duplicate, until it has been merged and carried forward.

### Manual-Unread Override Layer

This section defines a manual mark-as-unread mechanism as a CRDT override layer within the existing `contexts` map. It does not change the frontier merge rule, event structure, or encryption scheme. Fetching follows the override-specific full-state procedure (see Full-State Load) rather than the horizon-bounded fetch used by clients that do not implement this section. Clients that do not implement this section remain fully interoperable (see Backwards Compatibility).

#### Wire Encoding

For each manually-unread context `<ctx>`, a client publishes up to three sibling keys alongside the existing frontier entry in the `contexts` map:

| Key | Type | Description |
|-----|------|-------------|
| `ov_s:<ctx>` | uint32 | Set counter S — incremented on each mark-unread |
| `ov_c:<ctx>` | uint32 | Clear counter C — incremented on each mark-read |
| `ov_b:<ctx>` | uint32 | Baseline B — the effective frontier value at the time of the most recent mark-unread |

Values MUST be integers in the range 0–4294967295 (same validation range as context timestamps). The `<ctx>` suffix is the raw context ID without any escaping (escaping applies only to the frontier wire key; see Reserved Namespace).

#### Merge Rule (Override Registers)

Override counters are merged by componentwise `max()`, identical to the frontier merge rule:

```
merged_S[ctx] = max(S) across all blobs
merged_C[ctx] = max(C) across all blobs
merged_B[ctx] = max(B) across all blobs
```

No new wire-level merge logic is required. The same `mergeReadStateEvents` path that joins frontier timestamps joins the counter entries as integer max.

#### Liveness Predicate

A context `ctx` has an active manual-unread override if and only if ALL of the following hold, evaluated against the merged register `(S, C, B)` and the merged effective frontier `F`:

1. `S > 0` — at least one mark-unread action has been recorded.
2. `F <= B` — the effective frontier has not advanced past the baseline captured at mark-unread time. (A natural frontier advance strictly past `B` dominates a stale set, clearing the override without any explicit clear action.)
3. `S > C` — set counter exceeds clear counter. (`S == C` is treated as inactive: clear wins on ties — see Tie Policy.)

Formally (clear-wins is the only conforming tie policy — see Tie Policy):

```
override_active(S, C, B, F) =
  S > 0
  AND F <= B
  AND S > C
```

The **unread verdict** for a context is:

```
unread(ctx) = (latest_message_ts > F) OR override_active(S, C, B, F)
```

where `latest_message_ts` is the `created_at` of the newest message in the context.

#### Actions

Every action below requires a complete full-state load (see Full-State Load); on a potentially incomplete load the client MUST report the action as failed rather than act on a partial view of its own override state.

**Mark-unread:** increment S to `max(S, C) + 1`; set B to the current effective frontier value for the context. C is unchanged. If `max(S, C) == 4294967295` (uint32 maximum), the client MUST refuse the mark-unread action and leave the register unchanged; wrapping or resetting to zero is prohibited.

**Mark-read (explicit):** advance the frontier to cover the context as normal; increment C to `max(S, C) + 1`. S and B are unchanged. If `max(S, C) == 4294967295`, no representable counter increment exists; wrapping or resetting to zero is prohibited. The client MUST then complete the action only if the resulting state satisfies `override_active == false` — i.e. the frontier advance alone deactivates the override, or the override was already inactive. Otherwise the counters MUST be left unchanged and the client MUST report the mark-read as failed; the monotone frontier advance itself is still permitted, but a client MUST NOT report an explicit mark-read as successful while `override_active` remains true.

**Natural read (frontier advance):** advance the frontier past B. No counter update is needed — the liveness predicate's `F <= B` condition automatically deactivates the override when the frontier dominates the baseline.

#### Tombstone Floor

A register where `S > 0` or `C > 0` (ever-active) that evaluates as inactive MUST be compacted to the tombstone floor before publication:

```
tombstone = RegB(S=0, C=max(S, C), B=0)
```

This preserves the counter ceiling as a reuse-blocking floor. A register where `S == 0` and `C == 0` (virgin, never activated) MUST be omitted from the wire entirely (0 keys).

#### Mandatory Canonical Publication

Publishers MUST canonicalize every override against their own effective frontier at serialization time before writing to the wire:

- **Live override** (`override_active` is true): publish all three keys (`ov_s:`, `ov_c:`, `ov_b:`) with their current values unchanged.
- **Dead override** (`override_active` is false, `S > 0` or `C > 0`): publish only the tombstone floor — a single `ov_c:` key with value `max(S, C)`.
- **Virgin register** (`S == 0` and `C == 0`): omit all three keys from the wire.

This is a protocol requirement, not an optimization. A client that publishes raw (non-canonical) dead registers can cause two independently-dead registers from different devices to produce a live join on merge. See `docs/formal/nip-rs-unread/` for the exhaustive proof and mutation harness.

#### Override Group Co-Location Rule

A context's frontier entry and ALL of its `ov_*` sibling entries MUST travel in the same event, and that event MUST be the primary coordinate. Because all `ov_*` entries live in the primary (see `d` Tag), an override-bearing context has exactly one legal destination for its whole group: a client that splits frontier entries into additional coordinates MUST NOT move the frontier entry of an override-bearing context out of the primary, and MUST NOT place `ov_*` entries anywhere else. Only frontier-only groups — contexts with no `ov_*` entries — may be distributed across additional coordinates.

Implementations that split blobs across coordinates MUST group context entries per logical context — not per individual key — and assign the entire group atomically to one coordinate. Round-robin or other assignment strategies MUST operate on groups, not on individual entries.

**Unescape-before-group rule (corollary):** when grouping, a frontier wire key MUST be unescaped to its raw logical context ID (stripping one leading `esc:` if present) before being used as the group identity. Without this step, a frontier key `esc:ov_s:evil` and its `ov_*` siblings (keyed by the raw suffix `ov_s:evil`) resolve to different groups and the register splits across coordinates, reproducing the partial-reconstruction poison across publication cycles.

**Rationale:** a receiver holding only a partial group (e.g., `ov_s:ctx` without `ov_b:ctx`) reconstructs a register with incorrect baseline and may canonically publish a false tombstone. With atomic grouping, a compliant publisher's output never permits partial reconstruction.

#### Tie Policy

**Clients MUST use clear-wins.** When `S == C` and `S > 0`, the override MUST be treated as inactive, and the register MUST be compacted to the tombstone floor on publication (see Tombstone Floor).

Clear-wins is normative rather than a local implementation choice because the tie verdict is not encoded on the wire. Two conforming clients holding the same merged register `(S, C, B, F) = (1, 1, 10, 10)` would otherwise disagree permanently: a clear-wins client reports read and publishes the single-key tombstone floor, a set-wins client reports unread and publishes all three keys. Further deliveries converge the counters but can never converge either the verdict or the canonical wire form, which defeats cross-device synchronization. Supporting a selectable tie policy would require encoding the policy in the blob plus a separate interoperability design; neither is in scope here.

Clear-wins also matches the product semantics this layer is designed for: a false negative (a missed badge) is recoverable by re-marking unread, while a false positive (a badge that will not clear) is more disruptive. See `docs/formal/nip-rs-unread/NOTE.md` for the policy comparison — both policies satisfy the merge-correctness invariants in isolation, so this is an interoperability requirement, not a merge-safety one.

#### Override State Durability

`ov_*` override entries — especially tombstone floors (`ov_c:` keys) — carry a reuse-blocking counter ceiling that prevents stale override components from resurrecting a dead register. Specifically, if a tombstone floor `(S=0, C=k, B=0)` is dropped and a stale snapshot `(S=k, C=0, B=b)` is later replayed, the merged result `(S=k, C=0, B=b)` would evaluate as live — a resurrection.

Because legacy clients can carry and republish old `ov_*` keys indefinitely (they pass through `sanitizeContexts` as unknown opaque entries), there is no finite time after which all stale override components are guaranteed absent. Therefore:

**Clients MUST NOT drop `ov_*` override entries (including tombstone floors) based on age pruning or budget eviction.** This exemption applies permanently. Age-based pruning applies to frontier entries only. Eviction strategies that respect byte/key budgets MUST apply to frontier and `msg:`/`thread:` entries first and MUST NOT touch `ov_*` entries.

**Durability is a property of retrievable logical state, not of keys within one blob.** An override register survives only if a client that loads its full state can still reach every component. Therefore, in addition to the per-entry rule above:

- Full-state loads by clients implementing this layer MUST NOT be restricted by a finite event-level `since` window, and MUST establish completeness rather than assume it; the containing event must remain reachable, not merely retain its keys (see Full-State Load).
- No coordinate carrying `ov_*` entries may be deleted or abandoned until the componentwise `max()` of every override register it holds — especially every tombstone ceiling — has been republished under the client's current primary coordinate and accepted on every relay from which the old coordinate will be deleted or allowed to lapse (see Client-ID Rotation, Orphaned Blob Deletion).

**There is no safe finite GC horizon for override state.** Any protocol that proposes to delete tombstone floors after a bounded period requires a separately proved guarantee that no stale override component can re-enter the merge — this amendment does not provide such a guarantee.

#### Bounds and Budget

- **Key growth:** a live override adds 3 entries per context; a tombstoned override adds 1 entry per context. At 100 overridden channel contexts: ~300 live entries or ~100 tombstone entries.
- **Byte cost (small-counter example, common case):** channel UUID context (36 chars), counters S=1/C=0/B=10 — live override ~138 bytes; tombstone ~45 bytes. **Byte cost at uint32 maximum** (S=4294967295, worst case): live override ~164 bytes; tombstone ~54 bytes.
- **Hard ceiling on ever-overridden contexts.** Because all `ov_*` entries live in one coordinate (see `d` Tag) and tombstones can never be pruned (see Override State Durability — there is no safe finite GC horizon), the primary blob's plaintext budget is a hard ceiling on the number of contexts a single installation can ever have manually marked unread. Against a 32 KiB plaintext budget: roughly **600** tombstoned contexts at the worst-case ~54 bytes, ~730 at the common ~45 bytes, or ~199 simultaneously live overrides at ~164 bytes — and that is before frontier entries get any room at all.
- **Terminal behaviour at the ceiling.** When the primary blob cannot accommodate a new override group after all prunable frontier entries have been evicted, the client MUST refuse the mark-unread action and report it as failed. It MUST NOT split override state across coordinates and MUST NOT drop tombstone floors to make room. Likewise, a client whose merged override state — including tombstones merged in from peer installations — no longer fits in its primary blob MUST leave its last-published primary in place, MUST NOT publish a primary that omits merged `ov_*` entries, and MUST report override actions as failed; publishing a truncated override set is budget-driven override loss under another name. No floor is lost in this state, because the installations that originated those floors still carry them; the constrained installation simply stops acting as a replica until it has room. This is the same policy shape as counter exhaustion (see Actions): visible failure, never silent degradation.
- **10,000-key limit:** override entries count toward the existing per-blob validation limit. Tombstones accumulate permanently with every distinct ever-overridden context; they cannot be pruned. Clients SHOULD compact dead overrides aggressively and MAY enforce an active-live-override cap. Note that a cap on live overrides does not bound the total `ov_*` entry count over an unbounded context lifetime — tombstones from all historical overrides remain. The 32 KiB and 10,000-entry limits are expressed per blob at write time; a client that has overridden many distinct contexts over its lifetime must account for all accumulated tombstones when evaluating budget headroom.
- **256-byte key limit:** override keys (`ov_s:`, `ov_c:`, `ov_b:` + context ID) count toward the per-entry 256-byte validation limit. Context IDs up to 251 bytes are safe. Buzz's own context ID shapes (UUID 36 bytes, `msg:hex64` 68 bytes, `thread:hex64` 71 bytes) are well within this limit.

#### Verification Artifact

The design was verified by bounded exhaustive model checking prior to this amendment. See `docs/formal/nip-rs-unread/` for the full model (`model.py`, `exhaustive.py`), 9-mutant harness (`mutation.py`), and design notes (`NOTE.md`).

The harness verifies the three load-bearing safety requirements — tombstone floor, mandatory canonical publication, and atomic per-context grouping with unescape-before-group — are necessary: each mutant that drops one of these rules produces a detectable witness of permanent false-clear or resurrection. M3 validates that the clear-wins tie policy produces the intended product-semantics behavior; M5 and M6 witness value-range and convergence failures respectively. Clear-wins is normative for interoperability (see Tie Policy), not because set-wins violates merge safety — the model confirms both tie policies satisfy the merge-correctness invariants when applied uniformly.

**Scope of formal verification:** the bounded model covers the CRDT register algebra, merge/compaction rules, per-context grouping atomicity, and escape/unescape bijection. The model is a broader predecessor of this NIP: its `split_blob_into_slots` permits override groups in any slot, whereas this NIP confines them to one primary coordinate, so the verified atomicity property holds for every arrangement this NIP permits but the converse does not follow. The model does **not** verify the single-primary rule, the full-state-load completeness procedure, the relay conformance requirements or the mutation fence it depends on, or the carry-forward rule; those are normative here and argued, not proved. The model also does NOT cover malformed-group wire validation (the accepted-shape rules in Content Validation). That rule is sound by the partial-group argument (rejecting a partial group leaves a virgin register — a merge no-op — which is strictly safer than zero-filling missing components), but its correctness under parser-level implementation is outside the model's verified scope. Implementation-level tests MUST cover the accepted wire shapes and rejection behavior.

## Example

A user runs two clients: a desktop app and a mobile app. Each has a random `<slot-id>` with no relationship to its `client_id`.

Desktop blob (`d` tag: `read-state:a3f8c2e1d4b7906f5e2a1c8d3b6e9f04`), decrypted content:
```json
{
  "v": 1,
  "client_id": "desktop-v2-prod",
  "contexts": {
    "ctx:AAA": 1700000100,
    "ctx:BBB": 1700000050
  }
}
```

Mobile blob (`d` tag: `read-state:7b1d5a3e9c2f804d6e1b3a7c5d8f2e06`), decrypted content:
```json
{
  "v": 1,
  "client_id": "mobile-ios-v1",
  "contexts": {
    "ctx:AAA": 1700000200,
    "ctx:CCC": 1700000080
  }
}
```

The `d` tag slot IDs are random and reveal nothing about the client identity. The `client_id` values inside the encrypted content identify which device owns each blob.

Merged effective state:
```json
{
  "ctx:AAA": 1700000200,
  "ctx:BBB": 1700000050,
  "ctx:CCC": 1700000080
}
```

## Test Vectors

The following vectors show plaintext content only. Actual events would carry NIP-44 ciphertext in the `content` field. The slot IDs in the `d` tags are random and have no relationship to the `client_id` values.

### Device A — plaintext content

```json
{
  "v": 1,
  "client_id": "client-aabbccdd",
  "contexts": {
    "group:general": 1700001000,
    "group:dev":     1700000500
  }
}
```

Event tags:
```json
[
  ["d", "read-state:1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d"],
  ["t", "read-state"]
]
```

### Device B — plaintext content

```json
{
  "v": 1,
  "client_id": "client-11223344",
  "contexts": {
    "group:general": 1700001200,
    "group:random":  1700000800
  }
}
```

Event tags:
```json
[
  ["d", "read-state:f0e1d2c3b4a5968778695a4b3c2d1e0f"],
  ["t", "read-state"]
]
```

### Merged effective state

```json
{
  "group:general": 1700001200,
  "group:dev":     1700000500,
  "group:random":  1700000800
}
```

Device A's own blob is identified because its decrypted `client_id` (`client-aabbccdd`) matches Device A's locally stored `client_id`. Device B's blob is merged but not overwritten by Device A.

### Ciphertext Test Vector

The following vector demonstrates the full encrypt-to-self pipeline using NIP-44 v2. The private key is the well-known secp256k1 scalar `1`.

```text
private_key = 0000000000000000000000000000000000000000000000000000000000000001
public_key  = 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
```

Plaintext:
```json
{"v":1,"client_id":"test-vector-client","contexts":{"group:general":1700001000,"group:dev":1700000500}}
```

Ciphertext (NIP-44 v2, base64):
```text
Akt10yui5aDIjfH+xED2Dr1NJ/SGWp85SC/r/bloiLRtj8K59rJrYhcfsNQMoMhpLlvhKqrN0HIGb9/V9BcYKxWV8HT/jjDdvfHLUVfo688I6WpapcX41GzL4VnGGDdFyUom53odJncjHszS3dpTrG1OKp2x9dtdG+924/+Ne49KN4nztd1pikqYeqQuxflKCmh+VcCFbDclQ8a9NUpqWkPpeoweISVVuZDnP9WFoKG5X6YcpXBWH6wjc69xK4cs6KkJ
```

The conversation key is `nip44_conversation_key(private_key, public_key)` — ECDH of the key with itself. NIP-44 v2 uses a random nonce, so re-encryption will produce different ciphertext. Verification is decrypt-only: any conforming NIP-44 implementation MUST satisfy `decrypt(private_key, public_key, ciphertext) == plaintext`.

### Conflict Detection Vector

Device A has `slot-id` = `aaa111aaa111aaa111aaa111aaa111aa` and `client_id` = `client-A`. It fetches its own `d` tag coordinate `read-state:aaa111aaa111aaa111aaa111aaa111aa` and decrypts the blob. The decrypted `client_id` is `client-B` (not `client-A`). This is a slot-id conflict — another device has claimed this coordinate.

Device A MUST NOT publish to `read-state:aaa111aaa111aaa111aaa111aaa111aa`. Device A MUST generate a new random `slot-id` (e.g., `ccc333ccc333ccc333ccc333ccc333cc`) and publish its blob under `read-state:ccc333ccc333ccc333ccc333ccc333cc`.

### Clock Skew Vector

Device A fetches its own blob from two relays:
- Relay 1 returns the blob with `created_at` = 1700001000
- Relay 2 returns the blob with `created_at` = 1700001500

Device A's local clock reads 1700001200 (behind Relay 2). The maximum fetched `created_at` is 1700001500.

Device A MUST publish with `created_at` = 1700001501 (max_fetched + 1), not 1700001200.

## Invalid Cases

Clients MUST reject or discard each of the following:

- A blob whose `content` does not decrypt to valid JSON — discard the entire event.
- A blob with a missing `client_id` field — discard the entire event.
- A blob with `v: 2` (unknown version) — ignore the entire event.
- A blob with a non-integer timestamp for a context entry (e.g., `"ctx:AAA": "yesterday"`) — discard that context entry; process remaining entries.
- A blob with a context ID exceeding 256 bytes — discard that context entry; process remaining entries.
- A blob with more than 10,000 context entries — client MUST reject the entire blob.
- An event with no `d` tag — ignore the entire event.
- An event with a `d` tag value that does not begin with `read-state:` — ignore the entire event.

## Privacy Considerations

The `content` field is NIP-44 encrypted to the user's own keypair. Context identifiers, timestamps, and the `client_id` are not visible to relay operators or other users. As with all NIP-44 encrypt-to-self data, compromise of the user's private key exposes all stored read state.

The `d` tag prefix `read-state:` and the number of distinct slot IDs are visible to relay operators, revealing that the user employs read-state sync and approximately how many client instances they run. Write frequency may reveal approximate activity level.

Ciphertext length reveals the approximate number of tracked contexts and may correlate with the user's activity level across sessions.

Because slot IDs are random and independent of `client_id` values, relay operators cannot directly link blobs to specific devices or client implementations. Timing correlation and write patterns may still allow probabilistic linkage.

Because the frontier merge rule is monotonic, replaying an old frontier event to a relay is harmless — it cannot lower a read timestamp. The override layer's counter merge is also monotonic (componentwise max), so replaying an old override event cannot lower a counter; however, a stale override component replayed after a tombstone floor was published could suppress a fresh set for one reconciliation cycle (see Override State Durability). The debounce window (see Debounce and Pruning) limits convergence re-publishes to at most one per window.

Clients supporting multiple Nostr identities SHOULD use distinct `client_id` values and distinct slot IDs per identity. Reusing identifiers across pubkeys allows relay operators to link those identities.

Clients SHOULD describe relay-managed read state wherever they describe
relay-synced account data. This NIP does not authorize read receipts; clients
that expose read activity to other users MUST require explicit user consent.

## Kind Usage

| Kind | Usage |
|------|-------|
| `30078` | Per-client read state blob (parameterized replaceable, [NIP-78](78.md)) |

## Backwards Compatibility

This NIP introduces no changes to existing event kinds and adds no new kind, wire message, or relay-stored read-state logic. It uses only standard NIP-01 event storage, NIP-33 addressable event semantics, NIP-44 encryption, and NIP-78 application data conventions. Clients that do not implement this NIP are unaffected, as are clients that implement everything but the manual-unread override layer.

The override layer is the exception, and it is a relay-compatibility one rather than a client one. Its full-state load carries the completeness guarantee only against a relay that satisfies the ordering, capacity, floor, push, and barrier requirements enumerated in Full-State Load. Against a relay known or evidenced not to conform, every load resolves to *cannot prove complete* and the actions that depend on a complete load report as failed; against an undetectably nonconforming relay, a load may still return *complete*, and the completeness guarantee does not apply to that verdict. In either case the layer still runs and still merges, and frontier sync is unaffected.

## References

- [NIP-01](01.md) — Basic Protocol Flow Description (defines filter `limit`, `since`, and `until`)
- [NIP-09](09.md) — Event Deletion Request
- [NIP-33](33.md) — Parameterized Replaceable Events
- [NIP-44](44.md) — Versioned Encryption
- [NIP-78](78.md) — Arbitrary Custom App Data (defines `kind:30078` for application-specific data)
