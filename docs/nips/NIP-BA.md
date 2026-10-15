NIP-BA
======

Brokered Agent Operations
-------------------------

`draft` `optional` `Buzz-local, unassigned upstream`

An agent delegates named application operations to a **host** that holds its
Nostr identity key. The agent holds a public identity and a sensitive session
credential, not that durable key. The host authenticates, authorizes, constructs,
signs, and publishes ordinary Nostr events; reads also pass through the host.
This specification defines their HTTP boundary, not a relay extension or a host
implementation. MUST, MUST NOT, SHOULD, and MAY are normative requirements.

This is a closed Buzz capability profile. It relies on
[NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md),
[NIP-10](https://github.com/nostr-protocol/nips/blob/master/10.md),
[NIP-29](https://github.com/nostr-protocol/nips/blob/master/29.md), and the local
[NIP-AE](NIP-AE.md), [NIP-AO](NIP-AO.md), and [NIP-OA](NIP-OA.md) conventions.
No generic signing, publishing, encryption, decryption, or relay-authentication
action is exposed. Adding capabilities requires revising this specification. Channel UUIDs are a
Buzz restriction; upstream NIP-29 accepts arbitrary group IDs. The marked-root/
reply convention from NIP-10 is reused for Buzz kind 9, not a claim that NIP-10
itself defines kind 9 messaging.

[NIP-46](https://github.com/nostr-protocol/nips/blob/master/46.md) already keeps
identity keys elsewhere and permits inspection and refusal of complete unsigned
events. The distinction here is **executing application operations and mediating
reads**, not an inability to constrain NIP-46 signing. Layering a NIP-46 signer
behind a broker is possible but not specified, including memory-key derivation.

## Session and transport

The endpoint and credential are provisioned out of band. Each credential MUST
bind to one agent principal, community, owner relationship, and permitted scope.
The host MUST derive these from authentication, never from request content.
Channel IDs and agent selectors are targets, not grants. Authorization MUST
include current community/channel membership and ownership where applicable;
the broker cannot expand relay authority. Credentials MUST be revocable and MUST
NOT be reassigned to another principal or community.

```http
POST /v1/action
Authorization: Bearer <session credential>
Content-Type: application/json

<request bytes>
```

Use TLS with server authentication, except for a deliberately configured
loopback endpoint. Clients MUST NOT forward credentials on redirects and MUST
NOT send loopback traffic through a proxy. Hosts MUST authenticate every attempt,
including retries, and authorize both execution and release of stored results.
A revoked credential MUST NOT retrieve a cached read or outcome.

Every correlated verdict MUST be a JSON result with HTTP 200 and
`Content-Type: application/json`. Clients MUST attempt envelope validation
regardless of HTTP status; a valid correlated result takes precedence. A proxy
401, missing route, timeout, truncated body, or invalid result is a **transport
failure**, not evidence of non-execution. No usable `requestId` means no
correlated verdict is possible; the host MUST NOT execute such a request.

Hosts and clients MUST bound body sizes, concurrent work, timeouts, and retries.
Hosts MUST document deployment limits and reject oversized input before dispatch;
clients MUST use bounded backoff, not an unbounded retry loop. This version has
no discovery endpoint or automatic version negotiation.

## Wire and normalization

JSON is UTF-8. Objects are closed: unknown members, duplicate member names,
wrong types, and explicit `null` MUST be rejected at every structural depth.
This does not parse JSON embedded inside opaque strings. Integers MUST have
integer JSON syntax, be nonnegative, and fit the stated unsigned width; booleans
are not integers. All version fields are u16, counts/kinds u32, timestamps u64
Unix seconds. Senders MUST preserve integers exactly, including in JavaScript.

`?` below means optional by **omission**, never `null`. The optional `mentions`
array MUST be omitted when empty; explicit `[]` is invalid. Required arrays
(`messages`, `tags`, `updatedFields`) MAY be empty. `frames` MUST NOT be empty.
Optional booleans default false; senders SHOULD omit false, receivers accept it.

Before freezing a request, clients MUST validate and normalize. Hosts MUST
validate independently; normalization MUST NOT change the body used for the
retry digest. Trim surrounding Unicode White_Space from scalar arguments, then
apply limits; required and supplied optional scalars MUST be nonempty. Do not
trim `requestId`, cursors, message `content`, storage `value`, or observer
`payload`. The latter three MUST contain a non-whitespace character but retain
all original bytes. Limits count Unicode scalar values unless marked **bytes**.
Action names and wire discriminators are exact, not trimmed. Signed event
fields MUST NOT be normalized or rewritten.

| Type | Accepted form and canonical output |
|---|---|
| Channel | UUID, lowercase hyphenated output; input also accepts uppercase hex, 32 hex digits, braces around hyphenated form, or `urn:uuid:` plus hyphenated form |
| Hex | 64 hexadecimal characters, lowercase output |
| Pubkey | Hex encoding of a valid secp256k1 x-only public key; not an npub |
| Request ID | 1–128 bytes, each `0x21`–`0x7e` |
| Cursor | 1–256 bytes, each `0x21`–`0x7e`; opaque, retained verbatim |
| Name / prompt / scalar / about | At most 120 / 20,000 / 300 / 2,000 scalars respectively |
| Slug | `core` or `^mem/[a-z0-9][a-z0-9_-]{0,63}(/[a-z0-9][a-z0-9_-]{0,63})*$`, at most 255 bytes |

### Request

Exactly six required members:

```json
{"type":"broker_request","protocolVersion":1,"requestId":"example-1","actionVersion":1,"action":"storage.get","args":{"slug":"core"}}
```

`type` is `broker_request`; `protocolVersion` is 1. All fifteen actions below
have `actionVersion` 1. `requestId` is caller-chosen and unique for each logical
operation in the host's community/principal namespace. `args` has exactly the
chosen action's shape. No requester, owner, credential, relay, or scope member
is permitted. Missing versions do not mean version 1.

### Result

Common required members: `type:"broker_result"`, `protocolVersion:1`,
`requestId` copied exactly, and `status`. Optional `replayed` is boolean.

| Status | Additional required members | Promise |
|---|---|---|
| `succeeded` | `action`, `outcome` | Operation completed as defined by the action |
| `failed` | `error:{code,message}` | This submission caused no operation effects |
| `indeterminate` | `error:{code,message}` | Effects may have happened; reconciliation is required |

Success MUST NOT contain `error`; other statuses MUST NOT contain `action` or
`outcome`. `message` is an operator-facing string, not machine-readable detail;
it MUST NOT disclose keys, credentials, or decrypted application payloads.

| Error code | Meaning | Permitted status |
|---|---|---|
| `invalid_request` | Malformed envelope/arguments | failed |
| `unsupported_protocol_version` | Unsupported envelope version | failed |
| `unknown_action` | Unknown action name | failed |
| `unsupported_action_version` | Unsupported version of a known action | failed |
| `unsupported` | Known action not offered | failed |
| `unauthenticated` | Missing, invalid, expired, or revoked credential | failed |
| `unauthorized` | Session lacks permission | failed |
| `request_id_conflict` | Same retry key, different received bytes | failed |
| `action_failed` | Known domain failure with no effects | failed |
| `outcome_unknown` | Execution effects cannot be determined | indeterminate |
| `internal` | Unexpected host fault | failed or indeterminate |

If several preflight checks fail, the host MAY choose any applicable code,
but MUST NOT expose authenticated state to an unauthenticated caller.
Authentication refusal is a result when correlation is possible, not a transport
error. **A refusal of a retry does not undo or disprove earlier execution.** In
particular, conflict, revocation, or permission loss only proves this attempt
introduced no new effects. Clients with earlier uncertainty MUST retain it.

Clients MUST validate the complete shape, status/code pair, request ID, and
success action before accepting a verdict. Validate all outcome identities and
bounds, the create channel against the requested channel, and update/delete
pubkey against a pubkey selector. A name selector cannot prove identity by echo.
Read pages MUST fit the effective limit and observer receipts the submitted
frame count. A mismatch is a transport failure. Event signature verification
is separate; clients SHOULD verify NIP-01 IDs/signatures before trusting content.

## Execution and retries

Let `K = (community, authenticated principal, requestId)` and
`D = SHA-256(received HTTP body bytes)`. Community is host-derived; a shared host
MUST NOT allow one community to retrieve another's records. Credential rotation
within the same principal/community MUST retain this namespace.

1. Client validates, normalizes, and serializes **once**. Every retry MUST send
   the identical bytes and ID. Equivalent reserialized JSON is not a retry.
2. After authentication and permission checks, the host MUST atomically claim
   an unused K with D in durable storage **before any operation effect**. Only
   one executor may own K, including across replicas and restarts.
3. An existing K with a different D MUST return `request_id_conflict`, without
   overwriting the record or executing. A matching completed record MUST return
   its stored domain result, adding `replayed:true`. Delivery metadata is not
   part of the stored result. Replaying MUST NOT execute, republish, or renew an
   ephemeral signal.
4. For a matching in-flight record, the host MUST join/wait within its timeout
   or return `indeterminate/outcome_unknown`; it MUST NOT start another executor.
   A wait timeout need not finalize the record: the original executor can finish.
5. After a crash, a record whose executor may have acted MUST NOT be reclaimed
   for fresh execution. Reconcile from durable execution evidence or retain an
   indeterminate result. A lease expiring alone is not proof of non-execution.
6. Store the result durably before delivering it. `failed` requires proof that
   no operation effects occurred; partial publication/provisioning is not a
   failure with no effects. `indeterminate` may later resolve only from evidence,
   never from re-executing the requested operation.

The host MUST retain records for as long as that principal/community namespace
can submit requests. If full results are discarded, a durable tombstone MUST
retain K and D and prevent re-execution; a matching retry then returns
`indeterminate/outcome_unknown`, not fabricated success or failure. A host MAY
retire a namespace only if it permanently rejects all future submissions in it.
It MUST apply admission quotas rather than silently evict retry protection.
Clients MUST NOT reuse an ID, or switch to a new ID to retry an uncertain write.

This is **at-most-once dispatch**, not exactly-once relay delivery. Hosts MUST
fence workers and any downstream retries so one dispatch cannot itself mint
duplicate effects. Persisting an event before publication can enable replay of
that same signed event; whether a relay accepts it remains a separate question.
Reconciliation can require operator intervention; termination is not guaranteed
through a permanent partition. New reads and periodic signals use new IDs.

## Actions

`Published` means exactly `{eventId:Hex,kind:u32,createdAt:u64}`. It describes the
host-built event, not a signed receipt. Success requires positive acceptance by
the configured community relay, not merely a local send; ephemeral acceptance
does not guarantee any subscriber saw it. The host chooses timestamps, signer,
relay, and tags, except values derived from explicit arguments below.

| Action | Exact args (`?` optional) | Exact outcome |
|---|---|---|
| `channel.read` | `channelId:Channel, rootEventId?:Hex, mentionsOnly?:bool, cursor?:Cursor, limit?:u32` | `messages:Event[], nextCursor?:Cursor` |
| `message.post` | `channelId:Channel, content:string, mentions?:Pubkey[]` | Published |
| `message.reply` | post args plus `replyToEventId:Hex` | Published |
| `reaction.add` | `channelId:Channel, targetEventId:Hex, reaction:string` | Published |
| `profile.set` | `displayName?:Name, about?:about, picture?:scalar` | Published |
| `storage.address` | `slug:Slug` | `authorPubkey:Pubkey, kind:u32, dTag:Hex` |
| `storage.get` | `slug:Slug` | `value?:string` |
| `storage.put` | `slug:Slug, value:string` | Published |
| `presence.set` | `status:"online"\|"away"\|"offline"` | Published |
| `typing.set` | `channelId:Channel` | Published |
| `observer.emit` | `frames:{kind:scalar,payload:string}[]` | `accepted:u32` |
| `liveness.ping` | `channelId:Channel, turnId:scalar` | Published |
| `agents.create` | `channelId:Channel, displayName:Name, systemPrompt:prompt, runtime?:scalar, provider?:scalar, model?:scalar, respondTo?:mode` | `agentPubkey:Pubkey, displayName:Name, channelId:Channel` |
| `agents.update` | `target:Target, displayName?:Name, systemPrompt?:prompt, runtime?:scalar, provider?:scalar, model?:scalar, respondTo?:mode` | `agentPubkey:Pubkey, displayName:Name, updatedFields:string[]` |
| `agents.delete` | `target:Target` | `agentPubkey:Pubkey, displayName:Name` |

### Channel and messages

`limit` is 1–500; omission means 100. Filters intersect: channel, optional
thread root, and optional signed `p` mention of the authenticated agent.
`Event` has exactly the seven NIP-01 members `id,pubkey,created_at,kind,tags,
content,sig`; tags are arrays of string arrays, signature is 128 hex characters.
The host MUST return original signed events, not projections. All returned
messages MUST satisfy the requested channel/thread/mention filters. Ancestry
and mentions come from signed tags, not unsigned sibling metadata.

The host MUST document initial window, ordering, and cursor lifetime. A cursor
MUST bind to the principal/community and query filters and preserve continuation
without skipping same-timestamp events. Invalid, expired, or mismatched cursors
MUST fail with `invalid_request`, never silently restart. Clients MUST round-trip
cursors verbatim, never parse, synthesize, or compare them for ordering. Continue
while `nextCursor` is present, even after a short page; omission means exhausted
for that traversal, not a promise that no future message can arrive. Each page
uses a new ID. Clients SHOULD deduplicate overlapping polling windows by event
ID. This is not a subscription or a completeness proof.

Post/reply content is at most 65,536 bytes; mentions contain 1–50 pubkeys when
present. The host constructs kind 9 messages with the target channel's `h` tag
and supplied notification `p` tags. Mentions do not grant membership. For replies,
the host MUST fetch/validate the parent in that channel, derive its actual
NIP-10 root, and encode both ancestry and immediate parent consistently; it MUST
NOT silently treat a nested parent as the root. Missing, inaccessible, or
inconsistent ancestry MUST fail before publication. Reaction targets likewise
MUST belong to the channel; reaction text is trimmed, nonempty, at most 66
scalars, published as kind 7. Hosts MAY restrict reaction vocabulary further.

Profile setting publishes kind 0 for the requester, merging only supplied fields;
at least one field is required. Omitted fields remain unchanged. Empty/whitespace
values are invalid, not clear commands. Picture is a bounded string, not a
promise that its URL is safe to fetch.

### Memory

The host performs NIP-AE address derivation, validation, head selection, and
NIP-44 encryption/decryption for the session's agent/owner pair, restricted to
the session community. `storage.address` returns the agent's author pubkey,
kind 30174, and NIP-AE's HMAC-derived d-tag, never its conversation key.

`storage.get` returns `{}` for a missing head or memory tombstone. Otherwise
`value` is the memory body's `value`, or **core body's `profile` string**, not
serialized core JSON. An existing empty string remains `{"value":""}`.
Unavailable reads MUST NOT masquerade as absent memory.

`storage.put` replaces the selected record. Encode exactly
`{"slug":s,"value":v}` for memory or `{"slug":"core","profile":v}` for core.
The complete compact UTF-8 JSON plaintext, including escaping, MUST fit 65,535
bytes. This is the Buzz NIP-AE/profile cap, not a claim about the maximum of
all upstream NIP-44 formats. For size measurement use the displayed member
order, no insignificant whitespace, literal non-ASCII scalars, escapes `\"`, `\\`, `\b`, `\t`, `\n`,
`\f`, `\r`, and lowercase `\u00xx` for other U+0000–001F controls. Do not escape
`/`. Publish kind 30174 following NIP-AE's monotonic timestamp rule. This API
has no delete, empty write, or core-clear operation. Concurrent distinct-ID
writes retain NIP-AE's eventual head semantics; success is not a lasting lock
on the head. The broker's no-null rule does not forbid NIP-AE tombstones inside
host-decrypted records.

### Live signals

Reaction and all four live actions are **best effort**: `unsupported` is a
normal refusal and clients MUST be able to continue without them. Other actions
can also be refused but callers MUST surface loss of the requested capability.
Best effort does not weaken authentication, validation, or retry guarantees.

Presence publishes kind 20001 with content equal to `status`; offline clears
presence. Typing publishes kind 20002 with channel `h` tag and empty content.
There is no stop action: the indicator expires under relay policy. Each fresh
renewal is a new operation, not a replay of the preceding signal.

Observer frame `kind` is a runtime discriminator, **not a Nostr kind**. Payload
is an opaque serialized string; the host MUST NOT interpret it as routing or
authority. There are 1–256 frames; each payload is at most 65,535 bytes, and the
whole normalized `{"frames":[{"kind":k,"payload":p},...]}` is at most 65,535
bytes using the compact encoding rule above. The host encrypts accepted payloads
to the session owner using NIP-AO telemetry, deriving all Nostr metadata itself,
and MAY batch/pace publication. `accepted` is a count, 0 through input length,
not a list or prefix guarantee. It acknowledges intake for best-effort
publication, **not relay delivery**. Clients MUST NOT infer which frames were
accepted or resubmit a guessed remainder. Replaying a receipt does not enqueue
again. Drops after intake remain possible.

Liveness refers to the current session's channel and process-local `turnId`.
The host MUST reject a turn not bound to that session/channel before effects.
It publishes an owner-encrypted NIP-AO telemetry event (kind 24200) describing
the keepalive and MAY also renew that turn's stall watchdog. Watchdog renewal
and relay publication can partially complete; that requires `indeterminate`,
not `failed`. The precise runtime telemetry payload is defined by the runtime
profile, not this action interface; neither it nor observer data proves actual
computation progress.

### Managed agents

`Target` is exactly `{"pubkey":Pubkey}` or `{"name":Name}`. The host resolves
names within authorized ownership scope and MUST reject missing or ambiguous
matches, never choose an arbitrary agent. `mode` is `owner-only` or `anyone`.
Create defaults to owner-only; optional runtime/provider/model defaults are
host-declared and unknown choices MUST be refused, not silently substituted.

Create mints an identity, stores configuration and attaches it to `channelId`;
the authenticated requester is its owner. Ownership chains MUST terminate at
a human; allowed depth is host policy. No private key or credential is returned.
Success promises the managed record and attachment exist, not that the runtime
has booted. Hosts MUST document provisioning and deletion scope.

Update requires at least one mutable field. Absent fields remain unchanged;
empty strings do not clear them. `updatedFields` lists actually changed wire
names in lexicographic order without duplicates; it is empty for an idempotent
no-op patch and MUST be a subset of supplied mutable fields. Delete removes the
managed record; it does not erase historical Nostr events or imply that an
unreachable process stopped. Outcomes report the resulting (or deleted) name.
Partial provisioning/deletion MUST be indeterminate unless all effects are
proven absent. Authorization is rechecked at the operation's effect boundary.

## Security and conformance boundary

Both sides MUST implement the wire, correlation, and retry rules; hosts MAY
refuse individual capabilities. These requirements are independent of language
and of any SDK. The [model and conformance note](../formal/nip-broker/NOTE.md)
records checked properties, proposed clarifications, tests, and exclusions;
it does not replace the normative rules above.

The trusted host can impersonate its agents and suppress reads. Signatures prove
event authorship/content, not freshness, completeness, or benign intent. Memory
and observer payloads are plaintext at the broker boundary and MUST be protected
in transit, logs, and retry storage. Closed schemas remove key-export fields;
**they cannot prevent a string from containing a secret**. A compromised bearer
can exercise its permitted capabilities until revoked. Loopback alone does not
isolate same-user hostile processes. This contract is not a sandbox, a complete
information-flow policy, a cryptographic proof, or an upstream-accepted NIP.
