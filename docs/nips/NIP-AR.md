NIP-AR
======

Channel and Community Artifacts
-------------------------------

`draft` `optional` `relay`

Defines `kind:45010` for editable records called **artifacts**. A channel artifact has one home channel and may be attached to a thread. Its home determines who can read it. The optional [community scope](#community-scoped-artifacts-proposed-extension) has its own audience and no channel home. Any number of artifacts, including of the same type, may share a channel or thread.

The relay manages identity, access, and revisions. Clients define the content types, such as `buzz.task` or `buzz.project`.

## Event format

Each event contains a complete snapshot. This section defines the channel envelope. This task is created in a channel, with the standard signing fields omitted.

```json
{
  "kind": 45010,
  "tags": [
    ["ar", "1"],
    ["d", "04737c81-e5e8-4412-bb47-f446813cfeba"],
    ["h", "9b353519-f4fe-4757-aef4-bec6cc0ae54c"],
    ["type", "buzz.task"],
    ["title", "Fix the payment timeout"],
    ["op", "create"],
    ["assignee", "<assignee-pubkey>"],
    ["project", "17cfe3ca-b3d0-4a76-a2a4-4eaf0d3a4109"]
  ],
  "content": "{\"version\":1,\"status\":\"open\"}"
}
```

The `assignee` and `project` tags and the payload are illustrative, not required fields; this NIP does not define a task schema.

| Tag | Meaning |
| --- | --- |
| `ar` | Envelope version; `1`. |
| `d` | Stable artifact UUID. |
| `h` | Home channel UUID. |
| `type` | Namespaced content type. |
| `title` | Absent on `delete`; otherwise required, nonblank display title, at most 512 UTF-8 bytes. |
| `op` | `create`, `update`, `move`, `delete`, or `restore`. |
| `root` | Optional conversation anchor event ID within `h`. |
| `prev` | Previous accepted revision ID; required except on creation. |

Each listed tag has exactly two string elements and occurs once, except optional tags may be absent and `title` MUST be absent on `delete`. UUIDs are lowercase, hyphenated, and non-nil; event IDs are 64 lowercase hex characters. Type names are dot-separated lowercase ASCII components, each starting with a letter and otherwise using letters, digits, `_`, or `-`, up to 128 bytes total. `buzz.*` is reserved for published Buzz client contracts.

Relays MUST reject invalid envelopes, unsupported envelope versions, and events with more than 256 tags. Normal event-size and write quotas apply.

Clients place fields needed for filtering in client-defined tags. The relay matches tag names and values without interpreting their meaning; `content` remains opaque text. Client-defined tags cannot override the envelope or grant access.

## Identity and edits

A channel artifact is identified by `(community, d)` within the channel identity space, independently of its author or home. `d` and `type` are immutable. Kind 45010 is a regular stored event, not a NIP-01 addressable event. It falls in Buzz's forum and social kind range (45000–45999) but is not a forum post and follows none of the forum rules.

Creation requires an unused `d`, `op=create`, and no `prev`. Every later revision MUST name the current accepted event in `prev`. The relay checks authorization and advances the current revision atomically: two competing edits cannot both succeed. Timestamps do not choose the winner.

Resubmitting an already accepted event succeeds without applying it again, even if a later revision has since become current, so a client that lost an acknowledgement never sees a false conflict. The response reveals nothing the sender can no longer read. A different event whose `prev` is not the current revision is a conflict. On conflict, clients fetch the current revision and reconcile their changes rather than resubmitting with only `prev` replaced, which would overwrite the competing edit. An `update` keeps `h`; a `move` changes it.

A `create` whose `d` is already in use fails. When the sender cannot read the existing artifact, the error reveals only that the identity is taken, never its home, type, title, or current revision. Clients MAY derive `d` deterministically, for example from the message an artifact tracks, and accept that disclosure.

On creation or when changed from the previous accepted revision, `root` MUST identify an existing, undeleted conversation anchor in `h`. Later loss of that anchor does not prevent edits. Replies belong to that conversation, not to an artifact revision.

## Channel access

Channel read permission governs every artifact read, including lookups, history, search, previews, counts, and live updates. Threads inherit their channel's audience. Membership and visibility changes take effect on subsequent reads and deliveries.

Write permission means permission to post a kind-9 message in the channel, including authentication, token restrictions, moderation, and archive checks. Relays apply these checks the same way, and at the same point, as for a kind-9 message. This includes agents and, where channel policy permits, nonmembers of open channels. Artifact writes use `messages:write`, the scope that admits kind-9 posting.

| Operation | Required permission |
| --- | --- |
| Create, update, delete, or restore | Write in the home channel. |
| Move | Write in both source and destination. |

These rules apply to every type and do not depend on who created the artifact. DM participants are peers. The relay determines channel type from the stored channel, not from an artifact tag.

Relationship tags organize work without changing access. Linking a task to a project does not move it or share it. References to repositories or other services retain those services' access and action permissions. Errors MUST NOT disclose inaccessible artifact details.

## Moving and deleting

A move publishes the current snapshot into the destination under the same `d`. Its `root` must be absent or belong to the destination. Clients MUST show the destination audience and the information being shared before confirmation.

The relay MUST atomically advance the current revision and store both arrival in the destination and removal from the source. The source receives a separate relay-authenticated removal identifying the artifact, source scope, and the revision it replaced, without destination metadata or content. Both are stored events, so a client that misses live delivery recovers them by replaying the channel. Relays unable to provide this MUST reject moves.

Earlier revisions remain under their original channels' access rules; a move never lets the destination read revisions from the source. The destination can load current state without reading earlier revisions. Conversation messages stay where they are.

Deletion is a soft delete: a revision with `op=delete`, empty content, and no `title` tag. It preserves the current `type`, `h`, and `root`, and contains only envelope tags and NIP-OA `auth` tags the relay has verified. It removes the artifact from current-state queries and active views. Its revisions stay readable under their channels' access rules until redacted or expired by retention, so a lookup by `d` returns the deletion. After deletion, only `restore` is accepted: it names the deletion in `prev`, keeps its `h`, and carries a complete snapshot that becomes the current state again. A deleted `d` stays reserved and is never reused for another artifact. Completing or archiving work is a content change, not deletion or channel archival.

## Moderation and retention

Relays MUST reject kind-5 deletion requests targeting artifact revisions with an explicit reason. A NIP-29 kind-9005 removal redacts a revision under the channel's ordinary message-removal rules: the relay withholds that revision from every surface, including lookups, history, and search. The relay still records the revision as accepted, so redaction never changes which revision is current, and its ID remains valid as `prev`. Redacting an artifact's current revision retires the artifact: it is absent from current-state queries, and clients that do not already hold that revision's ID cannot edit it further. Revision history is subject to the community's retention policy; expiring earlier revisions MUST NOT remove the current revision or break `prev` checks.

## Current state and queries

An artifact's current state is its latest accepted revision, as determined by the `prev` rules above. Current-state queries MUST return only that revision and omit deleted artifacts. For example, a task whose current revision belongs to Project B no longer appears in Project A's list, even if an earlier revision belonged there. Revision history is queried separately. This NIP does not define a public acceptance order or history pagination. Relays record the order in which they accept revisions; clients observe the resulting current state through sync (see Sync), not through timestamps or `prev` chains.

Relays MUST support querying current artifacts across channels the reader can access using exact tag-name/value matches, including multi-character tag names. Conditions on different tag names are combined with AND; requested values for one name are combined with OR. A match compares the tag name and its first value in the same tag. Access checks and tag matching apply before pagination and any counts. Older revisions do not contribute matches.

Relays MUST advertise and enforce limits on tag-name/value sizes, total tag bytes, query predicates and values, and page size. Queries MUST run within an enforced execution budget. Unsupported or over-limit requests fail explicitly; predicates MUST NOT be silently ignored or incomplete results presented as complete.

Artifact revisions are state changes, not conversation messages. Receiving one MUST NOT increase chat or reply counts, add a chat unread, mark a conversation read, or repeat mention or push notifications for tags carried over from the previous revision.

## Sync

Sync lets a client hold an exact local copy of a set of artifacts and keep it current. The relay owns the order of changes: the relay already decides which revision is current (the `prev` check above), and sync only reports those decisions by always sending the current head. Clients do not order changes by `created_at`, which the signer chooses, or by walking `prev` chains, which needs every revision.

Sync is additive. A client that does not use it keeps working unchanged, and a relay advertises each part separately (see Relay information document). A reader receives through sync only artifacts that it can already read with a current-state query, under the same channel read permission.

Incremental reads (below) are a separate HTTP protocol with their own capability. They let a client update a saved store after a disconnect without reading the whole set again, but they do not prove the complete set that `EOSE` proves here.

```mermaid
sequenceDiagram
  participant C as Client
  participant R as Relay
  C->>R: REQ sub {"artifact":"current", "#h":[...], "#type":[...]}
  Note over R: Register live interest first,<br/>then read all matching heads<br/>in one snapshot
  R-->>C: EVENT sub head  (0..n)
  R-->>C: EOSE sub  (the set is complete at one cut)
  Note over C: Replace local set only here
  loop live
    R-->>C: EVENT sub head or kind 45011 removal
  end
  R-->>C: CLOSED sub "resync: ..."  (access changed, or a change could not be delivered)
```

### Sync subscription

**Request.** Sync is a NIP-01 `REQ` whose single filter has `"artifact": "current"`. It is the current-state filter of HTTP `/query`; the `REQ` transport adds the complete set, `EOSE` and live heads. The client does not page: the relay streams the whole set until `EOSE`. Other views (`history`, `head`) have no `REQ` form and stay HTTP-only:

```json
["REQ", "s1", {"artifact": "current", "#h": ["<channel>"], "#type": ["buzz.task"]}]
```

The filter MUST include `#h` with 1 or more channels. It accepts the same `#name` predicates and limits as a NIP-AR current-state query, with the same AND/OR rules, and also `kinds: [45010]`. It accepts no `limit` or `offset`. A relay rejects a filter without `#h`, or with an unknown key, with `CLOSED`.

**Scope.** The sync's scope is the set of artifacts that a NIP-AR current-state query with the same filter returns to this reader at that time. The rules below use this one definition for access, matching and system channels.

**Response.** The relay answers with ordinary NIP-01 messages. Only the meaning of `EOSE` is stronger: for a sync, it means "this is the complete set at one point in time", not only "end of stored events".

| Message | Meaning |
|---|---|
| `["EVENT", sub, <kind 45010 event>]` | This revision is the artifact's current head. If it is a deletion, or it no longer matches the filter, the client removes the artifact from its set. |
| `["EVENT", sub, <kind 45011 event>]` | Live only. A move removed the artifact from a channel the reader can read. The client removes it. |
| `["EOSE", sub]` | Sent once, after the initial set. The set has been delivered in full, as one consistent cut (rule 2). |
| `["CLOSED", sub, "resync: <reason>"]` | The relay can no longer keep this subscription exact. The client starts a new sync. |
| `["CLOSED", sub, "restricted: <reason>"]` | The reader cannot read a channel that `#h` names, or the channel does not exist. The message does not say which. |
| `["CLOSED", sub, "error: <reason>"]` | The sync failed. The client keeps its last complete set and treats it as possibly stale. |

Rules:

1. **Live before stored.** The relay MUST register live interest, and wait until it is active, before it opens the snapshot of rule 2. Live interest is active only when the pub/sub server has confirmed the subscription (for Redis, the reply to `SUBSCRIBE`) and the node's pub/sub connection is still up. While that connection is down or reconnecting, a new sync waits, or gets `CLOSED "error: ..."`. Then every change that commits after the snapshot causes a live entry, or a resync when the relay detects that the entry was lost (rule 5). This is the order that `REQ` already uses. The relay MUST NOT send a live entry before `EOSE`. During the read it holds the `d` of each changed artifact. After `EOSE` it reads and sends their heads as in rule 3. A held change can already be in the snapshot; then the relay sends the same head again, a newer one, or nothing (rule 4). The held list is bounded: if it grows past `max_events` distinct `d`, the relay sends `CLOSED "resync: ..."` and no `EOSE`.
2. **Initial set: one consistent cut, read in pages.** The relay reads the set inside one read-only transaction that has one database snapshot (Postgres `REPEATABLE READ`). It opens the transaction after it receives the `REQ` and after rule 1, on the writer database or on a replica proven at least as fresh. So every change that the relay accepted with `OK` before it received the `REQ` is in the set. Inside the transaction it reads pages ordered by `d`. Each page starts after the last `d` of the page before it (keyset pages, not offset pages). All pages see the same snapshot. So the set at `EOSE` is the exact set at one point in time: a combination of heads that existed together, never a mix of earlier and later states. This is the complete-set read. The relay sends one `EVENT` for each artifact in scope as it reads it, then `EOSE`. The client sees one stream; it never joins pages. A snapshot does not need the whole set in one result, so there is no byte limit on the whole set. The relay bounds how long it holds a snapshot open, with a budget that it defines. It MAY copy pages into a bounded buffer so that it can close the snapshot before a slow reader has received them. If the read fails, the snapshot is lost, or the budget runs out (for example, the reader is too slow), the relay sends `CLOSED "error: ..."` and no `EOSE` (rule 8). It MUST NOT continue the read on another snapshot. If the reader cannot read a channel that `#h` names, the relay sends `CLOSED "restricted: ..."` and no `EOSE`, whether or not the channel exists.
3. **Live changes.** After `EOSE`, when an artifact in scope changes, the relay reads its current head. If the reader can read the head's channel, it sends the head, even when the head is a deletion or no longer matches. If the artifact moved to a channel the reader cannot read, it sends the kind 45011 removal stored in the source. It never sends anything about an artifact the reader cannot read. A change is an accepted revision (any `op`), or a move's removal from the source. An artifact is in scope for a change when it was in scope before the change or is in scope after it. "Before" is the client's copy as this sync sent it: the initial set plus later heads that matched. "After" is a current head, not a deletion, that matches every predicate of the filter with the NIP-AR match rule (the tag name and its first value in the same tag). The relay sends nothing for an artifact that is in scope neither before nor after, so a head that never matched the filter never reaches the client.
4. **Heads only, never older.** The relay sends the current head, never an older revision. It MAY skip intermediate revisions and MAY repeat a head the reader already has. For any one `d`, it MUST NOT send a head that it read before the head it last sent for that `d`. To keep this, for each sync the relay reads the head of one `d` one read at a time, starts each read only after the result of the read before it (a head, a kind 45011 removal or nothing) is in the outbound queue, and reads from the writer database or a replica proven at least as fresh. Clients that need every revision use the NIP-AR history view.
5. **No silent loss.** Live fan-out can lose entries: a publish can fail, a node's pub/sub connection can drop for seconds, and a node's local fan-out can lag. Pub/sub does not replay any of these. A sync MUST NOT lose a change silently. So:
   - A node MUST send `CLOSED "resync: ..."` to every sync it serves that a loss on that node can affect: its pub/sub connection dropped or reconnected; its local fan-out lagged or dropped entries, including a message that it could not decode; it could not check the reader's access to a change for that sync (for example, a failed membership lookup); or the connection's own send queue was full and dropped an entry for that sync. One drop is enough: the slow-client grace limit does not apply to a sync. It MAY resync every sync it serves.
   - If the node cannot queue the `CLOSED "resync: ..."` itself (for example, because the same send queue is full), it MUST close the connection. A client starts a new sync on any disconnect (rule 7), so a resync is never lost.
   - A node MUST retry a failed publish until it succeeds or the node stops. Delivery order does not matter. For a sync, a live entry is only a signal that one `d` changed. The reader's node never sends the published event itself to a sync; it reads the current head (rule 3). So a late retry of an older change sends the current head or nothing, never an older one (rule 4).

   So a sync that is open and has had no resync has received every change, except when a node crashes after it commits a change and before it publishes it. A client that must bound that window can sync again on its own schedule. The relay does not read the set again on a timer.
6. **Retirement.** A moderation redaction that retires the current revision of an artifact in scope makes the relay send `CLOSED "resync: ..."`. The relay withholds the redacted head, so a new sync is the only way the client learns that the artifact left the set. A relay MUST NOT retire a head in any other way that open syncs cannot observe, such as an operator delete outside NIP-29; it refuses such deletes or ends the affected syncs with `resync`.
7. **Commit point.** A client MUST replace its local set only when it receives `EOSE`. If the connection drops before `EOSE`, it discards what it received. After `EOSE`, it applies each live entry as it arrives. On any disconnect it starts a new sync; there is no resume. Before a new sync after a resync, an error or a dropped connection, the client waits a random delay that grows after each failure in a row (backoff with jitter).
8. **No partial success.** If the relay cannot deliver the full set within its limits or budget, it MUST send `CLOSED` and MUST NOT send `EOSE`. A closed connection without `EOSE` is never proof of an empty set. The relay bounds its buffers for a sync in bytes as well as in count, and it ends the snapshot and its reads as soon as the client disconnects or sends `CLOSE`.
9. **Resync.** The relay MUST send `CLOSED "resync: ..."` when the reader's access to a channel that `#h` names changes: membership, channel visibility or channel deletion. This closes the whole sync, also when `#h` names other channels. An ordinary `REQ` can keep its other channels; a sync cannot, because its set is no longer exact. Archive does not change reads (see Home channel lifecycle), so it does not cause a resync.

**Short-lived clients.** A CLI that needs the set one time sends the same `REQ`, waits for `EOSE`, then sends `CLOSE`. It gets the cut of rule 2: a complete set that includes every change the relay accepted before it received the `REQ`. It does not see changes after the cut. A writer catches those with `prev` (NIP-AR compare-and-set). There is no separate one-shot request.

**Set digest.** `set_digest` is the lowercase hex SHA-256 of the UTF-8 bytes `nip-ar-set-v1`, a zero byte, and then `<d>:<head event ID>\n` for each artifact in the set, sorted by `d`. It is a content identifier. It is not a counter, not a `prev` precondition and not a live-delivery cursor. An unchanged set has the same digest. A client can compute it over the set that it holds, or over any part of that set, and get the same value as any other client with the same heads.

The relay does not send the digest. A client computes it over the set that it holds at `EOSE` or later.

**Offset pages are views.** The existing `/query` offset pages stay for display and browsing. This section states that they are not evidence of a complete set.

### Head lookup, including deletions

Retries and "add or restore" flows need the current revision of one artifact even when it is deleted. Today a current-state query omits deleted artifacts, and history returns every revision.

Relays that support sync also support the query view `"artifact": "head"`. It requires `#d` and returns the current revision of each named artifact the reader can read, **including** a deletion. It returns nothing for an unknown `d`, or for one the reader cannot read; it does not say which.

### Deterministic identity and the ensure flow

Clients MAY derive `d`. This section defines one standard derivation, so that independent clients compute the same `d` for the same logical record:

```
d = UUIDv8( first 16 bytes of SHA-256("nip-ar-artifact\0" + h + "\0" + type + "\0" + key) )
```

`h` is the home channel at creation, `type` is the artifact type, and `key` is a client-chosen name, unique within that type and channel (for example a config entry name). Version and variant bits are set as in RFC 9562.

A derived `d` is a name, not a proof of authorship. Anyone who can write in `h` can create it first. Readers decide which authors to trust, as with every artifact.

**Ensure (create or restore, safe to retry).** To make "this record exists with this content" true, a client:

1. Sends `create`. If accepted or a duplicate of an accepted event, it is done.
2. On `conflict`, reads the head (see Head lookup).
   - If the head is a deletion, sends `restore` with `prev` set to it.
   - If the head is live with the same content and tags, it is done.
   - Otherwise sends `update` with `prev` set to the head, or reports the conflict, as the client's type requires.
3. Repeats step 2 on a further conflict, up to a client-defined limit.

The relay changes nothing for this flow except the head view. A deleted `d` stays reserved (see Identity and edits).

### Home channel lifecycle

| Home channel event | Writes | Current-state reads and sync |
|---|---|---|
| Archived | Rejected with the kind-9 archive reason. | Unchanged. Readers keep reading. To keep editing, unarchive or `move` the artifacts out first. |
| Unarchived | Allowed again. | Unchanged. |
| Deleted | Rejected. | Its artifacts leave every current-state result. Each live sync that covered the channel gets `resync`. A later channel with a different ID does not inherit them. |

### Client guidance (non-normative)

- **Type is the record family.** `type` cannot change. Put variants that can change (for example watch vs timer) in `content` under one type. If a family must change type, create new artifacts and delete the old ones.
- **Version the content.** Put `"version": n` in `content`. Ship readers that accept `n+1` before writers that produce it. Editors already keep unknown fields or decline the edit (see Client behavior).
- **Titles are visible to every reader of the channel.** Put no secrets in `title`.
- **Conflicts are normal.** On `conflict`, reload with the head view, reapply the user's change to the new head, and show the user when that is not possible. Do not resubmit with only `prev` replaced.

### Relay information document

The NIP-11 relay information document has an `artifacts` object. A relay that supports sync adds:

```json
"sync": {"version": 1, "max_events": 4096},
"modes": ["current", "history", "head"]
```

`max_events` is the largest set that a sync serves. It bounds each initial read and the held list (rule 1). There is no byte limit on the whole set: the relay reads and sends it in pages from one snapshot, and the relay's existing event size limit bounds each event. The relay does not advertise its snapshot time budget (rule 2); a reader that is too slow gets `CLOSED`.

and sets `websocket_query_extensions` to `true`.

Relays advertise `sync` and `head` only when every backend that can serve the connection runs them.

## Client behavior

Clients validate content before interpreting it as a known type. Unknown types or malformed content render a safe fallback using the title, type, and home link. Editors MUST preserve unfamiliar fields and annotation tags or decline the edit. References may be unavailable or cyclic; clients must handle them safely without assuming relay validation.

Artifact text is untrusted. A type name, relationship tag, or payload cannot authorize a privileged action. Moving, deleting, or revoking access cannot erase copies already downloaded.

## Community-scoped artifacts (proposed extension)

### Envelope and identity

A relay advertising `artifacts.community_scope.version = 1` accepts a second form of kind 45010. A community artifact uses `["ar","1"]`, `["community","<community UUID>"]` and the usual `d`, `type`, `title`, `op` and `prev` tags. It MUST omit `h` and `root`. A channel artifact retains its existing envelope and MUST omit the new `community` tag. Reject a missing, mixed or repeated scope. The signed community UUID MUST equal the tenant resolved by the connection or HTTP request, including on retries. Clients obtain this stable UUID from the serving relay's `artifacts.community_scope.community_id` in NIP-11. Two hostnames that serve the same community advertise the same ID. The NIP-11 `artifacts` object advertises the scope:

```json
"community_scope": {"version": 1, "community_id": "<community UUID>", "max_policies": 256}
```

`max_policies` is the bound on stored type policies; the value shown is an example.

Both scopes reuse the existing head and revision ledger. The identity in a community is `(community, scope class, d)`. Every channel remains in the same channel identity space, so moving a channel artifact preserves its identity. An artifact cannot move between scope classes. Reject `op=move` for a community artifact. In both scopes, `type` is immutable, and a revision naming an identity in the wrong scope answers like an unavailable head.

For one record per canonical key in a community type, clients MAY derive:

```
d = UUIDv8(first 16 bytes of SHA-256(
    "nip-ar-community-artifact\0" + community_uuid + "\0" + type + "\0" + key))
```

The inputs are UTF-8. The UUID is lowercase and hyphenated. The type's client contract defines the canonical key, including case and Unicode normalization. Apply the version and variant bits as in the channel derivation. The distinct prefix separates the two derivations. Channel authors can choose arbitrary UUIDs, so the relay MUST also separate the two identity spaces in storage. The same UUID may exist in each scope and cannot reserve a record in the other. Within a scope, the first accepted create reserves the identity. The relay does not interpret names or content, and does not require the key or verify its derivation.

### Admission and authorization

| Operation | Required permission |
| --- | --- |
| Read, count, policy lookup, history, head, or sync | Current effective community admission and `messages:read`. |
| Create, update, delete, or restore | Current effective community admission, `messages:write` and the current write policy for the type. A community timeout blocks the write. |
| Set a type's write policy | A direct community owner or admin acting under `admin:users`. Apply the relay's administrative ban and timeout rules. |

Apply the same authenticated transport, event signer, expiry, size, quota and moderation rules on HTTP and WebSocket. A channel-restricted credential cannot use community scope, even if it covers every channel. Unauthenticated reads and generic channel permissions confer no community access. Treat lookup failure as denial or an error, with no artifact data. On an open relay, any authenticated principal admitted by current relay rules qualifies under `everyone`, even with no roster row. On a roster relay, the principal needs current effective membership, direct or through verified delegation admitted by the relay. Delegation does not transfer an owner's or admin's role. Apply the principal's and applicable owner's community restrictions. A ban blocks reads and writes, including replay. A timeout leaves reads available. Channel membership, archive, posting restrictions and channel admin roles do not grant or restrict these operations.

### Type write policy

The policy is tenant-scoped and keyed by the exact canonical artifact `type`. Missing means `everyone` with no prior policy revision. `admins` admits only an author who personally holds the community owner or admin role. The default on a rosterless open community is still `everyone`. Setting policy requires a configured community owner or admin.

Use the signed relay-admin command `SET_ARTIFACT_TYPE_POLICY` (proposed kind 9034, pending allocation). Require `community`, `type`, `policy` (`everyone` or `admins`), and `prev` (the current policy command ID or `0` for no prior command). Omit `h` and use empty content. Apply the relay-admin freshness and signer checks. Return a conflict for stale `prev`; an exact retry of the current accepted command is a no-op if the requester is still authorized. A superseded policy command may conflict on retry. Read current policy before retrying a conflicted command. The relay records the accepted admin action in its audit trail. The command is neither a kind 45010 revision nor an event in an ordinary subscription. A client with artifact-signing access alone cannot sign this command through that capability.

A signed HTTP `/query` policy lookup takes one exact type. For example, `{"artifact":"policy","scope":"community","#type":["example.settings"]}`. It returns `{"policy":"everyone","revision":null}` for an unset type, or the current policy and accepted command ID. The relay advertises and enforces a bound on stored policy entries. Switching a configured type back to `everyone` retains its revision.

A committed policy change governs new revisions, including updates, deletes and restores of existing records. It neither rewrites records nor changes their audience. A write and a policy change MUST be serialized so their admission and commit order agree. Take the same policy lock before the artifact's identity lock, including if the type has never had a policy. Recheck the write policy in the transaction. An older pending write that still qualifies must still satisfy the head compare-and-set. An exact previously accepted artifact replay by the same signer, with current community read admission and an unrestricted `messages:write` credential, returns only acknowledgement. It does not re-evaluate the edit policy, timeout or freshness, and it does not write or publish again. A ban still refuses the replay. Check current community admission and the signed community before consulting receipts.

### Reading and syncing

Select the community scope explicitly on signed HTTP views `current`, `head` and `history`. For example:

```json
["REQ", "s1", {"artifact":"current","scope":"community","#type":["example.settings"]}]
```

This sync selects one exact type and can use the existing exact tag predicates. `#h` is forbidden, and `head` still requires `#d`. Apply admission and scope before any filtering, counting or paging. A legacy filter still sees only channel artifacts. Generic event reads, search, counts, direct ID fetches and ordinary live fan-out MUST exclude community artifact payloads. Route authorized reads through the explicit artifact views. HTTP incremental tokens and deltas remain channel-only in this version. Reject attempts to apply them to community scope.

Use the same snapshot, buffer bounds, `EOSE` and head sequencing as channel sync. Confirm live interest in the authenticated tenant before opening the snapshot. Only pass change signals to sync, and re-read authorized heads before delivery. A tag or deletion transition follows the existing before-and-after match rule. There is no kind 45011 move removal for community scope. Recheck current access outside the old snapshot before sending pages, `EOSE` and live heads. An authorization change invalidating the sync MUST close it with `CLOSED "resync: ..."` or disconnect the session if that is how the relay revokes access. This includes changes in direct or delegated membership, community admission mode, bans, and credential revocation. End the sync on a failed recheck. A failed or revoked initial read cannot send `EOSE`. After loss of access, the client discards the active set for that community; after re-admission it starts with a fresh complete snapshot. Follow existing sync rules for loss of pub/sub, output and redaction signals. Changing an edit policy alone changes no content or read access.

### Delete and moderation

A community soft delete reserves its `d` under the same rules as a channel soft delete. The authorized way to recreate a canonical key is to read its deletion head and send `restore` with that ID in `prev`. A rename to a different key is a create and delete with separate commits. Existing references still address the old identity. A kind-5 deletion cannot target a community artifact revision.

For community moderation, use a community-bound kind 9005 command with a single target revision `e` and no `h`. Verify that the target is a community-scoped kind 45010 revision in this tenant. Require the existing moderation write scope and direct owner or admin authority, independent of who can edit its type. Redact through the existing moderation path and retain the accepted revision ID. Redaction of a current head ends any affected sync with resync. This command does not emit a channel removal or update chat counters. Deleting the community itself purges both scopes, their acceptance ledgers and the type policies.

A relay MUST enable community writes and advertise the capability only after all backends that can serve the community enforce the community audience on queries, lookups, moderation and delivery.

## Incremental reads

Incremental reads maintain an eventually consistent local store. They do not prove a complete set at one database snapshot. Absence from a page never proves deletion or loss of access.

This HTTP protocol uses `artifacts.incremental`. The WebSocket Sync subscription uses a separate `artifacts.sync` capability and defines its own complete-set guarantee. Clients select each protocol through its own capability.

The relay advertises `artifacts.incremental` when incremental reads are enabled.
Each signed `POST /query` request contains one filter. The query uses the
same community, relay admission, channel access, and exact tag rules as an
ordinary artifact read. The existing `current` and `history` array responses
keep their format.
Unlisted open channels require membership or an explicit `#h` scope.
Known-ID reads do not bypass this rule.

```json
[{"artifact":"incremental","query":{"artifact":"current","#type":["buzz.task"]}}]
```

The initial read pages through current identities. A later read supplies the
last completed `checkpoint`. A continuation supplies only `cursor`, with
the same `query`. The server signs these opaque strings. They bind the
community, reader, exact query, access-policy boundary, and database start
time. Tokens contain a fixed-size digest of the query. The cursor also binds
the last page position. Relay instances use the
same configured signing identity, so another instance can serve a continuation.
No server-side manifest or Redis allocation is required.

```json
{
  "version": 1,
  "startedAt": "2026-10-21T12:00:00.123456Z",
  "overlapSeconds": 120,
  "changes": [
    {"type":"upsert","artifactId":"22222222-2222-4222-8222-222222222222","event":{}},
    {"type":"remove","artifactId":"33333333-3333-4333-8333-333333333333"},
    {"type":"removeChannel","channelId":"44444444-4444-4444-8444-444444444444"}
  ],
  "nextCursor": null,
  "checkpoint": "opaque"
}
```

`event` is the complete signed current revision. `remove` has one meaning:
the requested artifact is unavailable for this query. It does not distinguish
a missing identity, deletion, redaction, query exit, or denied access.
`removeChannel` names a removed membership retained for this exact reader.
It applies only to local rows whose current saved home is that channel.
An earlier move upsert to another home therefore survives a later source
channel removal.

Each pass fixes `startedAt` from the database clock. A later pass reads from
the previous completed start time minus 120 seconds through its own start
time, inclusive. New head writes, move-source records, and membership changes
use database `clock_timestamp()`. The protocol assumes the delay from this
stamp to commit visibility stays within the accepted 120-second overlap.
That overlap is an operating assumption. It is not a measured upper bound.

Pages order by `(change timestamp, source, stable identity)`. The source
separates membership removals, head changes, grants, and move-source records.
The identity breaks equal timestamp ties. It does not encode creation time.
The time window and page position are stable. The current contents can change
between requests. Replays must be idempotent. A later pass repairs changes
that move beyond the current window.

Only the final page returns a checkpoint. The client saves it after every
page and local commit succeeds. A failed read keeps the prior checkpoint.
Neither an initial read nor delta completion establishes absence. The client
must not remove unseen rows. A fresh access grant replays older current
heads from that channel. A move retains its source channel in the existing
revision ledger, so a reader who cannot read the destination receives a
current revalidation result without destination metadata.

### Current reads and write confirmation

```json
[{"artifact":"incremental","action":"current","query":{"artifact":"current","#type":["buzz.task"]},"artifactIds":["22222222-2222-4222-8222-222222222222"]}]
```

The response has `version`, database `startedAt`, and `changes`. It contains
exactly one upsert or remove for each requested ID, in request order.
The request permits 1–100 unique nonempty strings, each at most 256 UTF-8
bytes. A string that is not a canonical artifact UUID returns remove.
Duplicates and invalid bounds fail with `incremental_invalid`. The server does not
truncate the request.

Clients serialize a full delta pass and its commits before a current-ID read.
Live revision pushes request a current-ID read; they do not replace confirmed
rows directly. A current-ID read that starts after publication acceptance
reads that commit or a later state on the writer. It can return a later head
or remove. An empty delta does not confirm an accepted edit.

### Access recovery and bounds

Relevant channel access changes atomically advance one community policy
stamp, including transitions on empty channels. Ordinary artifact writes do
not update this stamp. A different stamp
returns HTTP 409 `incremental_reset` with `reason: "policy_changed"`, without naming
private channels. A checkpoint older than 30 days returns the same code with
`reason: "checkpoint_expired"`.

Recovery runs a new initial read to discover current readable records and
bounded current-ID reads for the IDs already saved locally. These reads
revalidate known IDs without a channel roster or an absence scan. Save new
progress only after discovery, revalidation, and local commits succeed.
Preserve preferences when a record becomes unavailable.

The server retains task heads, membership removal rows, and move-source
ledger records. Removing this recovery history requires an explicit policy
reset before a client can resume. Whole-community deletion closes admission
and purges these rows through the existing deletion path.

NIP-11 advertises:

```json
{"version":1,"overlapSeconds":120,"pageRows":100,"currentRows":100,"responseBytes":8388608,"permanentChannelsOnly":true,"checkpointLifetimeSeconds":2592000}
```

Pages shrink to the response byte bound. A single oversized result returns
HTTP 413 `incremental_limit`. A current-ID batch can return that error; the client
can split the batch. No page or batch silently truncates its result. The
server permits four concurrent operations per process, a two-second SQL
statement budget, and an eight-second operation budget. There is no
identity-wide membership count limit on this path.

Incremental reads include only matching readable artifacts in permanent channels.
TTL channels do not contribute discovery candidates. A known-ID read returns
`remove` for an artifact in a TTL channel. Moves into a TTL channel return
removals for the permanent source. An explicit `#h` scope limits candidates
to those channels, including retained move sources and membership removals.
Initial reads omit deleted heads. Later reads retain deletion removals.
HTTP 400 `incremental_invalid`
rejects invalid requests or tokens. HTTP 503 `incremental_unavailable` reports a
service or work-budget failure. These failures do not advance progress.

Incremental completion does not establish a live subscription to every channel.
