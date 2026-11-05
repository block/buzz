NIP-CL
======

Channel Identity and Discovery
------------------------------

`draft` `optional` `relay`

This NIP adds identity facts to the relay-signed NIP-29 channel-state events
(kinds 39000–39002). With these facts, a client can find channels with
ordinary NIP-01 filters. It needs no new filter key and no relay-specific API.

## Motivation

Plugins and agents often create a channel for their own use and must find it
again later, for example "the config channel that this agent created". With
plain NIP-29, a client cannot:

- tell who created a channel;
- read the channel type from any event except kind:39000; or
- tell which of two channels is older. The `created_at` of a kind:39000 event
  is the time of the last edit, not the time of creation.

A client also cannot mark its own channels so that it can find them again.
Channel labels (below) do this.

Also, every channel that a person belongs to shows in their client as a chat.
So a group that exists only to control who can read some data appears in
sidebars, search results and unread counts. System channels (below) fix this.

## Channel identity tags

On every channel-state event that the relay signs, the relay MUST put these
tags in this order, before any other `t` or `P` tag:

1. `["t", <channel type>]`, for example `stream`, `forum`, `dm`, `workflow`, `system`.
2. `["t", <label>]` for each channel label, in stored order (see
   [Channel labels](#channel-labels)).
3. `["P", <creator public key, hex>]`.

kind:39000 also carries `["created_at", <unix seconds>]`.

Example kind:39000 (signing fields and unrelated tags omitted):

```json
{
  "kind": 39000,
  "pubkey": "<relay public key>",
  "created_at": 1760025600,
  "tags": [
    ["d", "9b353519-f4fe-4757-aef4-bec6cc0ae54c"],
    ["name", "agent-config"],
    ["t", "stream"],
    ["t", "agent-config"],
    ["P", "<creator public key>"],
    ["created_at", "1759939200"]
  ]
}
```

| Tag | Kinds | Meaning |
| --- | --- | --- |
| first `t` | 39000–39002 | Channel type. |
| later `t` | 39000–39002 | Channel labels. |
| `P` | 39000–39002 | Public key that signed the kind:9007 that created the channel. |
| `created_at` | 39000 | Time the channel was created, in unix seconds, as a decimal string. |
| `auto_join` | 39000 | Present (bare, no value) while the relay joins new community members into this channel: the channel's `auto_join` setting is on and it is open and unarchived. |

**Channel type.** Clients MUST read the channel type from the first `t` tag.
Later `t` tags are labels.

**Creator.** The relay writes `P` from its own record of the kind:9007 that
created the channel. No client can set it. It does not change when ownership
moves to another member or when the channel is edited.

Uppercase `P` follows NIP-22, NIP-34 and NIP-72, where `P` names the author of
the root object. On these kinds, lowercase `p` already lists members, admins
and DM participants. A filter matches tag values, not their roles, so `#p`
would mix creators with members.

**Creation time.** The `created_at` tag does not change when the channel is
edited. Use it, not the event's `created_at`, to compare the age of channels.

**Older events.** A relay adds these tags only when it publishes a
channel-state event. An event that a relay version without this NIP published
can lack them, including one published during an upgrade. Such an event stays
until the channel next changes, for example on an edit or a membership
change. A client MUST treat a missing tag as unknown. A missing `P` does not
mean that a key did not create the channel.

## Channel labels

A label groups channels across channel types, for example `workspace` or
`agent-config`. Labels have no meaning to the relay.

**Set labels.** A kind:9007 (create group) MAY carry up to 8 distinct
labels as `["t", <label>]` tags. On kind:9002 (edit metadata):

- `t` tags replace the whole label set;
- `["t", ""]` tags and no other `t` tag clear the set;
- no `t` tag leaves the labels as they are.

Only an owner or admin of the channel MAY change its labels.

**Rules.** A label is 1–64 characters from `a-z`, `0-9`, `.`, `:` and `-`. A
label MUST NOT be `stream`, `forum`, `dm`, `workflow` or `system`. These are
channel type names, so this rule stops a label from making a channel look like
another type. The relay MUST reject the whole event if it has more than 8
distinct labels, an invalid label, a reserved name, or `["t", ""]` together
with other labels. The relay stores repeated labels once, so a repeat does
not count toward the 8, and repeated `["t", ""]` tags clear the set like one.

## Finding channels

Use ordinary NIP-01 filters on the channel-state kinds.

| To find | Filter |
| --- | --- |
| Channels that a key created | `{"kinds": [39000], "#P": ["<creator>"]}` |
| Channels with a label | `{"kinds": [39000], "#t": ["<label>"]}` |
| One creator's channels with a label | `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}` |
| Channels of one type | `{"kinds": [39000], "#t": ["<type>"]}` |

A `#P` filter does not return a channel whose events lack `P` (see
**Older events**). So it can miss channels that a key created before the relay
supported this NIP.

Values in one tag filter combine with OR, as in NIP-01. So
`"#t": ["workspace", "notes"]` finds channels with either label. To require a
label and a creator, use `#t` and `#P` together.

The relay MUST match `#t` and `#P` before it applies `limit`, as NIP-01
requires. A relay that reads the newest channel events first and matches tags
after can return a short or empty page while matching channels exist.

## Choosing one channel

The relay does not make labels unique. Many channels can share a label, and
any owner or admin can set any label. So two creates for the same label can
both succeed, for example when two processes of one agent start at the same
time.

A client that needs one channel per creator and label:

1. queries `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}`;
2. creates the channel only if the query returns nothing; and
3. when the query returns more than one channel, uses the oldest one.

The oldest channel has the lowest `created_at` tag value, compared as an
integer. If two channels have the same value, the lowest channel ID (`d` tag,
compared as a string) wins. A channel without a `created_at` tag ranks after
every channel that has one, and two channels without one compare by channel
ID.
Every reader that follows this rule picks the same channel, even after either
channel is edited.

## System channels

A system channel exists only to control who can read some data, for example
config that only an agent and its owner can read. It has the channel type
`system`. In every other way it is an ordinary channel: it has members, roles,
messages and labels.

**Create.** A kind:9007 with `["channel_type", "system"]` creates a system
channel. It is private unless the kind:9007 also has `["visibility", "open"]`.

**Hidden.** While a system channel is private, its kind:39000 carries the
NIP-29 `hidden` tag.

### Reads that leave system channels out

The relay leaves system channels out of every read that does not name them.
Clients that do not know about system channels then stay correct with no
change.

A client sees a channel only if it knows the channel's type. A read that does
not name a channel returns only the **legacy types**: `stream`, `forum`, `dm`
and `workflow`, the types that every client knew before this NIP. This list is
frozen. The relay leaves out every other type, so a channel type added later
is also hidden from clients that do not know it. A filter that names such a
channel (see below) reads it under the ordinary rules, and no other filter in
the same request gains it. The opt-in in "Asking for system channels"
(`#t:["system"]` or `#P`) adds system channels only, never a channel of
another type. Each new type needs its own way to ask for all its channels.
This applies to:

- a filter without `#h`, for example the channels or messages a reader can see;
- kind:39002 `#p:["<reader>"]`, the channels a reader belongs to;
- COUNT and NIP-50 search; and
- any relay-specific query API that does the same reads.

A filter **names** a channel when it has the channel ID in `#h`, or in `#d` on
a filter whose kinds are all 39000–39003. This is the same for a system
channel and for a type added later. Then the ordinary read rules apply: the
reader can read the channel if they are a member or if it is open.
A client that has the ID, for example from a link, can open the channel. Thus
a system channel stays an ordinary NIP-29 group.

The relay decides this for each filter. A filter that names a channel does
not add it to another filter in the same request.

An **unlisted** open channel (Buzz's `unlisted` setting, see NOSTR.md) is
named-only in the same way for readers who are not members, whatever its
type. Its members read it like any other channel. The system-channel opt-in
never adds it.

Push notifications (NIP-PL) use the same rule. A lease filter wakes its
author for an event in a system channel or a channel of a type added later
only when the filter names that channel. A lease filter cannot have `#t` or
`#P`, so here "names" means `#h`. The relay still checks that the author can
read the event, as for every other channel.

### Membership notices carry the channel type

Every kind:44100 (added) and kind:44101 (removed) notice has one more tag,
`["t", <channel_type>]`, for example `["t", "stream"]` or `["t", "system"]`.
The relay sets it from the channel type when it sends the notice. A channel's
type does not change. The relay stores the notice like any other 44100/44101.

A client sees a notice only if it knows the notice's channel type. Old
clients do not know types added later, such as `system`, and would show the
channel from such a notice. So the relay MUST apply this rule:

- **A filter with `#t`** matches a notice only when the notice's type is one
  of the `#t` values.
- **A filter without `#t`** matches a notice only when its type is one of the
  **legacy types**: `stream`, `forum`, `dm`, `workflow`. This list is fixed.
  A type added later is never on it, so old clients never see its notices.
- **A notice without a `t` tag** was stored before this rule. Every such
  notice is about a legacy type, so the relay treats it as all of them: it
  matches a filter without `#t`, and a filter whose `#t` has a legacy type.
  On these two kinds, this replaces the plain NIP-01 `#t` match.

The rule applies on every path: REQ history, live delivery, COUNT, and any
relay-specific query API. Existing clients do not send `#t` on these kinds,
so they see no change.

A client that knows every current type sends one filter:

```json
{"kinds": [44100, 44101], "#p": ["<me>"], "#t": ["stream", "forum", "dm", "workflow", "system"]}
```

When Buzz adds a type, a client adds its value to `#t` when it supports that
type.

The `#p` rule for 44100/44101 still applies: it must be the reader's own
pubkey. An `ids` filter does not lift it: the relay returns, counts or
delivers a 44100/44101 only to the pubkey in its `p` tag, even to a reader
who knows the event id.

Because the notices are stored, a client that reconnects can set `since` to
the newest notice it has seen. One subscription then returns the notices it
missed and the new ones, so no change falls between a read and a
subscription. A notice is a hint: the channel's group-state events decide
membership.

### Asking for system channels

A filter whose kinds are all 39000–39003 also returns the system channels that
the reader can read when it has:

- `"#t"` with the value `system`; or
- any `"#P"`. A query for the channels of named creators returns a small
  result, so it does not fill generic channel lists.

The filter then matches as usual. A label alone does not ask for system
channels. These filters add only system channels. A channel of a type added
later reaches only a filter that names it.

| To find | Filter |
| --- | --- |
| My system channels | `{"kinds": [39002], "#p": ["<me>"], "#t": ["system"]}`, then their kind:39000 by `#d` |
| Open system channels | `{"kinds": [39000], "#t": ["system"]}` |
| One creator's channel with a label, system or not | `{"kinds": [39000], "#P": ["<creator>"], "#t": ["<label>"]}` |

`"#t": ["system", "<label>"]` means `system` **or** the label. It does not
mean "a system channel with this label". Use `#P` and the label instead.

Private system channels stay hidden from non-members, as for every private
channel.

## Advertising support

Until this draft has an upstream integer NIP number, relays MUST NOT advertise
it in NIP-11 `supported_nips`. A relay that supports this NIP adds `"nip-cl"`
to its NIP-11 `supported_extensions` string array.

A relay without this NIP drops label tags on create and refuses
`channel_type=system`. So a client SHOULD NOT create a channel with labels or
a system channel on a relay whose NIP-11 document does not contain `"nip-cl"`.
Lookups by label on such a relay find nothing, which is safe.

## Compact viewer membership snapshot (version 1)

This optional capability replaces full-roster discovery for clients that
implement the reconciliation rules below. It does not change kinds
39000–39003 or their legacy query behavior.

### Capability and request

An enabled relay advertises `membership_snapshot` in its host-bound NIP-11
information document:

```json
{
  "membership_snapshot": {
    "version": 1,
    "community_id": "<community UUID>",
    "pubkey": "<relay signing public key>",
    "kind": 39008,
    "max_channels": 4096,
    "max_response_bytes": 2097152
  }
}
```

The operator switch `BUZZ_MEMBERSHIP_SNAPSHOT_ENABLED` defaults to false.
Disabled relays omit the descriptor and return HTTP 404 with
`{"error":"membership_snapshot unsupported"}` for this query mode. Capability
advertisement alone is not authentication or membership authority. Clients
MUST obtain it from the community's TLS origin and pin the relay signing key
as they do for group-state events. An unresolved host never advertises it.

Send one exact filter to the existing `POST /query` bridge:

```json
[{
  "kinds": [39008],
  "#p": ["<authenticated viewer public key>"],
  "#t": ["dm", "forum", "stream", "system", "workflow"],
  "membership_snapshot": 1
}]
```

The nonempty type array MUST be sorted, unique, and contain only known types
(`dm`, `forum`, `stream`, `system`, `workflow` in version 1). A client MAY
request a subset. Including `system` is an explicit opt-in to system channels,
not a change to ordinary NIP-CL reads. Unknown types, versions, extra filter
keys, constraints, multiple filters or another viewer return 400. A relay
MUST NOT silently narrow the requested set. Future types need a reviewed
protocol update; unsupported clients do not discover or revoke them.

This mode requires a valid NIP-98 event whose `u` and `method` bind the
host-derived query URL and POST, and whose `payload` binds the exact body,
even when unsigned development bridge reads are enabled. The bridge's normal
admission, membership/ban policy, rate limit and replay guard apply. The viewer
MUST be the signer proven by NIP-98, never `token_owner` or the NIP-OA
owner used for admission. An agent asking for its owner's memberships gets
400, even when its auth tag names that owner. Use a
fresh NIP-98 event for every attempt (a random auth-event nonce tag avoids
same-second duplicates). No second nonce scheme is introduced.

### Signed response and verification

Success is an **object**, never an ordinary Nostr event array:

```json
{
  "membership_snapshot": 1,
  "event": {
    "kind": 39008,
    "pubkey": "<relay signing public key>",
    "tags": [["d", "<NIP-98 event ID>"], ["p", "<viewer>"]],
    "content": "<JSON object below>",
    "id": "<event ID>",
    "created_at": 1793899000,
    "sig": "<signature>"
  }
}
```

The signed content is:

```json
{
  "version": 1,
  "community_id": "<host-resolved community UUID>",
  "host": "<canonical resolved request host>",
  "viewer": "<authenticated viewer public key>",
  "request_id": "<NIP-98 event ID>",
  "types": ["dm", "forum", "stream", "system", "workflow"],
  "complete": true,
  "snapshot_id": "<SHA-256 content identity>",
  "channels": [{
    "id": "<channel UUID>",
    "role": "guest",
    "channel_type": "stream",
    "visibility": "open",
    "archived": false
  }]
}
```

Kind 39008 is registered as `KIND_MEMBERSHIP_SNAPSHOT` in
[`buzz-core/src/kind.rs`](../../crates/buzz-core/src/kind.rs). Like the
39005–39007 query overlays, it occupies the addressable range but is an
on-demand projection: it is never persisted, replaced, broadcast or returned
by ordinary REQ, COUNT or `/query`. Clients cannot submit it through EVENT.
The `d` tag is a request coordinate, not a durable per-viewer list.

The client MUST verify the envelope version, full Nostr ID and signature,
expected kind and pinned relay key, both tags, and **all** signed binding
inputs: schema version, host-resolved community UUID, canonical host (the normalized host of the NIP-98 `u` URL used for this
request, not an alias inferred from another community URL), viewer,
outstanding NIP-98 event ID and exact requested type array. The request ID
MUST belong to the current viewer/community/session generation and be consumed
at most once. A canceled, superseded, timed-out or previous-session request
cannot establish current membership. `created_at` is informational wall clock;
it MUST NOT order responses, prove freshness or override a newer revocation.

The signature provides verifiable provenance when the result is cached or
exported. A saved response is display data, never fresh authorization. Only
matching the outstanding request proves an on-demand response; neither its
signature nor `snapshot_id` is proof of current membership on its own.

Arrays, including `[]`, malformed envelopes, missing/false completeness,
invalid signatures, binding mismatches, unknown fields needed for authority,
duplicate channels or invalid roles/types/visibility MUST fail validation.
Never interpret these as an empty complete list. An empty **valid signed**
`channels` list is authoritative only for its requested types and snapshot cut.

### Canonical selection and bounds

The relay reads the primary database in one SQL statement's MVCC snapshot.
It selects only the viewer's active `channel_members` rows, joined to live
`channels` in the same community, ordered by channel UUID. It does not load,
parse or serialize full rosters, other members' keys, or profiles. The existing
partial `(community_id, pubkey) WHERE removed_at IS NULL` index supports this
projection. Channel UUID, role, type, visibility and archive state all come
from that same statement, never a separately sampled version query.

There is one entry per active membership in the requested type set:

- Roles retain `owner`, `admin`, `member`, `bot`, and `guest`. A guest remains
  read-only under existing send-time policy; membership is not a write grant.
- Open, private, unlisted and explicitly requested system memberships are
  included. Open channels where the viewer is not a member are excluded.
- Archived channels are included with `archived:true`; deleted channels and
  removed memberships are excluded.
- Hidden DMs are included, as in kind-39002 discovery. NIP-DV kind 30622 owns
  the separate per-viewer hide overlay; hiding is not leaving a DM.
- Names, avatars, labels, creation identity, join policy and other channel
  metadata remain in kind 39000. No member list is needed to use this result.

The snapshot identity is SHA-256 of the UTF-8 compact JSON array
`["nip-cl-membership-state-v1", community_id, viewer, types, channels]`, with
UUIDs/pubkeys in lowercase and channel object keys in the order shown above.
Clients MUST recompute this identity from the verified signed fields; it
describes content, not a monotone revision, transaction ID or live cursor.
A role change, removal or channel-state transition committed after the cut is
visible on a subsequent refresh, even if legacy roster re-signing fails.

Version 1 permits at most 4096 channels and 2097152 encoded response bytes.
Selection probes one extra row. Either overflow returns 413 for the whole
request, with no success envelope or partial list. Database/signing failure
returns an error, never complete empty state. There are no pages, tokens,
server snapshot caches, or resume semantics in v1. Retry the whole projection
with fresh auth; a larger-account paging design requires a new version with
an immutable cut, authorization, expiry and resource bounds.

### Authority and snapshot-to-live handoff

A snapshot describes one committed cut; it does **not** fence live delivery.
The supported client path MUST keep viewer membership and role authority
separate from full-roster availability:

1. Establish the authenticated viewer's live 44100/44101 subscription for all
   requested types before issuing the snapshot. Treat these notices as hints,
   never an ordered or lossless delta log. Observe connection gaps, explicit
   denials and changes to projected archive/visibility/type/deletion state as
   invalidations too. Names or avatars alone do not dirty this projection.
2. Fence the request by account, tenant and session generation. Increment an
   invalidation generation for every relevant hint received after dispatch.
   Buffer these hints while loading. A changed generation makes the result
   dirty: for every channel named by a hint after dispatch, ignore that
   result's entry entirely (presence, absence and role). Keep prior state
   marked stale, including any denial already applied; suspend actions that
   depend on an unconfirmed role. For unhinted channels a dirty result may
   only add memberships, never alter existing roles or remove by omission.
   A global invalidation discards the whole result. Coalesce a follow-up
   refresh after processing the hints. A clean valid
   snapshot may reconcile omissions only within its exact requested type set.
3. Serialize refreshes per viewer/community and fence delayed completions.
   Never order snapshots by timestamps or hashes. Local access denials or
   removal/deletion hints immediately invalidate subscriptions, dependent UI
   and roster caches; a delayed response cannot undo them. A later clean
   request may establish a real re-add. Duplicate/out-of-order hints may cause
   redundant invalidation, never grant authority.
4. Lazy kind-39002 responses serve other-member features. They MUST NOT add,
   remove or change the viewer's own membership or role on this path, even
   when their timestamp is newer. Contradictions invalidate and schedule a
   compact refresh. Preserve existing send-time permission checks; a roster
   or snapshot cannot override a relay denial.
5. Refresh after every reconnect/gap before claiming freshness. Reconnect costs
   one compact projection per viewer, never one roster read per channel. On
   resume, reconnect or capability changes, recheck the host-bound descriptor.
   Coalesce notices, jitter dispatch and exponential failure backoff, and cap
   immediate repair to three attempts per recovery episode. Continuous churn
   then leaves explicit stale/incomplete state until the next bounded episode;
   it MUST NOT start an endless roster reload or certify dirty omissions.
6. Because notifications are post-commit and may be lost, refresh on foreground
   resume and use a jittered 15-minute backstop while active, with no overlapping
   reads. This bounds **attempts**, not successful convergence during outage or
   churn. Successful quiescent refresh repairs missed roles, grants, revocations,
   deletion and metadata transitions. Display stale state when repair fails;
   the relay still authorizes every operation against current access.

The relay continues to enforce current read/write authorization independently.
This response is discovery authority, never a bearer capability. Relay-wide
ban/removal invalidates the session even if channel rows remain active.

### Compatibility and rollout

A client MUST require the descriptor before selecting this mode. An explicit
absent/disabled capability uses legacy full-roster discovery. If a pod returns
an ordinary array or the exact disabled 404 error after another pod advertised
support, mark compact discovery unsupported for that generation, recheck the
descriptor and back off capability probes. Explicitly select a new legacy
authority/session generation and run full legacy validation; the array is
never a complete compact snapshot. This handles mixed-version fleets without
revocation by empty response. An auth error,
invalid signature, malformed envelope or transient failure MUST NOT silently
switch authority models. Old relays may ignore unknown filter fields and
return an ordinary array; this is not a valid compact response. Rollback may
explicitly select legacy discovery with a new authority/session generation,
without deleting saved display/user data. New relays keep legacy 39002 events,
query shape, type opt-ins and signatures intact. Roll out the image to the
**entire fleet with the flag off**, then enable the flag in a separate rollout.
Rollback turns the flag off; clients recheck support and explicitly select
legacy authority as above. Client opt-in and production-shaped throttled
validation follow separately.

## Trust

A client that relies on these tags MUST check that the relay's key signed the
event, as for all NIP-29 group-state events.

Labels give no trust. Any owner or admin can set them. Only `P` says who
created a channel.

## Relation to other NIPs

- NIP-29: the group-state events that this NIP adds tags to.
- NIP-01: the filters that find channels.
- NIP-22, NIP-34 and NIP-72: the convention for uppercase `P`.
