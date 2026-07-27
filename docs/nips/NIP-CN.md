NIP-CN
======

Per-Channel Notification Preferences
------------------------------------

`draft` `optional`

## Abstract

This NIP defines a scheme for synchronizing a user's own per-channel
notification preferences — notification level, temporary mute, and a small set
of per-channel toggles — across the client instances belonging to that user,
using an encrypted `kind:30078` event with the `d` value
`channel-notify-prefs`.

The blob is private to its author. It says nothing about other users, and it
carries no instruction for the relay: every decision described here is taken
client-side, at notification time.

## Motivation

A channel has exactly two notification states in most Nostr group clients:
subscribed or muted. Users need the middle ground and the temporary one — "only
tell me when someone mentions me here", "mute this for an hour", "hide this
channel until it needs me" — and they need those choices to follow them from
desktop to phone.

A boolean mute blob (`d` = `channel-mutes`) cannot express any of that, and
extending it in place is unsafe: existing clients parse it strictly and would
drop fields they do not recognize on a last-write-wins round trip, silently
erasing preferences set on a newer client. This NIP therefore defines a
**separate** document, and defines how the two interoperate (see
[Legacy `channel-mutes` Interop](#legacy-channel-mutes-interop)).

## Non-Goals

This NIP does not define relay-side notification behavior. The relay stores an
opaque encrypted blob; it MUST NOT be expected to filter, prioritize, or
suppress anything on the basis of these preferences.

This NIP does not define push delivery or any projection of these preferences
into a push transport. Mobile push wakes are the subject of NIP-PL, whose own
Non-Goals state that this NIP's preferences are not service-side flags —
"preferences are expressed as subscriptions and classes inside the lease". A
client that holds both documents MAY author a NIP-PL lease whose subscriptions
and priority classes reflect these preferences; that projection is client
policy, not part of this NIP.

This NIP does not define a per-post banner class. The level `all` means "record
every post as unread"; whether a client raises an OS banner per post is a client
UX choice, and the reference implementation deliberately does not.

This NIP does not define synced global defaults. Only per-channel divergence
from the default is stored; a user's baseline notification settings remain local
to each installation.

This NIP does not define per-installation preferences. `desktop` (and the
reserved `mobile`) are **device classes**, not device identities — there is no
per-install key in this document, and clients MUST NOT invent one by overloading
a channel id.

This NIP does not define admin- or community-side notification policy (forced
broadcast delivery, mandatory channels, and similar). That is separate work —
see issue [#2497](https://github.com/block/buzz/issues/2497).

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as
defined in RFC 2119.

- **channel**: a NIP-29 group, identified by the value of its `h` tag.
- **level**: one of `all`, `mentions`, `mute` (see [Levels](#levels)).
- **timed mute**: a `muteUntil` expiry that overlays the stored level.
- **entry**: the preference record for one channel inside the blob.
- **mention**: a direct `p` tag naming the reader.
- **broadcast marker**: a `["notify","channel"]` / `["notify","here"]` tag on a
  channel post, i.e. an `@channel` / `@here` announcement (see issue
  [#3146](https://github.com/block/buzz/issues/3146)).

## Specification

### Event Structure

Clients publish a `kind:30078` addressable event (per [NIP-78](78.md)):

```jsonc
{
  "kind": 30078,
  "pubkey": "<author>",
  "created_at": 1753600000,
  "tags": [
    ["d", "channel-notify-prefs"],
    ["t", "channel-notify-prefs"]
  ],
  "content": "<NIP-44 ciphertext, encrypted to self>"
}
```

The `d` tag MUST be exactly `channel-notify-prefs`. The `t` tag is OPTIONAL and
exists only for relay-side discoverability; clients MUST NOT filter on it.

`content` MUST be a [NIP-44](44.md) payload encrypted from the author to the
author (self-encryption), whose plaintext is the JSON document below. Because
the payload is self-encrypted, no other party — including the relay — learns
which channels a user has muted.

### Content

```jsonc
{
  "version": 1,
  "channels": {
    "<channel-id>": {
      "level": "all" | "mentions" | "mute",  // OPTIONAL; absent = "all"
      "muteUntil": 1753603600,                // OPTIONAL; absolute Unix seconds
      "desktop": true,                        // OPTIONAL; default true
      "followAllThreads": false,              // OPTIONAL; default false
      "broadcasts": true,                     // OPTIONAL; default true
      "updatedAt": 1753600000                 // REQUIRED; Unix seconds
    }
  }
}
```

| Field | Type | Default | Meaning |
|---|---|---|---|
| `level` | string | `"all"` | Notification level for the channel. |
| `muteUntil` | number | absent | Absolute Unix **seconds**. While `muteUntil > now`, the effective level is `mute`. |
| `desktop` | boolean | `true` | Deliver OS banners / sounds / dock bounce for this channel on desktop clients. |
| `followAllThreads` | boolean | `false` | Treat every thread in the channel as followed. |
| `broadcasts` | boolean | `true` | Honor broadcast markers (`@channel` / `@here`) in this channel. |
| `updatedAt` | number | — | Unix seconds; the merge key for the **whole** entry. |
| `mobile` | boolean | `true` | **RESERVED.** The `desktop` equivalent for mobile clients. |

`mobile` is reserved by this NIP and MUST NOT be written by a client that does
not implement mobile notification delivery. The reference desktop client neither
reads nor writes it. Reserving it here keeps the device-class shape stable for
the mobile follow-up instead of inviting a second, incompatible field name.

`version` MUST be `1`. A reader that encounters a different `version` MUST
ignore the blob rather than guess.

Unrecognized entry fields MUST be preserved verbatim through parse, merge and
republish, so a newer client's data survives an older client's write. This is
what makes `mobile` safe to reserve.

`updatedAt` covers the entire entry: entries are replaced atomically, never
merged field by field. A client that changes one toggle MUST republish the whole
entry with a fresh `updatedAt`.

### Sparse Entries

An entry that matches every default (`level` absent or `"all"`, no `muteUntil`,
`desktop` true, `followAllThreads` false, `broadcasts` true, no unrecognized
fields) carries no information and SHOULD NOT be stored. Clients SHOULD delete
such an entry instead of writing an all-defaults row, keeping the blob
proportional to the number of channels the user actually customized. NIP-44
payloads are capped at 65535 bytes; a client that materializes a row per channel
will eventually hit that wall.

One exception: when clearing a channel back to the defaults, a client MAY write
an explicit all-defaults entry (with a fresh `updatedAt`) if a non-default entry
for that channel may still exist on another device. Merging is a **union** of
keys (see below), so a deletion alone can be undone by an older blob
resurrecting the entry it deleted. Once the all-defaults row is the newest one
everywhere, the next default-valued write MAY drop it.

Deleting an entry cannot win a race against a concurrent remote write of that
same entry. This is an accepted limitation shared by the sibling `kind:30078`
documents.

### Fetching

```jsonc
{
  "kinds": [30078],
  "authors": ["<self>"],
  "#d": ["channel-notify-prefs"],
  "limit": 1
}
```

Clients MUST ignore any returned event whose `pubkey` is not the author's own.
Clients SHOULD also keep a live subscription on the same filter so a change made
on one device converges on the others without a poll.

### Merge Rule

Merging two blobs is a **per-channel last-write-wins union** over the channel
keys:

- Every channel id present in either blob is present in the result.
- When both blobs have an entry for a channel, the entry with the greater
  `updatedAt` wins **as a whole**.
- On a tie, the local entry wins (idempotent, and avoids a write loop between
  two devices whose clocks agree).

There is no field-level merge and no per-field timestamp. Entries are opaque
units under one `updatedAt`.

### Writing

Clients SHOULD debounce writes (the reference implementation uses 2 s), and MUST
re-fetch their own remote blob and merge into it immediately before publishing,
so a device holding stale state cannot erase a newer entry authored elsewhere.

`created_at` MUST be strictly greater than the `created_at` of the newest blob
the client has seen from itself (`max(now, lastSeen + 1)`), so a replaceable-event
store never rejects the write as older under clock skew.

Clients SHOULD suppress a publish whose merged payload is identical to the last
one they published, comparing **all** entry fields — comparing only the mute
dimension suppresses legitimate republishes of the other toggles.

A local edge made before the first remote fetch completed MUST NOT be clobbered
by the fetched snapshot: merge the fetched blob into the pending local state and
republish the result.

## Levels

| Level | Label (reference UI) | Meaning |
|---|---|---|
| `all` | All new posts | Every new post in the channel is recorded unread. The default. |
| `mentions` | Just mentions | Posts are recorded unread but do not alert; mentions and followed threads still alert. |
| `mute` | Mute and hide | The channel contributes nothing except mentions, and is hidden from channel lists. |

### Timed Mute

`muteUntil` is an **absolute epoch in seconds**, computed on the device that
sets it. Storing an absolute instant (rather than a duration plus a start time)
makes "until tomorrow at 9am" resolve in the setting user's local time zone
while remaining unambiguous on every other device.

Timed mute is an **overlay**, not a level:

- While `muteUntil > now`, the effective level is `mute`.
- The stored `level` is untouched, so expiry restores the previous level with no
  further write. Clients MUST NOT rewrite `level` when setting a timed mute.
- A timed mute MUST NOT hide the channel (see [Hiding](#hiding)).
- Expiry MUST be evaluated lazily, at resolution time, against the current
  clock. A client MUST NOT depend on a timer having fired: a client that was
  closed across the expiry, and a non-reactive consumer, both resolve correctly.
  A client MAY additionally run a coarse timer purely to refresh its UI at the
  expiry instant.
- Setting a timed mute while one is running **replaces** it; durations do not
  stack.
- Selecting any level, or explicitly unmuting, clears `muteUntil`.

Because older clients cannot express a temporary mute, a timed mute is
deliberately **not** mirrored into the legacy blob (see below): clients that do
not implement this NIP simply do not honor it.

## Resolution

All of a client's notification decisions for a channel MUST derive from one
resolved state, computed from (a) the entry in this blob, (b) the legacy
`channel-mutes` entry, and (c) the current time:

```
{ level, timedMuteActive, muteUntil, desktop, followAllThreads, broadcasts, hidden }
```

### Legacy `channel-mutes` Interop

The boolean `d` = `channel-mutes` blob remains authoritative for clients that do
not implement this NIP. Clients that implement both MUST keep them consistent:

- **Write.** When a preference write changes the channel's *durable* mute state
  — level set to `mute`, or away from `mute` — the client MUST also update the
  legacy blob (`muted: true` / `muted: false`) so older clients see the change.
  Timed mutes are exempt (see above); the other toggles have no legacy
  representation and are not mirrored.
- **Read.** When both blobs have an entry for the channel, the entry with the
  newer `updatedAt` wins **for the mute dimension only** — an unmute performed
  on an old client MUST beat a stale `"mute"` here. This blob wins ties. A newer
  legacy unmute resolves the channel to level `all`; the other fields of this
  blob still apply.
- A channel muted **only** in the legacy blob resolves to level `mute` but MUST
  NOT be hidden — users who muted under the old UI must not have channels
  disappear on them.
- **Writes MUST fold the read decision.** A preference write stamps a fresh
  `updatedAt`, so a client MUST first apply the rule above to the entry it seeds
  from. Otherwise a stale stored `level` wins retroactively over a newer legacy
  write and the channel is silently re-muted (or un-muted) because the user
  toggled an unrelated preference. One consequence is deliberate: folding a
  newer legacy *mute* materializes an explicit `level: "mute"`, so the
  "legacy-only mute never hides" rule holds only until the user next edits that
  channel's preferences on a client implementing this NIP. Keeping the two
  dimensions independent past that point would require a stored `hidden` (or a
  per-dimension timestamp) and is out of scope for this version.

### Hiding

`hidden` is derived, never stored: it is true if and only if this blob's entry
explicitly sets `level: "mute"` **and** the resolved level is `mute`. A newer
legacy unmute therefore clears hiding together with the mute, while a newer
legacy mute leaves hiding in place. Legacy-only mutes and running timed mutes
never hide.

A client that hides channels SHOULD keep two escape hatches, so a hidden channel
cannot swallow something addressed to the user:

1. the channel currently being viewed is always rendered; and
2. a channel holding a mention-tier unread is rendered (in muted styling).

Hidden channels MUST remain reachable through channel browse / search surfaces.

## Precedence Ladder (normative)

The following ladder is evaluated top to bottom for one incoming channel event;
the first matching row decides. `unread` marks the channel unread and advances
per-channel counters; `alert` is the OS banner / sound / dock-bounce tier
(clients apply their own slot and dedupe rules on top); `highPriority` is the
mention-tier (numeric badge) classification.

| # | Condition | `unread` | `alert` | `highPriority` |
|---|---|---|---|---|
| 1 | Direct-message conversation | unchanged — DM delivery bypasses channel levels entirely | | |
| 2 | Direct `p`-tag mention of self | ✅ | ✅ | ✅ |
| 3 | Broadcast marker (`@channel` / `@here`), and effective level ≠ `mute`, and `broadcasts` ≠ false | ✅ | ✅ | ✅ |
| 4 | Broadcast marker, but effective level = `mute` **or** `broadcasts` = false | — fall through to row 5 as an ordinary post — | | |
| 5 | Top-level post, or a NIP-CW broadcast reply — level `all` | ✅ | ✅ | broadcast reply only |
| 6 | Top-level post, or a NIP-CW broadcast reply — level `mentions` | ✅ | ❌ | ❌ |
| 7 | Top-level post, or a NIP-CW broadcast reply — level `mute` | ❌ | ❌ | ❌ |
| 8 | Thread reply whose root the user muted | ❌ | ❌ | ❌ |
| 9 | Thread reply, thread followed (explicitly, by participation, by authorship, or via `followAllThreads`) — level `all` or `mentions` | ✅ | ✅ | ❌ |
| 10 | Thread reply, thread followed — level `mute` | ❌ | ❌ | ❌ |
| 11 | Anything else | ❌ | ❌ | ❌ |

Notes on the ladder:

- **Mentions pierce mute** (row 2). A muted channel still badges a direct
  mention; this matches Slack and is what makes "Mute and hide" safe.
- **Broadcast markers do not pierce mute** (rows 3–4). `@channel` is
  channel-wide attention, and a muted channel has opted out of channel-wide
  attention. The per-channel `broadcasts` toggle opts out while leaving the rest
  of the level intact.
- **`["notify",…]` and NIP-CW `["broadcast","1"]` are different things.** The
  first marks a channel-wide mention; the second marks a thread reply surfaced
  to the channel timeline. Only the first is governed by `broadcasts`; the second
  is governed by the level like any top-level post.
- **`followAllThreads` promotes replies, it does not defeat mute** (rows 9–10).
- `desktop` is deliberately absent from the ladder. It is a **delivery-side**
  gate: a client MUST apply it where an alert is actually delivered, and MUST
  NOT let it change `unread` or `highPriority`. The ladder stays
  device-agnostic.
- A client whose notification surface has no event graph (e.g. a server-built
  activity feed) MUST still apply the channel dimension of this ladder, **in the
  ladder's order**: a direct `p`-tag mention of self pierces first (row 2) —
  including when the same item also carries a broadcast marker; only items
  without such a mention fall to the broadcast-marker rows and obey the level
  and `broadcasts`; everything else is suppressed while the channel resolves to
  `mute`.

Level changes apply at resolution time. A client MUST NOT re-notify already
delivered events because a level changed, and MUST NOT retroactively re-tier
recorded unread events.

## Privacy Considerations

The blob names every channel the user has customized, which leaks the shape of
their attention. It is therefore self-encrypted with NIP-44 and MUST NOT be
published in plaintext, even though its contents are "only preferences".

Metadata still leaks: the relay sees that the author holds a
`channel-notify-prefs` blob and how often it changes. Clients SHOULD debounce
writes (which they already do for convergence reasons) rather than publish per
keystroke or per toggle.

## Kind Usage

`kind:30078` (NIP-78 application-specific data), `d` = `channel-notify-prefs`.
This is one of several `kind:30078` documents in this family, distinguished only
by their `d` values (`read-state:<slotId>` — NIP-RS, `channel-sections`,
`channel-mutes`, `channel-stars`, `channel-sort`).

## Backwards Compatibility

Clients that do not implement this NIP ignore the blob entirely and continue to
honor `channel-mutes`, which implementers keep in sync for the durable mute
dimension. Levels, timed mutes, and the per-channel toggles degrade to "not
honored" on those clients — never to a wrong value.

No relay changes are required: `kind:30078` is generic addressable storage with
no `d`-value allowlist.

## References

- [NIP-29](29.md) — relay-based groups (channels, `h` tags)
- [NIP-44](44.md) — encrypted payloads
- [NIP-78](78.md) — application-specific data (`kind:30078`)
- NIP-RS — cross-device read state sync (sibling `kind:30078` document)
- NIP-PL — push leases; the eventual mobile push projection and its Non-Goals
- NIP-CW — channel-wide broadcast replies (`["broadcast","1"]`)
- Issue [#3146](https://github.com/block/buzz/issues/3146) — `@channel` /
  `@here` broadcast markers, the source of the `["notify",…]` tag
- Issue [#3160](https://github.com/block/buzz/issues/3160) — per-channel
  notification settings
