# Solo desktop/mobile contract

The current Solo producer is **[block/buzz-app](https://github.com/block/buzz-app)**,
not the older `desktop/` client retained in this repository.

The captured producer revision is
[`64726576ed3b082daa5786e5e7ca6b5a018663a1`](https://github.com/block/buzz-app/commit/64726576ed3b082daa5786e5e7ca6b5a018663a1):

- [`SoloDraft.tsx`](https://github.com/block/buzz-app/blob/64726576ed3b082daa5786e5e7ca6b5a018663a1/src/features/solo/SoloDraft.tsx) calls `session.channelWrites.create`.
- [`channel-writes.ts`](https://github.com/block/buzz-app/blob/64726576ed3b082daa5786e5e7ca6b5a018663a1/src/features/relay/channel-writes.ts) emits kind 9007 with `channel_type=stream`, `visibility=private`, and `t=buzz.solo`. Its existing production-service test is “creates a Solo chat in its single durable outbox command” in `channel-writes.test.ts`.
- [`model.ts`](https://github.com/block/buzz-app/blob/64726576ed3b082daa5786e5e7ca6b5a018663a1/src/features/channel-templates/model.ts) retains project assignments but drops the legacy `groups.channels` discovery list.

The desktop does **not** author `P`. The relay derives the creator from the
signed creation event. In this repository, `channel_labels_from_tags` in
`crates/buzz-relay/src/handlers/side_effects.rs` parses creation labels;
`group_state_identity_tags` in `crates/buzz-core/src/channel.rs` emits the
leading type `t`, subsequent label `t` tags, and uppercase creator `P`.
`emit_group_discovery_events` uses that builder for relay metadata. Mobile reads those
facts through `ChannelData.fromEvent` and its channel providers.

## Reproducible contract coverage

`test-fixtures/solo-creation.json` contains an **actual captured production
create command**, plus expected relay identity tags using a synthetic creator.
To recapture it from a checkout of the pinned desktop revision:

```sh
node --experimental-transform-types scripts/contracts/capture-solo-creation.mjs /path/to/buzz-app > test-fixtures/solo-creation.json
```

The script invokes `createChannelWrites(...).create(...)` and captures its
outbox command without publishing anything. The core test
`desktop_solo_creation_preserves_mobile_identity_contract` feeds those command
labels through the production parser and relay identity builder. The mobile
regression “refresh replaces stale Solo details before opening and sending”
consumes the same identity tags through the real list/detail providers, primes
old details, refreshes, opens Solo, and sends through the personal sender with
an empty legacy chat list. Existing widget coverage checks project and unfiled
placement and shared-chat ordinary sending.

These are compositional contract tests, not a live desktop/relay/device E2E
run. Physical-device installation and launch alone do not prove that journey.

## Changing an existing member's access

A Solo chat owner changes an existing member's access with a signed Buzz
kind:9050 event with `admin:channels` scope. The content is empty. Each
required tag appears once.

Relays advertise `buzz-solo-member-role` in NIP-11 `supported_extensions`.
Clients offer role changes only when the relay advertises this extension.

| Tag | Value |
| --- | --- |
| `h` | Solo channel UUID |
| `p` | Target member's hex public key |
| `expected_role` | Exact active role from the relay-signed roster: `admin`, `bot`, `member`, or `guest` |
| `role` | `admin`, `member` (Participating), or `guest` (Viewing only) |

A chat owner may lower any active non-owner to Viewing only, or lower an
Admin to Participating. A move from Viewing only to Participating requires
a verified agent. An Admin grant requires a verified agent owned by the
signer. The relay stores Participating as `bot` for verified agents and
`member` for people. Role changes bypass invitation policy.

The relay requires a private Solo stream and checks the current chat and
membership under the membership lock. It commits the command, role, signed
admin list, and signed member roster together. It emits no admission notices.

Refresh the roster after a role mismatch. Resending the same accepted event
confirms its receipt without applying another change. Relays that lack
kind:9050 support reject it. Clients must never fall back to kind:9000 for
this action.
