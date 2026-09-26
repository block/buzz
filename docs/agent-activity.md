# Conversation agent activity v1

**Version 1 contract; implementation and runtime validation are tracked separately.**

This contract describes a compact view of an agent working on a conversation
request. Everyone authorized to read that conversation can observe it. Execution
ownership and permissions remain with the requester. Activity is transient UI
state, separate from messages, history, unread counters and notifications.

It follows the legible, honest progress intent in [VISION_ACTIVITY](../VISION_ACTIVITY.md).
Unlike the owner-only debugging surface in [NIP-AO](nips/NIP-AO.md), this surface
never includes model reasoning, tool arguments, tool results, credentials, raw
errors or backend task/session identifiers. There is no raw-detail expansion.
That narrower audience contract is deliberate.

## Transport and trust

Kind **24201**, `KIND_AGENT_ACTIVITY_SNAPSHOT`, is a Buzz-specific ephemeral
kind. It is not a globally reserved Nostr standard. No collision was found in
Buzz or the [Nostr kind registry](https://github.com/nostr-protocol/registry-of-kinds/blob/master/schema.yaml)
when checked on 2026-09-26. It is distinct from typing (20002) and encrypted
owner observability (24200).

Publish signed NIP-01 `EVENT` frames over an authenticated WebSocket, or submit
the same signed event through NIP-98-authenticated `POST /events`, as the agent's
configured signer. Process negative acknowledgements and HTTP failures.
The baseline relay rejects ephemeral HTTP submission; the accompanying generic
ephemeral HTTP path must share WebSocket authorization and fanout without entering
persistent ingest. Merely adding this kind to a persistent-kind allowlist would
violate this contract. There is no activity recovery HTTP endpoint.

The relay binds the community to the connection host, verifies the signed event
and authenticated publisher, and uses `h` for channel routing. Private-channel
delivery is limited to authorized participants. Open channels follow Buzz's
existing open-channel visibility, including nonmember access. Ephemeral events
are not stored or replayed and do not enter durable-message side effects.
An accepted publish is not proof of receipt by every observer: transient fanout
loss is recovered by the next complete publication.

Consumers MUST verify signatures and event IDs and compare the publisher with
the expected agent key from trusted, authenticated configuration for this
community. Channel membership, a profile name or a bot badge is not signer
authorization. Missing trusted identity means activity is unavailable.

## Envelope

The normal seven signed Nostr fields apply. Tags MUST contain:

| Tag | Value |
| --- | --- |
| `h` | Exactly one non-nil conversation UUID |
| `e` with marker `root` | Exactly one containing thread root event ID |
| `e` with marker `reply` | Exactly one originating request event ID |

Both `e` tags are present when root and request are the same event. They use
`["e", "<64 lowercase hex>", "", "root" or "reply"]`. No other `e` tags are
allowed in v1. The two IDs MUST agree with content. A consumer MUST establish
that the referenced signed normal request belongs to this conversation and
root before presenting the run; an unavailable request is not authority to
attach activity to another message. Resolve missing requests with bounded
conversation-authorized reads, never by dispatching work.

Publishers MUST validate the envelope before signing. Generic ephemeral events
without `h` use global routing; the shared submission path rejects malformed or
duplicate `h` tags. Activity always requires `h`: correct conversation scoping
cannot be inferred from JSON content.

Subscribe using explicit `kinds: [24201]`, `authors: [trusted_agent_key]` and
`#h: [conversation_id]`. A thread observer may also filter `#e: [root_id]`,
but MUST validate the root marker itself because `#e` matches either reference.

## Complete snapshot content

`content` is a JSON object encoded as a string in the Nostr event:

```json
{
  "version": 1,
  "request_event_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "thread_root_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "run_id": "run-01",
  "seq": 1,
  "status": "working",
  "ended_at": null,
  "tools": [
    {"tool_call_id": "call-01", "label": "search", "status": "running"}
  ],
  "omitted_tool_count": 0,
  "omitted_running_tool_count": 0
}
```

All fields above are required. `run_id` identifies one attempt at one request,
not a backend session that may serve many requests. Run IDs MUST be unique for
the publisher within a community and MUST NOT be recycled. A retry starts a
new run ID; a retried tool starts a new tool-call ID. Parallel tools have
distinct IDs. The community, conversation, publisher, request and root binding
of a run is immutable. A later normal agent reply never rekeys the run.

`seq` starts at 1 and increases for **every** publication, including unchanged
refreshes and terminal rebroadcasts. Sequence gaps are permitted. This both
orders state and ensures fresh signed event IDs even within one timestamp
second. Timestamp order is not run order.

Run status is `working`, `completed`, `failed`, `cancelled` or `unknown`. Tool
status is `running`, `completed`, `failed`, `cancelled` or `unknown`. All except
`working`/`running` are terminal. Only authoritative producer signals justify
an outcome. `unknown` means the operation ended but its outcome is unavailable.
A failed tool does not imply a failed run. A terminal run has no running tools:
unresolved tools become `unknown`, never automatically completed.

`ended_at` is null while working and a fixed integer Unix time in seconds when
terminal. It MUST be no later than the publication's `created_at`. A terminal
status, ended time and tool projection are immutable across rebroadcasts;
only `seq` and the envelope's `created_at`, `id` and `sig` change.

Labels are public semantic codes, localized by clients, not copied from
unrestricted tool names or narration. The v1 vocabulary:

| Code | Default English label |
| --- | --- |
| `tool` | Tool |
| `search` | Search |
| `read` | Read information |
| `update` | Update information |
| `send` | Send message |
| `compute` | Calculate |

Unknown tools map to `tool` at the producer. Consumers map an unrecognized
bounded label code to the same generic label. No free-form narration in v1.
Unsupported versions, unknown statuses and malformed required fields are
rejected without disturbing accepted state. Unknown additive JSON fields are
ignored after enforcing the total payload bound; they are never rendered.

## Freshness and recovery

Publish a complete snapshot promptly on change and at most every **5 seconds**
while working. Coalesce rapid updates to bound publisher load; periodic
publications carry the complete current projection, not a heartbeat alone.
Publish the terminal snapshot immediately and every 5 seconds until **60
seconds after `ended_at`**, then stop. Do not reset that deadline on retry.

After a healthy authenticated subscription is established, an active run can
be recovered from its next publication within 5 seconds plus network delivery
time. This also applies to terminal runs still in the rebroadcast window.
There is no promise of historical trace recovery after that window, and EOSE
does not prove that no run is active. Reconnect and foreground observation
resubscribe and reauthorize; neither operation redispatches a request or tool.

At receipt capture wall and monotonic clocks together. Reject events aged
**20 seconds or more**, or more than **5 seconds** in the future. Set a working
run's freshness deadline to receipt monotonic time plus
`max(0, 20 - max(0, wall_now - created_at))` seconds. Thus a 19-second-old replay
has one second left, not a renewed 20-second lease. Duplicate, out-of-order,
wrong-signer and otherwise rejected events never extend the deadline.

Expiry changes freshness to stale and stops working indicators. It does not
invent successful completion or mutate the last authoritative outcome. A fresh
higher-sequence working snapshot can recover a stale run. A terminal run can
never change its outcome or return to working, and a terminal tool ID can
never change its outcome or return to running.
New tool-call IDs distinguish retries.

Terminal rows are displayed only until `ended_at + 60 seconds`; rebroadcasts
do not prolong display. Keep sequence/binding/terminal tombstones for **120
seconds after the last accepted snapshot**, including after row removal.
Do not drop a live tombstone to admit a new run. Exhaustion becomes a visible
degraded observation state until space is available. Connection/account or
permission changes fence old callbacks by generation and clear the visible
projection. Observation revision increments for local freshness/lifecycle
changes independently of per-run producer sequence.

## Bounds and projection

- Content: at most **16 KiB UTF-8**; tags: at most 16, each value at most 128
  UTF-8 bytes. Run and tool IDs: 1–128 ASCII characters from
  `[A-Za-z0-9._:-]`. Label codes: 1–32 lowercase ASCII letters or underscores.
- Integers are nonnegative and at most `9007199254740991`; `seq` is positive.
- At most **32 tools** per snapshot, with unique IDs. Running tools come first,
  then terminal tools; each group sorts by ascending tool-call ID. Fill the
  first 32 slots. `omitted_tool_count` counts all excluded tools and
  `omitted_running_tool_count` counts the running subset; the latter cannot
  exceed the former. An omitted tool is not implicitly completed or deleted.
- Clients expose at most **32 runs** per conversation and explicitly report
  omitted runs or degraded observation. Defensive state budget:
  **128 tracked runs/tombstones per conversation**, **128 distinct tool IDs per
  tracked run**, and at most **8 concurrently observed conversations per client**.
  Thread observers of the same conversation share one conversation slot.
  These are client admission limits, not claims that the producer has stopped
  additional work. Once tracking bounds are hit, retain anti-regression fences,
  reject untrackable state and expose degradation; do not silently evict guards.
  A 129th distinct tool ID irreversibly marks tool detail capacity-exceeded for
  that run; clients can still track its run terminal outcome, but must show
  unknown/incomplete tool detail instead of claiming a complete tool trace.
- A stale working row is removed 60 seconds after its last accepted snapshot.
  Terminal rows follow `ended_at + 60` regardless of rebroadcast. In both cases
  bounded tombstones remain until the rule above permits removal.

## Authorization lifecycle

Establish authorized conversation access before presenting activity. Stop and
clear it on authoritative permission removal, logout, community change,
subscription cancellation or terminal subscription closure. Recheck permission
on reconnect. Relay-authored roster events require the trusted relay signing
key and the exact conversation `d` tag; arbitrary member-authored rosters are
not authority. A removed private-channel member cannot receive the new channel roster, and
`CLOSED` is currently guaranteed only on the pod processing the removal.
Consumers also subscribe to trusted relay-signed kind 44101 notifications with
`#p: [self]`; their `h` tag identifies the revoked conversation. Notifications
are best effort, so authenticated access refresh runs every **10 seconds** and
grants at most a **20-second monotonic access lease**. Clear visible activity
on denied access or lease exhaustion; a failed refresh never extends a lease.
If the immediate notice is lost, private access removal clears activity within
20 seconds of the last successful authorization at most. Reconnect MUST
reauthorize before display, and activity subscriptions MUST replay without an
event-derived `since` watermark so rejected future events cannot suppress
recovery. A fresh authorization can resume observation. Open-channel authorization follows
open visibility, not an unconditional roster-membership requirement.

## Fixtures and acceptance boundary

[Shared fixtures](agent-activity.fixtures.json) contain unsigned templates.
Tests sign them using their own keys and explicit clocks; signature rejection
is a separate actor-boundary test. The fixtures are producer/consumer contract
inputs, not evidence of a deployed relay or a running producer.

Required integration proof covers two authorized observers, private-channel
nonparticipant exclusion, missed updates and reconnect, no history/unread/push
effects, and overlapping requests/tools. Synthetic fixtures establish protocol
handling only. Actual producer and client acceptance remains separate.
