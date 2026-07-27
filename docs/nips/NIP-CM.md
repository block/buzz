NIP-CM
======

Channel Mentions
----------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format, tags), NIP-29 (relay-based groups). Interacts with NIP-CW (channel window, `["broadcast","1"]`) and NIP-PL (push leases).

## Abstract

This NIP defines the **channel-wide mention**: a single marker tag on an ordinary channel message that asks the channel's *members* — not a listed set of pubkeys — to be notified.

```jsonc
["notify", "channel"]   // every member of the channel; persistent
["notify", "here"]      // members who are online right now; live-only
```

There is no per-member `p`-tag expansion. The event carries one two-element tag whose second element names *who*, and the relay resolves that to people at read time by joining the channel roster. The roster never enters the event, the notification never fans a `p` tag at an agent, and a client that does not implement this NIP sees a normal message carrying one unknown tag.

## Motivation

"Notify everyone in this channel" is the one notification intent that a `p`-tag list cannot express honestly. Expanding the roster into the event breaks on every axis at once: implementations cap mention counts (Buzz: 50 client-side), anti-hellthread gates drop high-`p`-count events outright (NIP-PL `suppress.p_tags_max`), each `p` tag is a wake signal that starts an AI agent turn, the mention index takes one write per member per message, and the full roster — including members a reader cannot otherwise enumerate — is published in the event body forever. The expansion is also a lie about time: the roster is frozen at send, so a member who joins a minute later is not mentioned and a member who left is.

A marker tag inverts all of that. It is O(1) on the wire and one row in storage regardless of channel size, it carries no identities so it cannot wake an agent or leak a roster, and because recipients are resolved when the feed is read, membership is always evaluated as of the read. `["broadcast","1"]` (NIP-CW) already established the shape in this relay: a channel-wide fact stated as a flag, not as a list.

## Non-Goals

This NIP does not define who *may* send a channel-wide mention. In this version any channel member may; per-channel restriction is left to a future permission mechanism.

This NIP does not add an event kind, a notification category, or a user-facing preference. Channel-wide mentions reuse the existing mention category and the existing channel-mute control; the mute is the opt-out.

This NIP does not define agent behavior, because there is none: `channel` and `here` are not identities and MUST NOT resolve to one. Nothing in this NIP wakes an AI agent.

This NIP does not define presence. `@here` consumes whatever presence the client already has; it specifies no presence protocol and stores no online set.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **notify tag**: the tag `["notify", <mode>]` defined by this NIP.
- **mode**: `channel` or `here` — the two defined second elements.
- **channel-wide mention**: a message carrying a valid notify tag.
- **direct mention**: the pre-existing per-person mention, a `["p", <pubkey>]` tag on a message. Unchanged by this NIP.
- **member**: a pubkey with a current, non-removed membership row for the channel (NIP-29 group membership).
- **reserved token**: the literal text `@channel` or `@here` in message content, compared case-insensitively without the `@`.
- **escalate**: to raise an event to mention tier for a reader — badge, notification, mention feed row.

## The Notify Tag

```jsonc
{
  "kind": 9,
  "content": "deploy freeze starts in 10 minutes @channel",
  "tags": [
    ["h", "<channel-id>"],
    ["notify", "channel"]
  ]
}
```

- **Name.** The tag name is exactly `notify`.
- **Mode.** The second element MUST be exactly `channel` or `here`, lowercase. Parsing is case-sensitive: `Channel` is not a mode. Any other value is a malformed tag, not an unknown-and-ignorable one.
- **Cardinality.** An event MUST carry at most one notify tag. Two notify tags — even two identical ones — are a malformed event; there is no "last wins" rule, because "notify who?" has no safe default.
- **Extra elements.** Elements past the mode MUST be ignored by parsers, per the usual Nostr forward-compatibility convention.
- **No identities.** The notify tag never contains a pubkey, and a channel-wide mention MUST NOT be accompanied by a roster expansion of `p` tags. A message MAY carry both a notify tag and ordinary `p` tags for people it also mentions by name; those are direct mentions and keep their own semantics.

### Allowed Kinds

A notify tag is meaningful only on the kinds that carry channel-scoped prose:

| Kind | Meaning | Notifies | Feed row |
|------|---------|----------|----------|
| `9` | stream message | yes | yes (`channel`) |
| `45001` | forum post | yes | yes (`channel`) |
| `45003` | forum comment | yes | yes (`channel`) |
| `40003` | message edit | **no** | **no** |

`40003` accepts the tag for *render continuity* only: an edit of a message that said `@channel` must still render the token as a mention chip. It MUST NOT re-notify and MUST NOT create or refresh a feed row — otherwise editing a typo would re-blast the channel, and repeated edits would be an amplification primitive.

On any other kind a notify tag MUST be rejected. Silently ignoring it is the worse failure: the sender is told the message shipped and believes the channel was notified.

### Mode Semantics

**`channel`** — every current member of the channel. Persistent: it produces a stored feed row, so a member who was offline when it was sent still finds it on return, and it counts toward mention-tier unread state.

**`here`** — members who are online at the moment of delivery. Live-only, by construction: **no notification is ever recorded** for it — no index row, no mentions-feed entry, no retroactive badge state. The event itself is an ordinary stored channel message like any other; it is the *mention* that has no persistent form. A reader who was offline, or who scrolls the message into view an hour later, gets nothing beyond a normal unread message. `@here` is a doorbell, not a letter.

## Relay Processing

A relay implementing this NIP validates the notify tag at ingest, before storage and before fan-out, for **every** kind — not only the allowed ones, so that a tag on a disallowed kind is rejected rather than dropped.

Validation, in order:

1. **Shape** — at most one notify tag; the tag has a mode element; the mode is `channel` or `here`. Shape errors are reported before the kind gate, so a duplicate tag on a disallowed kind reports the duplicate.
2. **Kind** — if a valid tag is present, the kind MUST be one of `9`, `40003`, `45001`, `45003`.
3. **Channel type** — a notify tag MUST be rejected on an event addressed to a DM channel. A DM has no "everyone else" to notify, so the tag is meaningless there and its presence signals a confused client.

Rejection is a normal validation rejection on the relay's existing surface (Buzz: `OK false` with an `invalid: …` message on WebSocket, HTTP `400` on `POST /events`). Accepted events are stored and fanned out exactly as any other message; the notify tag is preserved verbatim in the stored event, which is how clients see it.

Steps 1–2 are pure functions of the event and are implemented in `buzz_core::channel_mentions`; step 3 needs the channel row and is applied at the ingest seam.

### Feed Storage

On accept, an event with `mode = channel` on kind `9`, `45001`, or `45003` records **one row** in a `channel_notifications` index — one row per *event*, never one per member:

| Column | Meaning |
|--------|---------|
| `community_id`, `event_id` | primary key; the tenant and the notifying event |
| `channel_id` | the channel whose roster resolves recipients at read time |
| `mode` | constrained to `'channel'` — `here` has no persistent notification form |
| `event_created_at` | ordering key, indexed `(community_id, channel_id, event_created_at DESC)` |

`mode = here` and kind `40003` write nothing.

Like the direct-mention index, this is a denormalized read index maintained alongside the event insert, not inside it: an index failure is logged and the message is still delivered. A message that reached the channel but is missing from one reader's mention feed is a degradation; a message rejected because an index write failed is data loss.

### Mentions Feed

The mentions feed is the union of two branches, deduplicated by event:

1. direct `p`-tag mentions of the caller, via the mention index;
2. `channel_notifications` rows whose `channel_id` the caller is **currently** a member of (`channel_members`, non-removed).

Both branches project identical event columns so `UNION` collapses an event that is both `p`-tagged at the caller *and* `@channel` into exactly one row. Both branches apply the caller's existing channel visibility scoping and the deleted-event filter, and the union is ordered by the event's own `created_at` under the same feed limit as before (Buzz: 100).

Two consequences worth stating explicitly:

- **Membership is evaluated at read time.** Someone who joins the channel after an `@channel` message sees it in their mentions feed; someone who has left does not. This is the behavior a frozen `p`-tag list cannot have.
- **The author is excluded.** Branch 2 omits events authored by the caller, so your own `@channel` announcement is not a mention *of you*. (Branch 1 keeps its pre-existing behavior.)

`@here` is absent from both branches by construction — it writes no index row, so there is nothing to union.

## Client Behavior

### Sending

1. A client SHOULD require an explicit, deliberate act to attach the tag — a selected autocomplete entry, a confirmation, or an explicit flag — rather than inferring it from prose. Typing the characters `@channel` MUST NOT by itself attach the tag.
2. A client that sees a reserved token in outgoing content with no tag attached SHOULD warn that nobody will be notified, and MUST still send the message. (Buzz CLI: `buzz messages send --notify channel|here`; without the flag, a literal `@channel` prints a stderr warning and the message sends untagged, exit `0`.)
3. Detection of reserved tokens for that warning, and for any confirmation prompt, MUST mask code regions: an `@here` inside a fenced block or a backtick span is documentation, not a mention.
4. When both reserved tokens appear in one message, a client attaching a single tag SHOULD choose `channel` — it is the broader, safer read of the author's intent, and only one tag may be attached.
5. Clients SHOULD NOT offer the tag in DM channels, where the relay rejects it.

### Reserved-Token Precedence

`channel` and `here` are reserved, case-insensitively, in **every** mention parser. A member whose display name is literally "here" MUST NOT be resolved from the token `@here`: the reserved token wins, the parser emits no `p` tag for it, and the token is consumed so it cannot fall through to name matching. This holds on the send path (mention resolution, autocomplete aliases), on the render path, and in any server-side automation that resolves names to pubkeys.

The consequence is deliberate and one-directional: the reserved words cost two display names their `@`-addressability, rather than letting a chosen display name hijack a channel-wide blast.

### Rendering

`@channel` / `@here` render as a mention chip **only** when the event actually carries the corresponding notify tag. Unbacked literal text stays plain — the same rule already applied to a `@name` with no backing `p` tag. A chip that is not backed by a marker would tell every reader they were notified when nobody was.

The chip is not clickable: there is no profile behind it.

### Notification Ladder

Order matters, and mute placement is the whole design:

1. `["broadcast","1"]` — unchanged; pierces a channel mute.
2. Direct `p`-tag mention of the reader — unchanged; pierces a channel mute.
3. **Channel mute — suppresses everything below.**
4. `notify: channel` — mention tier: badge, mention sound, OS notification, mention feed row.
5. `notify: here` — mention tier **iff** the liveness test below passes.
6. Existing per-message and per-thread rules.

Muting a channel MUST suppress its channel-wide mentions. A channel-wide mention is precisely the class of message a mute exists to stop; a direct mention, which someone chose to send *to you*, is not. Clients that receive mention-category feed items from a relay MUST apply this suppression on the feed path too — the relay files `@channel` items under `mention`, and only the client knows the mute.

A reader never escalates their own message, at any rung.

### `@here` Liveness

An event with `notify: here` escalates for a reader iff, **at observation time**, both hold:

- the reader's own presence reads as online; and
- `|now − event.created_at| ≤ 120` seconds.

The window absorbs clock skew and relay latency without letting `@here` become a persistent mention; 120 seconds is this implementation's value and other implementations MAY choose another, but it MUST be short enough that a message pulled from history never escalates. Because the test is evaluated at observation, the same event escalates for one reader and not another — that is the definition of "here", not an inconsistency.

Since `@here` records no notification, the live delivery path owns its notification entirely; the feed path can never produce one. Conversely `@channel` notifications SHOULD be owned by whichever single path a client already uses for mention notifications, with the other path skipping notify-tagged events, so one message never notifies twice.

## Degradation

- **Tag-unaware client**: sees a normal message with one unrecognized tag, per NIP-01 tag tolerance. It renders the message and the literal text `@channel`, and applies its ordinary unread rules. It under-notifies; it never misbehaves.
- **Tag-unaware relay**: stores and fans out the tag as an opaque tag. Clients still see the marker and can still escalate live; only the server-side mentions feed row is missing, so offline catch-up degrades to normal unread state. There is no fallback that could produce a *wrong* recipient set, because no recipient set is ever transmitted.
- **Foreign Nostr clients** publishing into a Buzz channel are subject to the same ingest validation as any other client; a malformed notify tag is rejected with the event, not stripped from it.

A relay implementing this NIP MAY advertise it in its NIP-11 document; clients need no advertisement, since an unsupported relay simply rejects or opaquely stores the tag.

## Security and Privacy Considerations

**No roster disclosure.** The event never names the people it notifies, so a channel-wide mention discloses nothing about membership — including to members who cannot enumerate the roster, and to any relay or client that later replicates the event.

**No agent amplification.** Agent wake is driven by `p` tags. A channel-wide mention has none, so it cannot start an agent turn. Implementations MUST NOT add roster expansion "for agents": it would make a single message start N agent turns, which is the failure mode this design exists to prevent.

**Bounded amplification.** Cost to the relay is O(1) per notifying event — one validation and at most one index row — regardless of channel size. Abuse is therefore a *social* problem (too many blasts) addressable by a permission rule, not a resource-exhaustion problem. Edits are excluded from re-notification for the same reason.

**Mute is enforced client-side.** The relay files `@channel` events into the mentions feed for all members; the mute lives with the reader. A client that ignores the mute rung will over-notify a user who asked for silence. Implementations MUST apply the ladder above on every path that can raise a notification.

**Tenant isolation** is unchanged: the feed index is community-keyed and joins the community-scoped roster, and the union keeps the caller's existing channel visibility scoping. A channel a reader cannot see contributes no mention rows.

## Implementation Gotchas

- **Validate for every kind, not just the allowed four.** A kind gate that runs only inside a per-kind handler silently drops the tag on kinds it never reaches — the sender then believes the channel was notified.
- **Both feed branches must project identical columns.** If one branch aliases its index's `event_created_at` into the select list, `UNION` compares a value two independent denormalized indexes have to keep byte-identical, and an event that is both `p`-tagged and `@channel` shows up twice. Project the event columns only, order on the event's own `created_at`.
- **Join `channel_members`, not a cached roster helper.** Roster helpers may cap their result; the feed join must be the raw membership relation or large channels lose recipients.
- **Duplicate-tag detection must precede the kind gate**, or a duplicate on a disallowed kind reports the wrong error and a client "fixes" the wrong thing.
- **The `@here` predicate needs the reader's *own* presence**, which usually lives in UI state far from event handling. Read it through a small explicit seam and pass it as an argument in tests, so the predicate stays deterministic.
- **Community-scoped caches** holding presence or notify state must be reset on community switch, like every other community-scoped singleton.

## Relation to Other NIPs

- **NIP-01**: supplies the tag grammar and the unknown-tag tolerance that makes degradation free.
- **NIP-29**: supplies the channel (`h` tag) and the membership relation that resolves recipients at read time.
- **NIP-CW (`["broadcast","1"]`)**: the closest sibling and a distinct thing. `broadcast` is about **placement** — it lifts a depth-1 reply onto the channel timeline as a window row — and it says nothing about notification recipients. `notify` is about **recipients** and says nothing about placement. They are orthogonal and composable: a broadcast reply may also carry a notify tag. Notably `broadcast` pierces a channel mute and `notify` does not, because the mute is exactly the control for "stop telling me about this channel."
- **Direct mentions (`["p", <pubkey>]`)**: the per-person mention, unchanged by this NIP. It names an identity, it is frozen at send time, it pierces a channel mute, and it can wake an agent. A notify tag does none of those. A message may carry both; each keeps its own semantics, and the mentions feed collapses the pair into a single row. `["mention", …]` reference-style tags, where used, are references — never a notification instruction.
- **NIP-PL**: push leases match filters over stored events, and this NIP changes nothing there. A notify-tagged message is a stored channel message, so it matches a lease exactly as the same message without the tag would — for both modes. Push delivery therefore carries no channel-mention tier: an `@here` that matches a reader's `#h` lease pushes as an ordinary channel message, not as a mention, which is the conservative outcome (a push cannot know whether the reader is *here*). Because there is no roster expansion, channel-wide mentions never appear in a lease's `#p` match set and cannot trip its `suppress.p_tags_max` hellthread gate.
- **NIP-RS**: read state is unaffected; a channel-wide mention changes the *tier* of unread state, not its bookkeeping.
