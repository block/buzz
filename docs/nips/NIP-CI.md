NIP-CI
======

Conversation Import
-------------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-10 (threads), NIP-11 (relay information document), NIP-25 (reactions), NIP-29 (relay-based groups)

## Abstract

This NIP lets the people in a conversation on another platform bring its history into a Buzz channel or DM, with the original timestamps, under explicit consent from every participant. Each participant signs and publishes their own past messages and reactions as ordinary `kind:9` and `kind:7` events carrying an `import` tag. The relay accepts those backdated events only inside a consented, time-bounded import session, and tracks the session in a relay-signed snapshot.

The protocol has four user-signed control kinds and one relay-signed snapshot kind:

- `kind:42110` Import Proposal, `kind:42111` Import Consent, `kind:42112` Import Withdraw, `kind:42113` Import Complete;
- `kind:30623` Import Session Snapshot (relay-signed, parameterized replaceable).

No new content kinds are defined. Imported history uses the existing `kind:9` message and `kind:7` reaction kinds.

## Motivation

Moving a team to Buzz strands its history elsewhere. A relay-side bulk import would let one party publish words under other people's keys, and would need the relay to trust an external export. Neither fits Buzz: every event is signed by its author.

NIP-CI keeps authorship intact. Each participant signs only their own messages, so an imported message carries the same proof of authorship as a live one. The relay's job is narrow: it relaxes its timestamp drift check for a bounded set of events, and only while everyone involved has agreed to the exact corpus being imported.

## Non-Goals

This NIP does not define how a client reads the source platform, maps source users to Buzz pubkeys, or computes the manifest. Those are client concerns; the manifest hash only lets every participant confirm they consented to the same corpus.

This NIP does not import edits or deletions as history. A message is imported with its final text; a later delete of an imported message is a normal `kind:5` or `kind:9005` at the current time.

This NIP does not let anyone publish on another participant's behalf. A participant who never consents is never imported, and their messages are not published by anyone else.

This NIP does not change processing of events without an `import` tag. An active session changes nothing for normal traffic in its channel.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The relay signing pubkey advertised in its NIP-11 `self` field.
- **proposal**: A `kind:42110` event. Its event id is the **proposal id**, which names the session everywhere else.
- **participant**: A pubkey listed in the proposal. The proposer is normally a participant.
- **session**: The relay's state for one proposal: status, consents, declared and published counts.
- **imported event**: A `kind:9` or `kind:7` carrying an `import` tag.
- **admin consent**: A `kind:42111` whose `source_user` is the literal string `"admin"`, signed by a current owner or admin of the channel.

## Kinds

| Kind | Name | Signer | Storage | Purpose |
|------|------|--------|---------|---------|
| `42110` | Import Proposal | proposer | regular, channel `h` | Declares source, range, participants, counts and manifest hash |
| `42111` | Import Consent | participant or channel admin | regular, channel `h` | Accept or decline, bound to the proposal id and manifest hash |
| `42112` | Import Withdraw | proposer or participant | regular, channel `h` | Ends the session |
| `42113` | Import Complete | participant | regular, channel `h` | The participant finished publishing |
| `30623` | Import Session Snapshot | relay | parameterized replaceable, `d` = proposal id | Authoritative session state |

`kind:30623` is relay-only: the relay rejects it from clients.

## Event Formats

### `kind:42110` Import Proposal

```jsonc
{
  "kind": 42110,
  "tags": [
    ["h", "<channel id>"],
    ["p", "<participant pubkey>"],      // one per participant, sorted ascending
    ["manifest", "<sha256 hex>"],
    ["import-source", "<source type>"]
  ],
  "content": "{\"v\":1,\"source\":{...},\"range\":{...},\"participants\":[...],\"excluded_authors\":0,\"manifest\":\"...\",\"expires_at\":0}"
}
```

Content, with keys in this order:

```jsonc
{
  "v": 1,
  "source": {
    "type": "<source type>",               // [a-z0-9_-]{1,32}, for example "chat-export"
    "conversation": "<sha256 hex of the source conversation id>",
    "kind": "dm" | "group_dm" | "private_channel" | "public_channel",
    "label": "<display label, at most 80 characters>"
  },
  "range": { "from": <unix seconds>, "to": <unix seconds> },
  "participants": [
    { "pubkey": "<hex>", "source_user": "<sha256 hex>", "messages": <int>, "reactions": <int> }
  ],
  "excluded_authors": <int>,
  "manifest": "<sha256 hex>",
  "expires_at": <unix seconds>
}
```

The relay rejects unknown keys. The content is at most 16 KiB. The `p` tags MUST equal the participant pubkeys sorted ascending, the single `manifest` tag MUST equal `content.manifest`, and the single `import-source` tag MUST equal `source.type`.

### `kind:42111` Import Consent

Tags: `["e", "<proposal id>"]`, `["h", "<channel id>"]`, `["manifest", "<sha256 hex>"]`, `["p", "<proposer pubkey>"]`.

Content: `{"v":1,"decision":"accept"|"decline","source_user":"<sha256 hex>"}`. A participant's `source_user` MUST equal their entry in the proposal. Admin consent uses `"source_user":"admin"`.

### `kind:42112` Import Withdraw

Tags: `["e", "<proposal id>"]`, `["h", "<channel id>"]`. Content is empty or `{"v":1,"reason":"<at most 200 characters>"}`.

### `kind:42113` Import Complete

Tags: `["e", "<proposal id>"]`, `["h", "<channel id>"]`. Content: `{"v":1,"published":{"messages":<int>,"reactions":<int>}}`. The counts are informational; the relay's own counters are authoritative.

### Imported `kind:9` and `kind:7`

An imported message carries, in this order: `h`, the NIP-10 `e` root and reply tags when it is a reply, one `p` per mentioned participant (sorted ascending), `["import", "<proposal id>"]`, `["source", "<source type>", "<source message ref>"]`, then optional `["source-edited", "1"]`, `["source-flag", "broadcast"]` and attachment tags. `created_at` is the source timestamp in whole seconds.

An imported reaction carries, in this order: `h`, `["e", "<target id>"]`, `["import", "<proposal id>"]` and `["source", "<source type>", "<target source ref>"]`, with no `p` tag. Its target MUST itself be an imported event of the same proposal.

Clients compute every imported event deterministically (fixed tag order and content normalization), so a participant can reference a partner's message id before the partner publishes it.

### `kind:30623` Import Session Snapshot

```jsonc
{
  "kind": 30623,
  "pubkey": "<relay identity>",
  "tags": [
    ["d", "<proposal id>"],
    ["e", "<proposal id>"],
    ["h", "<channel id>"],
    ["p", "<participant pubkey>"],      // one per participant, sorted ascending
    ["accepts_after", "<unix seconds>"] // present once status is active
  ],
  "content": "{\"v\":1,\"status\":\"active\",\"required\":[...],\"consents\":{...},\"published\":{...},\"completed\":[...],\"expires_at\":0}"
}
```

Content:

```jsonc
{
  "v": 1,
  "status": "proposed" | "active" | "declined" | "withdrawn" | "expired" | "completed",
  "required": ["<pubkey>", ..., "admin"],     // "admin" only when admin consent is required
  "consents": { "<pubkey>": "accept" | "decline", "admin": "accept" | "decline" },
  "published": { "<pubkey>": { "messages": <int>, "reactions": <int> } },
  "completed": ["<pubkey>", ...],
  "expires_at": <unix seconds>
}
```

`created_at` strictly increases across republications of one session, so the newest snapshot always replaces the previous one.

Once the status is `active`, the snapshot carries `["accepts_after", "<unix seconds>"]`: the earliest time the relay accepts imported events for the session. The relay computes it once, at activation, as the activation time plus its read-replica retention plus a margin (in this implementation, 60 seconds of ring retention plus a 15 second margin, 75 seconds in total), stores it, and repeats the same value in every later snapshot. It stays on terminal snapshots of a session that was active.

## Relay Processing Algorithm

### Control events

Control events pass the normal channel write gates (token scope, membership, posting role, archived channel), except that `kind:42113` skips the membership gate so a participant who has left the channel can still finish. Each control event carries exactly the tags listed for its kind; any other tag is rejected with `invalid: import event malformed: unexpected tag <name>`. Then:

1. **Proposal.** The relay validates the content and tags above, requires `946684800 <= range.from < range.to <= now` (the range starts no earlier than 2000-01-01), 1 to 64 distinct participants, and caps `expires_at` at now plus `BUZZ_IMPORT_SESSION_MAX_DAYS` (default 30). The proposer and every participant MUST be active members of the channel, otherwise `restricted: import proposer not an active channel member` or `invalid: import participant not an active channel member`. In a DM the participant set MUST equal the DM member set. Admin consent is required when the channel is not a DM, has at least one owner or admin, and the proposer is neither. The proposer's consent is recorded as accept. Declared counts are capped per participant (`BUZZ_IMPORT_MAX_EVENTS_PER_PARTICIPANT`, default 50000, messages plus reactions) and per session (`BUZZ_IMPORT_MAX_EVENTS_PER_SESSION`, default 200000), rejected with `invalid: import counts exceed the relay limit`. Unended sessions (status `proposed` or `active` and not past `expires_at`) are capped per channel (`BUZZ_IMPORT_MAX_ACTIVE_SESSIONS_PER_CHANNEL`, default 2), rejected with `restricted: too many active import sessions in this channel`, and per proposer within the community (`BUZZ_IMPORT_MAX_ACTIVE_SESSIONS_PER_PROPOSER`, default 8), rejected with `restricted: too many active import sessions for this proposer`. Both caps are checked under per-channel and per-proposer locks, so concurrent proposals cannot exceed them. Imported history never reaches agents (see [Agents](#agents)): a participant that is an agent is rejected with `restricted: import participants cannot include agents`, and a DM or private channel with an agent member with `restricted: import destination has agent members`.
2. **Consent.** The `manifest` tag MUST equal the proposal's manifest and the single `p` tag MUST name the proposer, otherwise `invalid: import consent does not match proposal`. An admin consent MUST come from a current owner or admin. Otherwise the author MUST be a participant whose `source_user` matches. An author who is neither is rejected with `restricted: import author not a participant`. A later consent replaces the earlier one. Consents are refused once the session is terminal.
3. **Withdraw.** The author MUST be the proposer or a participant, otherwise `restricted: import author not a participant`. The session becomes `withdrawn`.
4. **Complete.** The author MUST be an accepting participant of an active session. An author who is not a participant is rejected with `restricted: import author not a participant`.

The control event, the session change and a fresh snapshot commit in one transaction. A resend of a stored control event is answered `duplicate:` before the session caps and any lazy status change. A failed session write is answered `error: import session storage failed` and a failed session read `error: import session read failed`; both are retryable and carry no storage detail. Control events are live activity and create mention rows for their `p` tags like other channel events. A proposal (kind 42110) also appears in the mentions feed of each participant it tags; the other control kinds do not.

### Session status

Terminal statuses (`declined`, `withdrawn`, `expired`, `completed`) never change. A non-terminal session past `expires_at` becomes `expired`, detected lazily on the next write that touches the session. A required admin decline makes the session `declined` before activation and `withdrawn` after it. Any participant decline makes a DM session `declined`, and makes a channel session `declined` before activation. The session becomes `active` once every participant and any required admin have accepted. After activation, a channel participant who declines stops being accepted; the session becomes `withdrawn` when no accepting participant remains and `completed` when every accepting participant has sent `kind:42113`.

### Imported events

For any event carrying an `import` tag, every rule MUST hold, otherwise the relay rejects it with the listed reason:

| Rule | Check | Rejection |
|------|-------|-----------|
| 1 | Kind is 9 or 7 | `invalid: import kind not allowed` |
| 1 | Exactly one well-formed `import` tag and one well-formed `source` tag | `invalid: import tags malformed` |
| 2 | The proposal exists in this community | `invalid: import proposal not found` |
| 2 | The event's `h`, the resolved channel and the session channel agree | `invalid: import channel mismatch` |
| 2 | `source[1]` equals the proposal's `source.type` | `invalid: import source type does not match the session` |
| 2 | A DM's members still equal the participants | `invalid: import participants must equal the DM members` |
| 2 | A DM or private destination still has no agent member | `restricted: import destination has agent members` |
| 3 | The session is `active` and the author has not completed | `invalid: import session not active` |
| 3 | The current time is at or after the session's `accepts_after` | `invalid: import session warming` |
| 4 | The author is a participant whose consent is accept | `invalid: import author not a consenting participant` |
| 4 | The author is not an agent | `restricted: import participants cannot include agents` |
| 5 | `range.from <= created_at <= range.to` and `created_at <= now + 900` | `invalid: import timestamp outside declared range` |
| 6 | The author's accepted count for the kind is below the declared count | `invalid: import count exceeded` |
| 7 | Reply parent and root exist in the channel (existing NIP-10 rule) | existing thread rejection |
| 7 | A reaction target carries `["import", <same proposal id>]` | `invalid: import reaction target not imported` |
| 7 | The same actor has not already reacted to the target with the same emoji | `invalid: import reaction conflicts with an existing reaction` |
| 9 | `BUZZ_RATE_LIMIT_IMPORT_EVENTS_PER_MIN` per pubkey (default 600) | `rate-limited: import` |

`invalid: import session warming` is retryable: the client waits until the snapshot's `accepts_after` and publishes again. Rule 4's agent check is repeated for every imported event, so a participant who becomes an agent after the proposal cannot import.

Rule 8: all other existing checks (content size, media, link previews, membership) apply unchanged. The `MAX_TIMESTAMP_DRIFT_SECS` check is skipped for an event with a well-formed `import` tag; the event is then stored only if rules 2 to 7 hold in the storage transaction, so a backdated event is never stored outside a consented session.

Rules 2 to 7 are evaluated under the session row lock, in the same transaction that stores the event and increments the author's counter, so concurrent publishes cannot exceed a declared count. Imported events are charged to the import budget instead of the messages budget; the WebSocket per-second event limit still applies.

Accepted imported events are stored, indexed for search, audited and fanned out live to channel subscribers. The relay does not write mention rows or notification outbox entries for them, does not enqueue push, does not count them as unread evidence, does not move a thread's `last_reply_at` forward to the import time, and does not trigger workflows or agent subscriptions. The relay does not fan imported events out to connections authenticated as agents, imported events do not refresh an ephemeral channel's lifetime, and the agent harness drops them from the thread and DM context it gives agents.

The relay republishes the snapshot after every accepted control event, after a lazy expiry, and each time a participant's combined accepted count reaches a multiple of 50. A count republish is skipped while the newest snapshot's `created_at` is not yet behind the relay clock, so strictly increasing `created_at` values cannot run ahead of real time when many participants import at once; the next control event republishes the current counts.

### Agents

An agent is a pubkey whose user row in the community names an owner (`users.agent_owner_pubkey IS NOT NULL`). Every agent rule in this NIP uses that one definition. Three cases look human to the relay and are treated as human:

- an agent that was never claimed by an owner;
- an agent whose owner's user row was deleted, which clears the owner link;
- an agent process that signs with a human's key.

Operators who need imported history kept from such keys SHOULD claim the agent or remove it from the destination.

Imported events are hidden from agent readers on every read path, using the same predicate as live fan-out: WebSocket `REQ` history, NIP-50 search, and HTTP `/query` in every mode, which covers `buzz-cli` and MCP tools that read through those paths. An agent's `COUNT` still includes imported events; it reveals how many exist, not their content. Thread metadata can likewise reveal that imported events exist: thread-summary overlays (`kind:39005`), thread roots and the `next_cursor` of a `kind:39006` page can expose an imported event's id, `created_at`, reply count and participant pubkeys to an agent reader, never its content.

Imported events are removed after a page is read, so a `REQ` or `/query` page made up only of imported events returns empty to an agent reader. An agent paging backwards with `until` SHOULD continue past an empty page rather than treat it as the end of history.

### Read replicas

A relay that serves channel reads from a replica MUST NOT let a replica snapshot that predates an imported event answer for that event's channel. This implementation routes a covered read to the writer while the channel has an active session, and keeps doing so until `expires_at` plus the same delay as `accepts_after`. `accepts_after` itself is what makes that check sound: imports start only after every replica snapshot the relay trusts already contains the activation. A thread read for a root the relay has not stored stays on the replica. Any active channel member can propose, so one member can hold up to the per-proposer cap of unended sessions (`BUZZ_IMPORT_MAX_ACTIVE_SESSIONS_PER_PROPOSER`) for the session lifetime, and once those sessions are active they pin each of those channels' covered reads to the writer.

## Client Behavior

1. The proposer computes the corpus and manifest, then publishes `kind:42110`.
2. Every other participant reviews the proposal, recomputes the manifest from their own view of the source, and publishes `kind:42111`. A client MUST NOT consent when its manifest differs.
3. Once the snapshot shows `active` and the current time has reached its `accepts_after` tag, each participant publishes only their own imported events in ascending `(created_at, source message id)` order. A reply whose parent belongs to another participant waits until the parent exists.
4. Each participant publishes `kind:42113` when done.

Clients SHOULD render imported events with a marker naming the source and noting the original time, and MUST exclude them from unread, inbox, badge and notification computations.

Clients read session state with `kinds: [30623]`, `#d: [<proposal id>]` and treat the newest snapshot signed by the relay identity as authoritative.

## Implementation Gotchas

- The drift bypass is keyed on the `import` tag alone; the session checks run later. A relay MUST NOT store an imported event before every rule holds.
- Reactions derive their channel from the target event, so rule 2 compares the event's `h` against the target's channel as well as the session's.
- Imported reactions are rows in the reactions table like any other, so a normal un-react later works unchanged.
- A duplicate imported event (same id) is reported as `duplicate:` and does not increment the counter, so clients can retry safely.
- An import published between activation and `accepts_after` is refused with `invalid: import session warming`; clients SHOULD read `accepts_after` from the snapshot instead of retrying blindly.
- The session caps count sessions that have not ended. A session past `expires_at` stops counting even before its lazy expiry is stored.
- Expiry is applied lazily. A session past `expires_at` keeps its last stored status until the next control event or imported event touches it; clients SHOULD compare `expires_at` with the current time themselves.

## Security and Privacy Considerations

Authorship is never delegated: every imported event is signed by its author, and the relay accepts it only from a participant who consented to the manifest. A proposal cannot enrol a pubkey that is not already an active channel member, and in a DM it cannot import into a conversation whose members differ from the participants.

The manifest binds consent to a specific corpus. A participant who consents to one manifest has not consented to any other, and the relay refuses a consent whose manifest differs from the proposal. Consent binds the manifest declaration (counts, range and hash), not message content: the relay enforces the declared counts and range but cannot check that each published event belongs to the corpus the manifest hashes.

Source identifiers are hashed (`source.conversation`, `source_user`) so the proposal does not reveal raw identifiers from the source platform.

The snapshot is stored in the session's channel and is readable through the same channel read gate as the channel's messages. It reveals participant pubkeys, consent decisions and counts, which channel readers can already infer from the control events.

Attachment blobs on imported events are stored like any other attachment, and the media read gate is community membership, not channel membership.

Client consent dialogs guard against buggy or accidental client-side code, not hostile code running in the same origin. A client whose plugins are trusted same-origin code cannot prevent such code from approving a consent on the user's behalf; the relay therefore treats a signed 42111 as the user's decision and clients that need a stronger guarantee must confirm consent outside the page, for example in a native signer.

Imported history is invisible to agents on every read path: live fan-out, `REQ` history, search and `/query` all drop import-tagged events for an agent reader, and the agent harness drops them from agent context. The agent definition and the cases that look human are listed under [Agents](#agents).

Backdating is bounded: events must fall inside the declared range, which must end before the proposal was published, and cannot exceed the declared counts. The import rate limit bounds the write load of a session.

## Relation to Other NIPs

- **NIP-DV** and **NIP-IA**: same relay-signed, parameterized-replaceable snapshot shape, signed by the NIP-11 `self` key.
- **NIP-29**: control events and imported events are ordinary channel-scoped writes and pass the same membership and role gates.
- **NIP-10** and **NIP-25**: imported replies and reactions use the existing thread and reaction tags unchanged.
