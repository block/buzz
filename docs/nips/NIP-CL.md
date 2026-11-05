NIP-CL
======

Channel Identity and Discovery
------------------------------

`draft` `optional` `relay`

This NIP adds identity facts to the relay-signed NIP-29 channel-state events
(kinds 39000–39002). With these facts, a client can find channels with
ordinary NIP-01 filters. The optional compact membership modes below use
explicit filter extensions on the existing `POST /query` bridge.

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

## Compact membership modes

These independent, optional query modes reserve distinct response kinds:

| Mode / NIP-11 capability | Kind | Subject | Scope |
| --- | --- | --- | --- |
| `membership_snapshot` v1 | 39008 | NIP-98 signer only | Own active memberships, including private channels and DMs as specified below. |
| `membership_public` v1 | 39009 | Any public key | Active memberships in listed, open forum/stream/workflow channels only. |

Kinds 39008 and 39009 are relay-only, request-bound projections. Support for
39008 does not imply support for 39009, or conversely. Each mode key MUST
appear only with its own kind; a mismatched mode key and kind returns 400.

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

The operator switch for this capability is off by default.
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

Like the 39005–39007 query overlays, kind 39008 occupies the addressable
range but is an on-demand projection: it is never persisted, replaced, broadcast or returned
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

The relay MUST derive the complete response from one consistent read of
committed state that reflects every change committed before the relay
received the request. Select only the viewer's active memberships in nondeleted
channels in the same community, ordered by channel UUID. Channel UUID,
role, type, visibility and archive state MUST reflect that same read.
The projection MUST NOT include full rosters, other members' keys or profiles.

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

### Compatibility

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
query shape, type opt-ins and signatures intact. Disabling the capability
requires clients to recheck support and explicitly select legacy authority
as above.

## Compact public membership lookup (version 1)

This independent mode answers “which public channels is this person in?”
without loading their peers' roster entries. It does not relax the self-mode
rule that kind 39008's `#p` MUST equal the NIP-98 signer. In this section,
**requester** means the authenticated signer and **subject** means the public
key whose memberships are requested. “Public” means listed and open within
the host-resolved community, not accessible across community boundaries.

### Capability and request

An enabled relay advertises this separate descriptor in its host-bound
NIP-11 information document:

```json
{
  "membership_public": {
    "version": 1,
    "community_id": "<community UUID>",
    "pubkey": "<relay signing public key>",
    "kind": 39009,
    "max_channels": 4096,
    "max_response_bytes": 2097152
  }
}
```

The operator switch for public lookup is off by default and independent of
the self-snapshot switch. Disabled relays omit this descriptor and
return HTTP 404 with `{"error":"membership_public unsupported"}` for this
mode. An unresolved host never advertises it. Clients MUST obtain and pin
this descriptor using the TLS-origin and relay-key rules of self mode.

Send exactly one filter to `POST /query`:

```json
[{
  "kinds": [39009],
  "#p": ["<subject public key>"],
  "#t": ["forum", "stream", "workflow"],
  "membership_public": 1
}]
```

`#p` MUST contain exactly one lowercase 64-character hex public key. The
nonempty `#t` array MUST be sorted, unique, and contain only `forum`,
`stream`, or `workflow`; a subset is allowed. Unknown types (including `dm`
and `system`), versions, extra keys or constraints, multiple filters, mixed
kinds, or mixing `membership_public` with `membership_snapshot` return 400.
A relay MUST NOT silently narrow the requested set.

A valid fresh NIP-98 event MUST bind the exact body, canonical host-derived
query URL and POST method, even when unsigned development reads are enabled.
The normal bridge admission, community membership/ban policy and replay
guard apply to the requester. Any requester admitted by those rules MAY
query any subject, including themselves or their owner. The requester is
always the signer, never `token_owner` or the NIP-OA owner. No relationship
between requester and subject expands the public scope or grants private
access. A subject need not have a profile or current community membership;
selection uses their active channel memberships in this community only.

Every attempt MUST consume the requester's existing per-community API-call
quota before selection. Changing subjects MUST NOT
reset that quota. Exhaustion returns 429 using the bridge's existing
`rate-limited: quota exceeded; retry in <seconds>s` error convention; shared
admission unavailability returns 503 and fails closed. Clients MUST respect
that retry delay, back off and jitter further attempts; no new per-subject
quota or auth nonce scheme is introduced.

### Signed response and verification

Success is an object with its own version marker and response kind:

```json
{
  "membership_public": 1,
  "event": {
    "kind": 39009,
    "pubkey": "<relay signing public key>",
    "tags": [["d", "<NIP-98 event ID>"], ["p", "<subject>"]],
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
  "scope": "public",
  "community_id": "<host-resolved community UUID>",
  "host": "<canonical resolved request host>",
  "requester": "<NIP-98 signer public key>",
  "subject": "<requested public key>",
  "request_id": "<NIP-98 event ID>",
  "types": ["forum", "stream", "workflow"],
  "complete": true,
  "snapshot_id": "<SHA-256 content identity>",
  "channels": [{
    "id": "<channel UUID>",
    "role": "member",
    "channel_type": "stream",
    "visibility": "open",
    "archived": false
  }]
}
```

Kind 39009 is relay-only, query-time data: it MUST NOT be persisted,
replaced, broadcast, returned by ordinary REQ, COUNT or `/query`, or accepted
through client EVENT. Its `d` tag is a request coordinate, not a durable list.

The client MUST verify the envelope marker/version, full Nostr ID and
signature, kind 39009 and pinned relay key, exactly one `d` and one `p` tag
with the values shown, and every signed binding: schema version,
`scope:"public"`, community UUID, canonical host (the normalized host of the
NIP-98 `u` URL for this request), requester equal to the current signer,
subject equal to the requested `#p`, outstanding request ID and exact type
array. The request MUST belong to the current requester/community/session
and the currently selected subject; consume it at most once. Canceled,
superseded, timed-out or previous-session results MUST NOT replace current
discovery data. Neither timestamps nor hashes order responses.

Arrays (including `[]`), malformed envelopes, missing/false completeness,
invalid signatures, binding mismatches, unknown fields needed to interpret
the result, duplicate channel IDs, invalid roles/types, non-boolean archive
state or visibility other than `open` MUST fail validation. Failure is never
a complete empty list. A valid signed empty list means only that no matching
public memberships existed at that cut. A saved response is display data;
its signature and identity do not prove present visibility or authorization.

### Selection, completeness and bounds

The relay MUST derive the complete response from one consistent read of
committed state that reflects every change committed before the relay
received the request. Select the subject's active memberships in nondeleted
channels in the same community, restricted to requested types, open and not
unlisted, ordered by channel UUID. This is the listed-open discovery
predicate, with no shared-private-channel exception. All entry fields MUST
reflect that same read. The projection MUST NOT include full rosters,
peers' public keys or profiles.

There is one entry per matching membership. Preserve the subject's role
(`owner`, `admin`, `member`, `bot` or `guest`); include archived channels with
`archived:true`. Exclude removed memberships, deleted channels, private or
unlisted channels, DMs and system channels, even when the requester is a
member too. Channel names and other metadata remain in kind 39000 under
existing read rules.

`complete:true` means all of the subject's memberships satisfying this
public predicate and requested type set at that instant. It never means
the subject's full membership set. A channel absent on refresh may have
become private, unlisted or deleted, or its membership may have ended;
absence MUST NOT establish loss of membership.

The snapshot identity is SHA-256 of the UTF-8 compact JSON array
`["nip-cl-public-membership-v1", community_id, subject, types, channels]`,
with UUIDs/pubkeys in lowercase and channel object keys in the order shown.
Clients MUST recompute it from the verified signed fields. Requester and
request ID bind the signed response but are excluded from content identity;
identical public sets may have identical identities across requesters.
The identity is neither a revision nor a live cursor.

The same bounds as self mode apply: at most 4096 matching channels and
2097152 encoded response bytes, probing one extra matching row. Either
overflow returns 413 for the whole request, with no success envelope or
partial list. Database/signing failure returns an error, never complete
empty state. There are no pages, tokens, server snapshot caches or resume
semantics in v1. A retry requires fresh NIP-98 auth.

### Discovery authority and compatibility

A validated result MAY replace a cached public discovery list keyed by
community, subject and exact requested types. It MUST NOT populate or alter
the requester's own membership/role authority, canonical kind-39002 roster
caches, send permissions or the self-mode 44100/44101 snapshot-to-live
handoff, even when requester equals subject. Roles describe the subject;
they are not grants to the requester. The self-mode handoff does not apply.
Ordinary read/write authorization remains independently enforced, and
saved public data MUST NOT authorize a read after a visibility change.

Clients MUST require `membership_public` before selecting this mode. Absence
or the exact disabled 404 means this lookup is unsupported; an ordinary
array from an older relay is never a valid public result. If a pod returns
an ordinary array or the exact disabled 404 after another pod advertised
support, mark public lookup unsupported for that generation, recheck its
host-bound descriptor and back off capability probes. Keep any saved public
list explicitly stale; this does not switch the self authority model.
Auth failures, invalid signatures, malformed responses and transient errors
MUST NOT be accepted as an empty result or silently retried through self mode.
Public and self capabilities can be enabled and disabled independently.
Turning off public lookup stops its use without changing the
self-membership authority/session generation or deleting saved display data.

### Normative examples and rejection cases

In these examples A signs fresh body-bound NIP-98 auth and B is the subject:

| Request / response | Required outcome |
| --- | --- |
| A requests B's `forum,stream,workflow` memberships; B belongs to listed-open stream X, archived listed-open forum Y, private Z shared with A, an unlisted stream, a DM and a system channel. | Return X and Y only, preserving B's roles and marking Y archived. |
| A is B's agent and its admission auth tag names B as owner. | The same public-only X/Y result; no owner's private memberships. |
| A requests A with `membership_public:1`. | The same public predicate; never private/DM/system memberships or self authority. |
| A requests B with `membership_snapshot:1`, kind 39008. | 400; self mode still requires subject equal to signer. |
| Public request contains `#t:["dm","stream"]`, `#t:["system"]`, duplicate/unsorted types, two subjects, both mode keys, or an added `limit`. | 400; never silently drop forbidden constraints or types. |
| A receives a validly signed answer for requester C, subject C, another host/community, another type subset or another request ID. | Reject the binding mismatch. A's outstanding request is unchanged. |
| A receives kind 39008 or the self envelope for its public request, or kind 39009 for its self request. | Reject the cross-mode response, including when A equals B. |
| X becomes private or unlisted before a later lookup's cut. | Omit X from the new public list even if A still shares X; replacing the public cache is allowed, revoking membership from the omission is forbidden. |
| B has no matching public memberships, or has never had a community profile. | Return a valid signed complete empty list if selection is empty; do not disclose excluded memberships. |
| Selection or encoding overflows, selection/signing fails, or requester quota is exhausted. | Return the specified whole-request error; never a partial or complete-empty success. |

## Trust

A client that relies on these tags MUST check that the relay's key signed the
event, as for all NIP-29 group-state events.

Labels give no trust. Any owner or admin can set them. Only `P` says who
created a channel.

## Relation to other NIPs

- NIP-29: the group-state events that this NIP adds tags to.
- NIP-01: the filters that find channels.
- NIP-22, NIP-34 and NIP-72: the convention for uppercase `P`.
