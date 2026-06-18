NIP-DV
======

DM Visibility
-------------

`draft` `optional` `relay`

**Depends on**: NIP-01 (basic event format), NIP-11 (relay information document), NIP-43 (Relay Access Metadata and Requests)

## Abstract

This NIP defines a relay-scoped, per-viewer projection of DM (direct message) hide state. A viewer can hide a DM conversation from their sidebar without leaving it: they remain an active member, still receive messages, and can re-open it later. The relay tracks this hide state privately. This NIP exposes it as a single relay-signed, parameterized-replaceable event per viewer so that pure-Nostr clients can filter hidden DMs out of the conversation list without any other source of truth.

The protocol has one relay-signed event kind:

- a relay-signed per-viewer snapshot (`kind:30622` DM visibility snapshot).

There is no user-signed request kind. The hide/unhide intent is already carried by the existing DM commands (`kind:41012` hide, `kind:41010` open/re-open); the relay derives and re-publishes the visibility snapshot as a side effect of accepting those commands.

## Motivation

Buzz DMs are surfaced to clients as NIP-29-style group membership (`kind:39002`), where the viewer appears as a `#p` participant. Hiding a DM is presentation state, not membership: the viewer stays in the member list because they can still receive messages and re-open the conversation. So `kind:39002` correctly continues to list the viewer, and a client that rebuilds its DM list from `kind:39002` alone cannot tell which DMs the viewer has hidden.

The relay does know — it records `hidden_at` per (viewer, channel) — but never emits that fact as a queryable Nostr event. A thin client is therefore flying blind on a piece of state only the relay holds. The result is the visible bug: a hidden DM is optimistically removed, then reappears on the next conversation-list refetch because the refetch is rebuilt from `kind:39002`, which never carried the hide.

NIP-DV fills that gap. The relay publishes a transparent, relay-signed, per-viewer snapshot of the currently-hidden DM set. Clients read the latest snapshot and filter hidden DMs out of the sidebar, while membership and message delivery are unaffected.

## Non-Goals

This NIP does not change membership. A hidden DM keeps the viewer as an active `kind:39002` participant; message delivery and re-open are unaffected.

This NIP does not delete events. No DM message or membership event is removed.

This NIP does not define a shared or global hide state. The snapshot is per-viewer and relay-scoped. A viewer's hide state is theirs alone; the other DM participant's view is unaffected.

This NIP does not define a user-signed request kind. Hide and unhide intent is already expressed by `kind:41012` and `kind:41010`. NIP-DV only describes the relay-signed projection.

## Terminology

This document uses MUST, MUST NOT, SHOULD, SHOULD NOT, MAY, and RECOMMENDED as defined in RFC 2119.

- **relay identity**: The relay signing pubkey advertised in its NIP-11 `self` field. NIP-DV relay-signed events are valid only when signed by this key.
- **viewer**: The pubkey whose per-viewer hide state a given snapshot describes.
- **hidden DM**: A DM channel the viewer currently has hidden (`hidden_at IS NOT NULL`) while still being an active, non-removed member.
- **visibility snapshot**: A relay-signed `kind:30622` event listing every DM the viewer currently has hidden.

## Kinds

| Kind | Name | Signer | Storage | Purpose |
|------|------|--------|---------|---------|
| `30622` | DM Visibility Snapshot | relay | parameterized-replaceable | Current per-viewer hidden-DM set |

`kind:30622` is parameterized-replaceable per NIP-01 (`30000 <= n < 40000`), keyed by its `d` tag. The `d` tag is the viewer's pubkey, so there is exactly one current snapshot per viewer. Clients use the latest valid `kind:30622` signed by the relay identity, addressed by `d` = the viewer's pubkey, as current state.

The snapshot is relay-scoped: it is signed by the relay identity advertised in NIP-11 `self`, mirroring NIP-IA's relay-signed snapshot shape. Relays without a stable NIP-11 `self` pubkey MUST NOT publish NIP-DV relay-signed state, because clients would have no stable key against which to verify it.

## Event Formats

### `kind:30622` DM Visibility Snapshot

A visibility snapshot is signed by the relay identity. It carries one `h` tag per DM channel the viewer currently has hidden.

```jsonc
{
  "kind": 30622,
  "pubkey": "<relay-identity-pubkey-hex>",
  "content": "",
  "tags": [
    ["d", "<viewer-pubkey-hex>"],
    ["p", "<viewer-pubkey-hex>"],
    ["h", "<hidden-dm-channel-id>"],
    ["h", "<hidden-dm-channel-id>"]
  ]
}
```

Required tags:

- exactly one `d` tag whose value is the viewer's 64-character lowercase hex pubkey. This is the parameterized-replaceable address key.
- exactly one `p` tag whose value equals the `d` value (the viewer's pubkey). The `p` tag is the read-authorization key: relays that `#p`-gate per-viewer state (see §Privacy Considerations) use it to restrict reads to the snapshot's owner. The `d` and `p` tags are deliberately redundant — `d` addresses the replaceable event, `p` authorizes the reader.

Optional tags:

- zero or more `h` tags, each identifying a DM channel the viewer currently has hidden. A snapshot with no `h` tags means the viewer has no hidden DMs (their hidden set is empty). Order is not significant; clients MUST treat `h` tags as a set.

The `content` field is empty and carries no meaning. Clients MUST NOT parse semantics from `content`.

## Relay Processing Algorithm

After the relay accepts and commits a DM command that changes a viewer's hide state, it republishes that viewer's snapshot:

1. On `kind:41012` (hide): the viewer's `hidden_at` for the target channel is set.
2. On `kind:41010` (open/re-open) that clears an existing `hidden_at`: the viewer's hide state for the target channel is cleared.

In both cases the relay recomputes the viewer's full hidden-DM set from its authoritative state (active, non-removed DM memberships with `hidden_at IS NOT NULL`) and publishes a fresh `kind:30622` snapshot signed by the relay identity, with `d` = the viewer's pubkey and one `h` tag per hidden DM.

The recompute-and-replace shape means the latest snapshot is always the complete, authoritative hidden set. There is no delta event to merge and no ordering hazard between hide and unhide: a stale snapshot is simply superseded by the newer one under NIP-01 parameterized-replaceable semantics.

Snapshot publication is a best-effort post-commit side effect. If publication fails, the hide/unhide command itself still succeeds; the relay SHOULD republish the snapshot on the next state change. A momentarily stale snapshot only affects sidebar presentation, never membership or delivery.

## Client Behavior

A client that rebuilds its DM list from `kind:39002` membership SHOULD additionally:

1. Query its own latest snapshot: `kinds: [30622]`, `#p: [<my-pubkey>]`, `limit: 1`. The query is keyed by `#p` (not `#d`) because that is the tag the relay's read-authorization gate checks.
2. If a snapshot exists, collect its `h` tag values into a set of hidden DM channel ids.
3. Filter the DM list, dropping any DM whose channel id is in that set. Non-DM channels MUST NOT be affected.

A client SHOULD verify that the snapshot is signed by the relay identity before trusting it (see §Security Considerations for the current implementation posture). A client that finds no snapshot MUST treat the hidden set as empty (no DMs hidden).

## Implementation Gotchas

- The snapshot is keyed by the viewer's pubkey via the `d` tag, not by channel. There is one event per viewer listing all their hidden DMs, not one event per (viewer, channel) pair.
- Hiding a DM does not remove the viewer from `kind:39002`. A client MUST NOT infer hide state from membership, and MUST NOT filter the viewer out of the member list — doing so would break re-open and message delivery.
- `kind:41010` (open) is used both to first-open a DM and to re-open a hidden one. Only the re-open path (which clears an existing `hidden_at`) needs to refresh the snapshot; a first-open had nothing hidden to clear.

## Security Considerations

The snapshot is relay-signed and relay-scoped. A client SHOULD verify the relay-identity signature before applying it; a snapshot signed by any other key is invalid and MUST be ignored.

Current implementation posture: the desktop client trusts whatever the configured relay returns from its authenticated `/query` endpoint and does not yet re-verify the relay-identity signature client-side, matching NIP-IA's current behavior for relay-signed state. This is acceptable because the relay is the configured, authenticated source of truth and the connection is authenticated (NIP-42 / NIP-98). Wiring explicit client-side relay-identity verification is a cross-cutting hardening that should be applied uniformly across all relay-signed reads (NIP-DV, NIP-IA, NIP-OA) rather than piecemeal here.

## Privacy Considerations

A viewer's hidden-DM set is per-viewer presentation state. The snapshot is addressed to the viewer (`d` = viewer pubkey) and reveals which DM conversations that viewer has chosen to hide. To prevent one viewer from enumerating another's hide choices, the relay MUST scope read access so that only the snapshot's owner can read it.

This NIP achieves that with two layers. First, a filter-level `#p` read-authorization gate: the snapshot carries `p` = viewer, and the relay rejects any query for `kind:30622` whose `#p` filter is absent or does not equal the authenticated reader's pubkey — the same gate that protects member-add/remove notifications and gift wraps. Second, a result-level owner check applied at every delivery surface (HTTP query, WebSocket historical, live fan-out, and NIP-50 search): a `kind:30622` event is only handed to a reader whose pubkey equals its `#p`. The result-level check closes the gap where a filter-level gate alone could be bypassed — most importantly a kindless `ids:[<known-snapshot-id>]` query, since the snapshot is relay-signed (its id is not author-bound) and its content is plaintext private hide choices. As defense in depth, the relay also excludes `kind:30622` from its full-text search index entirely, so a snapshot never becomes a search hit in the first place. A relay that exposes NIP-DV snapshots without owner-scoped read access MUST NOT do so.

## Implementation Note: Write Protection

`kind:30622` is relay-only. Relays MUST reject client-submitted events of this kind: only the relay identity may author a snapshot. Combined with the `#p` read-gate above, a snapshot can be neither forged by a client nor read by a non-owner.

## Relation to Other NIPs

- **NIP-IA (Identity Archival)**: Same relay-signed-snapshot shape (user-or-relay intent → relay-signed, replaceable, relay-scoped state). NIP-DV applies the pattern per-viewer for DM presentation rather than per-pubkey for membership surfaces.
- **NIP-43 (Relay Access Metadata and Requests)**: Defines membership/access control. NIP-DV is strictly presentation state layered on top — a DM can be hidden without any membership change.
- **NIP-29 group membership (`kind:39002`)**: The source of the DM list a client rebuilds. NIP-DV is the missing per-viewer filter applied on top of it.
