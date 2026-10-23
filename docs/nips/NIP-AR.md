NIP-AR
======

Channel Artifacts
-----------------

`draft` `optional` `relay`

Defines `kind:45010` for editable records called **artifacts**. Each artifact has one home channel and may be attached to a thread. Its home determines who can read it. Any number of artifacts, including of the same type, may share a channel or thread.

The relay manages identity, access, and revisions. Clients define the content types, such as `buzz.task` or `buzz.project`.

## Event format

Each event contains a complete snapshot. For example, a task created directly in a channel (standard signing fields omitted):

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

An artifact is identified by `(community, d)`, independently of its author or home. `d` and `type` are immutable. Kind 45010 is a regular stored event, not a NIP-01 addressable event. It falls in Buzz's forum and social kind range (45000–45999) but is not a forum post and follows none of the forum rules.

Creation requires an unused `d`, `op=create`, and no `prev`. Every later revision MUST name the current accepted event in `prev`. The relay checks authorization and advances the current revision atomically: two competing edits cannot both succeed. Timestamps do not choose the winner.

Resubmitting an already accepted event succeeds without applying it again, even if a later revision has since become current, so a client that lost an acknowledgement never sees a false conflict. The response reveals nothing the sender can no longer read. A different event whose `prev` is not the current revision is a conflict. On conflict, clients fetch the current revision and reconcile their changes rather than resubmitting with only `prev` replaced, which would overwrite the competing edit. An `update` keeps `h`; a `move` changes it.

A `create` whose `d` is already in use fails. When the sender cannot read the existing artifact, the error reveals only that the identity is taken, never its home, type, title, or current revision. Clients MAY derive `d` deterministically, for example from the message an artifact tracks, and accept that disclosure.

On creation or when changed from the previous accepted revision, `root` MUST identify an existing, undeleted conversation anchor in `h`. Later loss of that anchor does not prevent edits. Replies belong to that conversation, not to an artifact revision.

## Access

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

An artifact's current state is its latest accepted revision, as determined by the `prev` rules above. Current-state queries MUST return only that revision and omit deleted artifacts. For example, a task whose current revision belongs to Project B no longer appears in Project A's list, even if an earlier revision belonged there. Revision history is queried separately. This NIP does not define a public acceptance order or history pagination; relays record the order in which they accept revisions, and a later sync specification defines how clients observe it.

Relays MUST support querying current artifacts across channels the reader can access using exact tag-name/value matches, including multi-character tag names. Conditions on different tag names are combined with AND; requested values for one name are combined with OR. A match compares the tag name and its first value in the same tag. Access checks and tag matching apply before pagination and any counts. Older revisions do not contribute matches.

Relays MUST advertise and enforce limits on tag-name/value sizes, total tag bytes, query predicates and values, and page size. Queries MUST run within an enforced execution budget. Unsupported or over-limit requests fail explicitly; predicates MUST NOT be silently ignored or incomplete results presented as complete.

Artifact revisions are state changes, not conversation messages. Receiving one MUST NOT increase chat or reply counts, add a chat unread, mark a conversation read, or repeat mention or push notifications for tags carried over from the previous revision.

## Client behavior

Clients validate content before interpreting it as a known type. Unknown types or malformed content render a safe fallback using the title, type, and home link. Editors MUST preserve unfamiliar fields and annotation tags or decline the edit. References may be unavailable or cyclic; clients must handle them safely without assuming relay validation.

Artifact text is untrusted. A type name, relationship tag, or payload cannot authorize a privileged action. Moving, deleting, or revoking access cannot erase copies already downloaded.

## Incremental reads

Incremental reads maintain an eventually consistent local store. They do not prove a complete set at one database snapshot. Absence from a page never proves deletion or loss of access.

This HTTP protocol uses `artifacts.incremental`. The WebSocket Sync subscription uses a separate `artifacts.sync` capability and defines its own complete-set guarantee. Clients select each protocol through its own capability.

The relay advertises `artifacts.incremental` when incremental reads are enabled.
Each signed `POST /query` request contains one filter. The query uses the
same community, relay admission, channel access, and exact tag rules as an
ordinary artifact read. The existing `current` and `history` array responses
keep their format.

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

Only matching readable artifacts in permanent channels are supported.
A matching TTL artifact returns HTTP 422 `incremental_unsupported_scope`. An
unrelated TTL channel does not block incremental task reads. HTTP 400 `incremental_invalid`
rejects invalid requests or tokens. HTTP 503 `incremental_unavailable` reports a
service or work-budget failure. These failures do not advance progress.

The rollout gate is `BUZZ_ARTIFACT_INCREMENTAL_ENABLED`, off by default. Follow
[artifact-sync-deployment.md](../artifact-sync-deployment.md) before enabling
it. Incremental completion does not establish a live subscription to every channel.
