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
`messages:write` and are both `p`-gated and result-gated. Their plaintext
content is excluded from the global full-text-search vector.

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

## Read API

PR 1 exposes only:

```http
GET /api/buzz-tasks?status=open&bucket=all|today|later&cursor=...
GET /api/buzz-tasks/{taskId}
```

`status` supports `open`, `resolved`, `withdrawn`, and `all`; it defaults to
`open`. `today` and `later` require `tz_offset_minutes`. Results sort by
overdue, priority, due time, newest source event, and stable task ID.

Each GET requires a NIP-98 signature for the exact host, path, and raw query,
then the shared replay fence, HTTP admission limit, relay membership,
`messages:read`, exact `p`-owner match, and current channel access. Detail
misses and unauthorized identities both return not found. Responses are
`private, no-store`.

There are intentionally no reply, approve, reject, comment, resolve, or
preferences mutation endpoints in PR 1.

## Native navigation target

The projection stores only the canonical target identity:

```text
community_id + channel_id + source Nostr event ID
```

After read authorization, the API boundary validates those typed values and
constructs:

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
