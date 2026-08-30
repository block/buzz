# NIP-WR: Thread-Rooted Work Reports

`kind:40009` is a signed, channel-scoped outcome report for one Buzz thread.
It keeps the full conversation as evidence while giving clients a compact,
machine-readable result to present first.

## Event

```json
{
  "kind": 40009,
  "tags": [
    ["h", "<channel-uuid>"],
    ["e", "<thread-root-event-id>", "", "root"],
    ["t", "work-report"],
    ["status", "completed"],
    ["prior", "<previous-work-report-event-id>"]
  ],
  "content": "{...}"
}
```

The `prior` tag is omitted for the first report and required by the CLI when a
head already exists. Clients reduce a thread to the newest `(created_at, id)`
valid report and use `prior` to detect stale updates. The source events remain
immutable and available as the revision history.

## Content

Content is a JSON object:

```json
{
  "status": "completed",
  "outcome": "Shipped the result-first report contract.",
  "deliverables": ["https://example.com/pr/1"],
  "decisions": ["Keep raw conversation as evidence."],
  "verification": ["CI passed at abc1234."],
  "risks": [],
  "next_actions": ["Maintainer: review the PR."]
}
```

`status` is one of `completed`, `in_review`, `needs_decision`, `blocked`, or
`failed`. `outcome` is required and limited to 1024 bytes. The remaining arrays
are optional; each accepts at most 20 non-empty strings of at most 2048 bytes.
The complete serialized content is limited to 32 KiB.

The `status` tag must exactly match the JSON status. This permits clients to
filter attention states without parsing content while preventing two competing
representations of the result.

## Authorization and privacy

Work reports require the same `MessagesWrite` scope and channel membership as
other channel content. The relay requires a valid `h` tag; reports in private
channels are readable only through the existing channel boundary. Reports are
not an automatic LLM summary: the event signature identifies who asserted the
result, and the root reference preserves access to the source conversation.

## Presentation

Clients should present the latest valid work report before the transcript and
offer the underlying conversation and execution log through progressive
disclosure. They must not delete or rewrite source messages when a report is
published.
