NIP-TC
======

To-Do Cards
-----------

`draft` `optional` `client` `relay`

**Depends on**: NIP-01 (basic event format), NIP-29-style channel scoping (`h` tags)

## Abstract

This NIP defines interactive to-do cards inside ordinary Buzz channel messages. A card is a fenced ` ```buzz:todo-card ` JSON payload embedded in a normal stream message (`kind:9` / `kind:40002`); clients that understand the fence render a native checklist card, while every other client falls back to the message's prose. Check-offs are separate user-signed response events (`kind:40009`) that reference the card and item; card state is a pure client-side fold over those responses, so the relay stores nothing card-specific and needs no new read model.

The protocol has one new event kind:

- a user-signed card response (`kind:40009` interactive-card response).

There is no card event kind. The card itself rides inside an existing message kind as a sentinel payload, mirroring the config-nudge sentinel pattern.

## Motivation

Teams coordinating in a Buzz channel routinely post checklists as prose ("- [ ] flip the flag"). Prose cannot be checked off, attributed, or kept in sync across viewers. A dedicated card event kind would require new relay storage, new feed/unread plumbing, and a migration for every render surface. Embedding the card in a normal message keeps delivery, threading, editing, deletion, unreads, and permissions exactly as they are — the card is just message content — while the small, append-only response events carry the interactive state.

## Non-Goals

This NIP does not define relay-side aggregation. The relay never computes card state; clients fold responses themselves.

This NIP does not add read gates. `kind:40009` responses are ordinary channel-scoped events readable by any channel member, and they are deliberately absent from feed, unread, and mention queries.

This NIP does not define composer UI. v1 cards are authored by agents and CLI tooling via the SDK builder; humans interact by checking items, not by composing cards.

This NIP does not cover multi-list cards, due dates, ordering edits, or item mutation. A card's items are immutable after publish; only completion state changes.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **card message**: A `kind:9` or `kind:40002` stream message whose content contains a `buzz:todo-card` fenced payload.
- **card event id**: The Nostr event id of the card message. Responses reference it via an `e` tag.
- **item**: One checklist entry inside a card, identified by a card-unique string id.
- **assignee**: The optional pubkey named on an item. Absence means anyone in the channel may complete the item.
- **response**: A user-signed `kind:40009` event asserting `done` or not-done for one (card, item) pair.

## Kinds

| Kind | Name | Signer | Storage | Purpose |
|------|------|--------|---------|---------|
| `40009` | Interactive-Card Response | user | regular | One check/un-check of one card item |

`kind:40009` is a regular event. It is channel-scoped: the relay requires an `h` tag and enforces channel membership and `MessagesWrite` scope exactly as for stream messages. It MUST NOT appear in timeline, unread, or mention kind sets — responses are not rows; clients fetch them per card by `#e` reference.

## Payload Format

### The `buzz:todo-card` sentinel

A card message's content is ordinary prose followed by a fenced JSON payload:

````
Launch checklist for Thursday:

```buzz:todo-card
{"v":1,"title":"Launch","items":[{"id":"a1","text":"Flip the flag","assignee":"<pubkey-hex>"},{"id":"b2","text":"Verify dashboards"}]}
```
````

The v1 payload schema:

```jsonc
{
  "v": 1,                     // REQUIRED, literal 1
  "title": "Launch",          // OPTIONAL string
  "items": [                  // REQUIRED, 1..=20 entries
    {
      "id": "a1",             // REQUIRED, non-empty, unique within the card
      "text": "Flip the flag",// REQUIRED string
      "assignee": "<hex>"     // OPTIONAL 64-char lowercase hex pubkey
    }
  ]
}
```

Constraints:

- `items` MUST contain between 1 and 20 entries. Clients MUST treat a payload violating any constraint as malformed.
- item `id`s MUST be non-empty and unique within the card. They are referenced verbatim by response `item` tags.
- A malformed, unterminated, over-cap, or wrong-version payload MUST cause the client to fall back to rendering the raw prose (including the fence) — never a partial card.
- The card message SHOULD carry one `p` tag per distinct assignee so existing mention delivery notifies them.

Prose above the fence is the fallback body. A rendering client suppresses the fence and shows the native card; a non-rendering client shows the prose and the fence as plain text.

### `kind:40009` Interactive-Card Response

```jsonc
{
  "kind": 40009,
  "pubkey": "<responder-pubkey-hex>",
  "content": "{\"done\":true}",
  "tags": [
    ["h", "<channel-id>"],
    ["e", "<card-event-id>"],
    ["item", "<item-id>"]
  ]
}
```

Required tags: exactly one `h` (the card's channel), one `e` (the card message's event id), and one `item` (the item id from the card payload). The content is a JSON object with a single boolean field `done`. `{"done":false}` is an explicit un-check, not a deletion.

Anyone with `MessagesWrite` in the channel MAY respond to any item, including items assigned to someone else — completion is attributed, not restricted.

## State Fold

Card state is a deterministic pure function of the card payload plus the set of `kind:40009` events whose `e` tag equals the card event id. For each item:

1. Discard responses whose `item` tag names an unknown item id or whose content is not valid `{"done":bool}` JSON.
2. Order responses by `created_at` ascending, tie-broken by event id; keep only the **latest response per responder pubkey**.
3. If the item has an assignee and the assignee has responded, the assignee's latest response decides the item's state (an assignee's `{"done":false}` overrides anyone else's completion).
4. Otherwise the item is done iff any responder's latest response is `{"done":true}`; the most recent such responder is the attributed completer.

Consequences: un-checking only retracts your own completion (your latest response flips to `done:false`; someone else's `done:true` still stands, except an assignee's un-check which is authoritative for their item), and every completion is attributable to the signing pubkey.

## Relay Processing

The relay treats `kind:40009` as one more channel-scoped content kind: `MessagesWrite` scope required, `h` tag required, channel membership enforced by the generic ingest pipeline. No new storage, index, or query path is introduced. Because feed, unread, and mention queries use explicit kind inclusion lists, responses are invisible to them by construction and MUST remain so — a check-off never creates an unread badge or a notification to the card author.

## Client Behavior

A rendering client SHOULD:

1. Detect the sentinel when rendering a message body; on a valid payload, render the card and suppress the fence.
2. Subscribe live to `kinds:[40009]`, `#h:[<channel>]`, `#e:[<card-event-id>]` while the card is on screen, folding events per §State Fold (deduplicating its own publish acknowledgements against subscription echoes).
3. Publish a `kind:40009` on toggle, disabling the control while the publish is in flight.
4. Disable un-check on items completed by someone else (except for the viewer's own completions), and attribute completions with the completer's profile.

Mobile and other non-rendering clients need no changes: they show the prose fallback.

## Security Considerations

Responses are user-signed, so every check-off is attributable and unforgeable. The relay's existing channel-membership and scope enforcement is the only write gate; there is no way to respond to a card in a channel you cannot post in. Clients MUST validate the payload before rendering interactive controls so a crafted fence cannot render a partial or misleading card, and MUST ignore responses referencing unknown item ids.

Because anyone in the channel may complete any item, a hostile channel member can mark items done; the fold's attribution (and an assignee's authoritative un-check) is the mitigation, matching the trust model of posting messages in the channel itself.

## Relation to Other NIPs

- **Config-nudge sentinel**: Same fenced-JSON-in-message carrier; NIP-TC generalizes it to interactive state with a response kind.
- **Huddle events (`kind:48100`–`48103`)**: Same client-side fold pattern (state = reduce over channel-scoped events); NIP-TC applies it per (card, item, pubkey).
- **NIP-29-style channel scoping**: `kind:40009` inherits the standard `h`-tag membership enforcement; no NIP-TC-specific gates exist.
