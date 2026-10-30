# User groups

Status: draft. Wire format: [NIP-UG](nips/NIP-UG.md). Line numbers of existing code refer to base commit `5d70622`.

A user group is a named handle (`@ios-team`) with a display name, a description, and a member list. A mention of the handle notifies every member. People and agents both create, read, update, and delete groups, and add and remove members.

This gap blocks the move from Slack. Teams use user groups daily to page a team, and Buzz has no equivalent today.

## What exists today

| Piece | Where | Why it does not fit |
|-------|-------|---------------------|
| NIP-29 admin kinds 9000 to 9008, state kinds 39000 to 39003 | `crates/buzz-core/src/kind.rs:378` to `:399` (9000 to 9009), `:463` to `:471` (39000 to 39003) | They describe channels. A channel has messages, an `h` scope, and a visibility. A group has none of these. |
| Persona "teams" (`KIND_TEAM` 30176, `KIND_TEAM_CATALOG` 30178) | `kind.rs:325`, `:362` | A team groups agent personas. Its content is a JSON list of persona IDs, and the workspace owner authors it (`kind.rs:318` to `:325`). It has no member list of people and no mention path. The word "team" already means this in Buzz, so this design says "user group". |
| NIP-51 follow sets (`KIND_FOLLOW_SET` 30000) | `kind.rs:39` | A follow set is per person. Only the author can edit it. It has no community-wide handle. |
| A reserved, empty range "User groups (47000–47999)" | `kind.rs:636` (removed by this change) | The range is for regular events. Group state must be addressable, so it needs a kind in 30000 to 39999. |
| `p`-tag mentions | `crates/buzz-sdk/src/mentions.rs:38`, `builders.rs:213` (`mention_tags`) | The SDK caps a message at 50 mention tags. |
| Mention index | `crates/buzz-db/src/runtime/mod.rs:56`, `store/feed.rs:86` to `:108` | The relay writes one `event_mentions` row for each `p` tag. The mentions feed reads that table for chat kinds only. |
| Push matching | `crates/buzz-relay/src/push_runtime.rs:312` to `:340` | A lease is a set of client filters, usually `#p` on the person. A "hellthread" gate drops an event that has more than `p_tags_max` `p` tags. Mobile sets `p_tags_max` to 20 (`mobile/lib/shared/push/push_subscription.dart:13`). |
| Agent triggers | `crates/buzz-acp/src/relay.rs:3451` to `:3478` | The agent harness subscribes with `#p` on the agent when a mention is required. |
| Desktop mention contract (block/buzz-app) | `src/bundled/mentions/README.md` | A recipient is a `["p", key]` tag. A list has at most 32 keys. Every recipient is a channel member when the message is written. |
| CLI `users` and `channels` | `crates/buzz-cli/src/lib.rs:996`, `:623` | `users get` reads profiles. `channels` manages NIP-29 channels. Neither has a group of people. |
| Closest precedent for a new addressable kind | `docs/nips/NIP-CE.md` and `kind.rs` on branch `bmcanally/emoji-relay-kind` (kind 30180) | Ingest-only validation, no tables. This is the pattern this change follows. |

## Decisions

### 1. The event model: one addressable event for each group

`kind:30181`, `d` is the handle, members are `p` tags, and the signer is a community member. The relay needs no new table and no migration. Storage, queries, `#p` lookup ("which groups am I in"), and live fan-out all use the generic addressable path.

Alternatives that I rejected:

- **NIP-29 kinds.** A group would become a channel with no messages. It would inherit channel visibility, join requests, and `h` scoping, and none of those apply.
- **A relay-signed projection with command events** (the NIP-29 pattern: members send 9xxx commands, the relay signs 39xxx state). This gives unique handles and multi-admin editing by design. It also needs five command kinds, a command executor, and tables. Each new table needs a community write fence (`docs/database.md:79`). I recommend it if the community needs audit history of who changed what. It is a larger second step. The wire format here does not block it.
- **NIP-51 follow sets.** They cannot be edited by anyone but their author.

### 2. Who may edit: a community-wide head

NIP-33 replaces per author (`crates/buzz-db/src/store/replaceable.rs:72`). Slack lets several people manage one group, and a Slack migration needs that: the second person who runs the sync must be able to update the group the first person created.

So the relay treats `(kind, d)` as one community-wide coordinate. It checks who may edit with the policy in `buzz_core::user_group::authorize_write`: the creator, listed admins, community admins (`relay_members` role, as in `moderation_authz.rs`), or anyone when the group is `open`. A `creator` tag keeps the original owner stable when another author writes the head.

The cost is a deviation from NIP-33: the relay removes the previous head of another author after it stores the new one. The removal is idempotent and runs again on a retried write. Two racing edits can leave two heads for a short time, and readers pick the newest. Plain per-author NIP-33 would be simpler but would make the group editable by its creator only.

Deleting a head by someone other than its author is not supported yet. A person who is not the author sets `status: disabled`. This matches how Slack disables a group.

### 3. How a mention resolves

There are two options.

**A. The sender expands the group to `p` tags.** Every receive path already works: live `#p` filters on desktop and mobile, push leases, the `event_mentions` index, the mentions feed, and agent wake-ups. No client needs a new subscription.

**B. The sender writes a group tag and the relay or client expands it.** The message stays small and the member list is always current. But every receive path would need to learn the tag. Desktop and mobile evaluate `p` tags on the event on the device. The iOS notification extension re-checks the lease filter against the event's tags (`mobile/ios/BuzzPushKit/Sources/BuzzPushKit/PushLease.swift:125`). Agents filter on `#p`. A relay cannot change a signed event, so the relay would need a synthetic mention for each of those paths. That is more work and more risk than A.

I chose **A**, with three changes so that a group is not blocked by limits meant for individual mentions:

1. The `p` tag carries a marker in its fourth element: `["p", <member>, "", "group:ios-team"]`. This follows the existing `owner` marker (`crates/buzz-core/src/filter.rs:11` to `:19`).
2. The relay checks the marker. It rejects a message when a marked pubkey is not a member of an active group with that handle (`validate_group_mentions`). Without this check, the marker would let anyone avoid the hellthread gate.
3. The hellthread gate counts one mention for each group, not one for each member (`weighted_p_count`). Without this, a group of more than 20 people would never push to mobile.

Trade-offs of A:

- The mention is a snapshot of the members at send time. Slack does the same.
- A message with a 500-member group carries about 50 KB of tags. The group member limit is 500 for this reason. A larger audience should use a channel.
- Each composer must expand the group. The SDK does this once (`group_mention_tags`), and the CLI and agents use it. Desktop and mobile must port it. This is the one place where "each client re-implements it" remains. It is a small function, and the relay check means a wrong port fails loudly instead of silently.

### 4. Handle rules

A handle is lowercase `[a-z0-9._-]`, up to 64 bytes. `channel`, `here`, `everyone`, and `all` are reserved. The relay refuses a handle that another author holds unless the signer may edit it.

## What this change implements in block/buzz

| Piece | Status |
|-------|--------|
| `KIND_USER_GROUP` 30181 and `buzz_core::user_group` (parse, validate, authorization policy, mention helpers) | done, unit tested |
| SDK: `build_user_group`, `build_user_group_deletion`, `group_mention_tags` | done, unit tested |
| Relay ingest: `UsersWrite` scope, global-only kind, structural validation, handle uniqueness, edit authorization, newer-than-head rule, removal of superseded heads (`handlers/ingest.rs`) | done; Postgres test written, not run |
| Relay ingest: group-marked `p` tags checked against the group (`validate_group_mention_tags`) | done; Postgres test written, not run |
| Push matcher: hellthread gate counts a group once (`push_runtime.rs`) | done, unit tested |
| CLI: `buzz groups list`, `get`, `members`, `create`, `update`, `add-member`, `remove-member`, `delete`, and `messages send --mention-group` | done, tested against a fake relay |

## Testing

The relay changes that read the database need Postgres. The unit tests of the parser, the authorization policy, the mention check, and the hellthread weight run without it. The Postgres tests (`#[ignore = "requires Postgres"]`) cover the production ingest path. They compile in CI but could not run in the environment that wrote them. Run `./scripts/postgres-test-run.sh` before merge.

## What is left

Relay:

- **One transaction for the handle check and the write.** Today the relay reads the heads, decides, stores, and removes the old head in separate steps. Two creators of a new handle in the same instant can both succeed. Readers still agree on one head. Fix with an advisory lock on `(community, kind, d)` in `buzz-db`.
- **Delete by an editor who is not the author.**
- **Search.** Add `30181` to the full-text index so a client can search names. Today a client lists all heads (`limit: 500`) and filters locally.
- **A bound on groups for each community**, and a rate limit on edits.
- **Membership check of members** against `relay_members` on write.

Desktop (block/buzz-app, `src/bundled/mentions`) and `desktop/`:

- Subscribe to `kinds: [30181]` and keep the newest head for each handle.
- Offer groups in the `@` picker next to people, agents, and teams. Show the group name, handle, and member count.
- On send, expand the chosen group to `["p", key, "", "group:<handle>"]` tags. Do not apply the 32-recipient list cap or the "every recipient is a channel member" rule to those tags. Ask the sender before mentioning members who are outside the channel, as for other mentions.
- Render a message that carries a marker as one `@handle` chip, not N people.
- Settings UI to create, edit, add and remove members, disable, and delete a group.
- Update the mention contract in `src/bundled/mentions/README.md` and its fixtures.

Mobile (`mobile/`):

- The same picker, expansion, and chip rendering.
- Count a group once in the hellthread check (`push_subscription.dart`, `BuzzPushKit/PushLease.swift:125`). The relay already does this.
- Add kind 30181 to `lib/shared/relay/nostr_models.dart` and `desktop/src/shared/constants/kinds.ts`.

Product:

- "Add a group to a channel": a client convenience that sends one put-user event for each member.
- A group as a default member set for new channels.
- Open question for the Buzz team: should the creator policy default to `open`, as Slack does, or `admins`? This change defaults to `admins`.
- Open question: confirm that 30181 is free. Branch `bmcanally/emoji-relay-kind` uses 30180.
