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
| `title` | Nonblank display title, at most 512 UTF-8 bytes. |
| `op` | `create`, `update`, `move`, or `delete`. |
| `root` | Optional conversation anchor event ID within `h`. |
| `prev` | Previous accepted revision ID; required except on creation. |

Each listed tag has exactly two string elements and occurs once, except optional tags may be absent. UUIDs are lowercase, hyphenated, and non-nil; event IDs are 64 lowercase hex characters. Type names are dot-separated lowercase ASCII components, each starting with a letter and otherwise using letters, digits, `_`, or `-`, up to 128 bytes total. `buzz.*` is reserved for published Buzz client contracts.

Relays MUST reject invalid envelopes, unsupported envelope versions, and events with more than 256 tags. Normal event-size and write quotas apply.

Clients place fields needed for filtering in client-defined tags. The relay matches tag names and values without interpreting their meaning; `content` remains opaque text. Client-defined tags cannot override the envelope or grant access.

## Identity and edits

An artifact is identified by `(community, d)`, independently of its author or home. `d` and `type` are immutable. Kind 45010 is a regular stored event, not a NIP-01 addressable event.

Creation requires an unused `d`, `op=create`, and no `prev`. Every later revision MUST name the current accepted event in `prev`. The relay checks authorization and advances the current revision atomically: two competing edits cannot both succeed. Timestamps do not choose the winner. Retrying an accepted event is idempotent.

On conflict, clients fetch the current accessible revision and reconcile their changes rather than blindly retrying. An `update` keeps `h`; a `move` changes it.

When supplied or changed, `root` MUST identify an existing, undeleted conversation anchor in `h`. Later loss of that anchor does not prevent edits. Replies belong to that conversation, not to an artifact revision.

## Access

Channel read permission governs every artifact read, including lookups, history, search, previews, counts, and live updates. Threads inherit their channel's audience. Membership and visibility changes take effect on subsequent reads and deliveries.

Write permission means permission to post a kind-9 message in the channel, including authentication, token restrictions, moderation, and archive checks. This includes agents and, where channel policy permits, nonmembers of open channels. Artifact writes use `channels:write`.

| Operation | Required permission at commit time |
| --- | --- |
| Create, update, or delete | Write in the home channel. |
| Move | Write in both source and destination. |

These rules apply to every type and channel, including DMs, regardless of who created the artifact.

Relationship tags organize work without changing access. Linking a task to a project does not move it or share it. References to repositories or other services retain those services' access and action permissions. Errors MUST NOT disclose inaccessible artifact details.

## Moving and deleting

A move publishes the current snapshot into the destination under the same `d`. Its `root` must be absent or belong to the destination. Clients MUST show the destination audience and the information being shared before confirmation.

The relay MUST atomically advance the current revision and durably record both arrival in the destination and removal from the source. The source receives a separate relay-authenticated removal identifying the artifact, source scope, and replay position, without destination metadata or content. Both changes must survive disconnects and crashes. Relays unable to provide this MUST reject moves.

Earlier revisions remain under their original channels' access rules. The destination can load current state without reading earlier revisions. Conversation messages stay where they are.

Deletion is a final revision with `op=delete` and empty content. It preserves the current `type`, `h`, `root`, and title, and contains only envelope and validated authentication tags. It removes the artifact from active views; the identity cannot be reused. Relays MUST reject kind-5 deletion requests targeting artifact revisions with an explicit reason. Completing or archiving work is a content change, not deletion or channel archival.

## Current state and queries

An artifact's current state is its latest accepted revision, as determined by the `prev` rules above. Current-state queries MUST return only that revision and omit deleted artifacts. For example, a task whose current revision belongs to Project B no longer appears in Project A's list, even if an earlier revision belonged there. Revision history is queried separately.

Relays MUST support querying current artifacts across channels the reader can access using exact tag-name/value matches, including multi-character tag names. Conditions on different tag names are combined with AND; requested values for one name are combined with OR. A match compares the tag name and its first value in the same tag. Access checks and tag matching apply before pagination and any counts. Older revisions do not contribute matches.

Relays MUST advertise and enforce limits on tag-name/value sizes, total tag bytes, query predicates and values, and page size. Queries MUST run within an enforced execution budget. Unsupported or over-limit requests fail explicitly; predicates MUST NOT be silently ignored or incomplete results presented as complete.

Artifact revisions are state changes, not conversation messages. Receiving one MUST NOT increase chat or reply counts, add a chat unread, or mark a conversation read.

## Client behavior

Clients validate content before interpreting it as a known type. Unknown types or malformed content render a safe fallback using the title, type, and home link. Editors MUST preserve unfamiliar fields and annotation tags or decline the edit. References may be unavailable or cyclic; clients must handle them safely without assuming relay validation.

Artifact text is untrusted. A type name, relationship tag, or payload cannot authorize a privileged action. Moving, deleting, or revoking access cannot erase copies already downloaded.
