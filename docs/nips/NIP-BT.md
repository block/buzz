# NIP-BT: Buzz Tasks v1

`draft` `optional`

Buzz Tasks is an owner-private, read-only inbox projection over signed Nostr
events. It does not own replies, choices, approvals, comments, or resolution.
Those actions remain in the referenced Buzz message thread.

## Kinds

| Kind | Contract | Purpose |
| ---: | --- | --- |
| 44300 | `buzz.task.requested.v1` | Create an open task |
| 44301 | `buzz.task.updated.v1` | Update mutable fields of an open task |
| 44302 | `buzz.task.resolved.v1` | Mark a task `resolved` or `withdrawn` |

All three kinds are regular persistent, channel-scoped events. They require
`messages:write` and are both `p`-gated and result-gated. The global search
query unconditionally excludes their plaintext content; fresh databases also
exclude it from the generated full-text-search vector.

## Signed identity envelope

Every task event has exactly one of each identity tag:

```json
[
  ["d", "<task UUID>"],
  ["p", "<owner pubkey hex>"],
  ["agent", "<agent pubkey hex>"],
  ["h", "<channel UUID>"],
  ["e", "<source Nostr event ID hex>", "", "source"]
]
```

The `agent` value MUST equal the event author. The relay MUST resolve the
community from the request host; community identity is never accepted from
event content or a navigation URL. Before persistence, the relay also checks:

- the authenticated signer equals the event author;
- the signer has `messages:write`;
- both agent and owner are direct members of the signed channel;
- the `p` pubkey is the registered owner of the authoring agent;
- the source event exists in the resolved community and signed channel; and
- the source event was authored by the same agent.

The last source check and the projection transition commit in the same database
transaction as the signed task event.

## Payloads

Content is UTF-8 JSON using camelCase field names. Unknown fields are rejected.

Kinds 44300 and 44301:

```json
{
  "taskType": "reply|approval|choice|review",
  "title": "Short owner-facing action",
  "context": "Optional short context",
  "priority": "low|medium|high",
  "dueAt": "2026-08-13T10:00:00Z",
  "agentName": "Display-name snapshot",
  "sourceVersion": 1,
  "sourceUpdatedAt": "2026-08-13T08:18:00Z"
}
```

`dueAt` and `context` may be `null`. Requested events require
`sourceVersion = 1`; updated events require `sourceVersion >= 2`.

Kind 44302:

```json
{
  "resolution": "resolved|withdrawn",
  "sourceVersion": 2,
  "sourceUpdatedAt": "2026-08-13T09:00:00Z"
}
```

Resolved events require `sourceVersion >= 2`. Title is limited to 200 UTF-8
bytes, context to 500, and agent name to 100. Display text must be non-empty,
trimmed, and free of control characters; context alone may contain newlines.

## Projection and replay rules

The signed event stream is the source of truth. `buzz_tasks` is a rebuildable
read projection keyed by `(community_id, task UUID)` with an additional unique
source identity:

```text
(community_id, channel_id, source_event_id, assignee_pubkey)
```

Exact signed-event replay is a no-op. A transition with a lower or equal
`sourceVersion` is stored for audit but does not change the projection. A newer
version may update only an open task and must not move `sourceUpdatedAt`
backward. `resolved` and `withdrawn` are terminal; later transitions are
rejected and their event insert is rolled back.

A kind 5 or kind 9005 deletion of a task event tombstones that event and
rebuilds the projection from the remaining live signed stream in one
transaction. Deleting a terminal event reopens the last live version, deleting
an update restores the preceding version, and deleting the sole request removes
the projection. A missing/already-deleted target is idempotent.

## Nostr read surface

PR 1 adds no task-specific HTTP endpoint. Clients read signed task events using
the relay's existing Nostr query contract. The authenticated bridge form is:

```http
POST /query
Authorization: Nostr <NIP-98 event>
Content-Type: application/json

[
  {
    "kinds": [44300, 44301, 44302],
    "#p": ["<authenticated owner pubkey>"],
    "#h": ["<accessible channel UUID>"],
    "limit": 50
  },
  {
    "kinds": [5, 9005],
    "#h": ["<accessible channel UUID>"],
    "limit": 50
  }
]
```

Clients fold task events by `sourceVersion` using the replay rules above. The
deletion kinds use a separate filter because standard NIP-09 deletions can be
channel-less (kind 5 derives channel access from its target) and are not
`p`-tagged task payloads. A client applies a deletion only to an `e`-tag target
that is one of the task events in its local owner fold; unrelated channel
deletions are ignored. The relay's normal NIP-42/NIP-98 authentication, target
channel membership, and per-result gates still apply. A deletion event is only
a live invalidation signal, never a task-content or access grant. A full
historical read remains the recovery path after a missed live deletion.
To continue a bridge page, echo the last event's complete deterministic cursor:
`until = created_at` and `before_id = event id`. Results are ordered by
`(created_at DESC, id ASC)` and the keyset predicate is
`created_at < until OR (created_at = until AND id > before_id)`. Both cursor
fields are required together. Events created, updated, or resolved above that
cursor belong to the live/head side and cannot duplicate or displace the
remaining historical page.

The bridge verifies NIP-98 against the exact host, `/query` path, HTTP method,
and body, then applies the shared replay fence, admission limit, relay
membership, exact `#p = authenticated owner`, current channel access, and a
per-result owner gate. The WebSocket REQ form uses the same standard Nostr
filter and NIP-42 identity, but the bridge is the pagination contract because
standard NIP-01 filters do not carry `before_id`.

There are intentionally no task-specific list/detail, reply, approve, reject,
comment, resolve, preferences, or UI endpoints in PR 1.

## Native navigation target

The projection stores only the canonical target identity:

```text
community_id + channel_id + source Nostr event ID
```

After read authorization, the client validates the signed `h` and source `e`
tags and constructs:

```text
buzz://message?channel=<channel UUID>&id=<source event ID hex>
```

The URL is navigation metadata only. It never proves community, identity,
membership, or source-message access; opening it must pass the normal Buzz
authorization gates.

An HTTPS/universal-link form is explicitly outside PR 1. Its host binding,
redirect behavior, authentication handoff, fallback behavior, and
cross-community resistance require a separate security-reviewed delivery
before PR 2 may depend on it.
