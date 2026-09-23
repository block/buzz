# Writing a third-party Buzz client

Buzz ships its own desktop, web and mobile clients. This note collects the
relay surfaces that are **not** obvious from the NIP specs, for anyone
building a separate client against a self-hosted relay.

Everything below is behaviour of the relay in this repository. Where a claim
rests on a specific code path, the path is cited so it can be re-checked when
the code moves.

## The bridge extension fields are REST-only, and fail silently on WS

`POST /query` accepts a standard Nostr filter **plus** extension fields. They
are extracted from the raw JSON `Value` before `nostr::Filter` deserialization,
because `Filter` silently drops unknown fields
(`crates/buzz-relay/src/api/bridge.rs:265-276`).

The WebSocket `REQ` path does **not** extract them. Unknown fields are dropped
by `nostr::Filter`'s unknown-field behaviour, so a client that sends an
extension field over WS gets a *valid but different* result — a normal
full-event query — never an error
(`docs/bridge-channel-window.md:24-26`).

| Field | Effect |
|---|---|
| `top_level: true` | Route the filter to the top-level timeline view. Requires exactly one accessible `#h`. |
| `before_id` + `until` | Composite `(created_at, id)` cursor. **Both or neither** — `top_level` with `until` and no `before_id` is rejected `400`. |
| `include_summaries`, `include_aux` | Opt-in overlay/closure appends. Never consume `limit`. |
| `feed_types` | Selects activity-feed types (below). |
| `depth_limit` | Thread depth cap. |

**Rule of thumb:** if you need any of these, use the HTTP bridge. WS is for
plain NIP-01 filters.

## The activity feed

`feed_types` on `POST /query` selects an activity feed. `agent_activity` is an
**alias** for `activity` (`crates/buzz-relay/src/api/bridge.rs:1227-1231`).

Granularity is **timeline-level, not tool-level**: stream messages (9, 40002),
forum posts, and the agent job lifecycle — request 43001, accepted 43002,
progress 43003, result 43004, error 43006
(`crates/buzz-db/src/store/feed.rs`). Workflow execution kinds (46001-46012)
are deliberately excluded to avoid noise.

Results are per-channel and permission-filtered, so a feed is only as complete
as the caller's channel access.

## Tool-level agent telemetry is owner-scoped, not channel-scoped

`kind:24200` (NIP-AO) carries genuine tool-call-granularity observer frames —
`acp_read`, `acp_write`, `turn_started`, `session_resolved`. It is in
`P_GATED_KINDS` (`crates/buzz-core/src/kind.rs:159-165`), so a subscription
must carry `#p` = the caller's own pubkey.

The practical consequence for a client: you can render a **live tool trace for
your own agents**, and for other members' agents you only ever get the coarse
activity feed. It is ephemeral — never stored, no replay.

## Presence and typing

- `kind:20001` **presence** is a channel-less ephemeral event, per-identity
  per-community (`crates/buzz-relay/src/handlers/event.rs:844-847`). The WS
  path accepts arbitrary status strings for forward-compatibility; the
  REST/MCP surfaces use the curated `online` / `away` / `offline` enum
  (`crates/buzz-core/src/presence.rs:5-18`).
- `kind:20002` **typing** carries a channel (`h` tag) and takes the
  membership-checked ephemeral path
  (`crates/buzz-relay/src/handlers/event.rs:849-874`).

Two doc-vs-code gaps worth knowing before you plan around them: as of this
writing `ARCHITECTURE.md` describes a `buzz:typing` sorted set and an
`/api/presence` endpoint, and neither exists in the relay source.

## Group discovery is pull, not push

`39000` / `39001` / `39002` are stored **channel-scoped** so that the existing
access control applies — a private channel's member list is only visible to its
members. The consequence is that live global subscriptions (for example
`{kinds:[39000]}`) receive nothing through fan-out
(`crates/buzz-relay/src/handlers/side_effects.rs:1122-1129`).

Discover groups with a **historical `REQ` on connect** and poll for changes.
The source comment notes live push for open-channel discovery as a future
enhancement, so do not build a client that assumes it.

## Read state belongs to the client

`kind:30078` carries a blob encrypted to self. The relay stores it as
transport; it does not compute unread counts or read frontiers, and NIP-RS is
explicitly not a read-receipt protocol. Merge it client-side.

## Permissions you can display honestly

- The **message write gate** is channel membership, or open visibility for
  non-members, plus a community-scoped ban (`9040`) / timeout (`9042`). There
  is no per-identity, per-room write permission.
- `kind:39002` carries the member role as the fourth `p`-tag element. It is
  **not** consulted on the message write path — a `guest` can post if the
  channel allows it.
- Role *is* enforced for **git push**: the permission model is
  "channel role = repo role" (`crates/buzz-core/src/git_perms.rs`), and a repo
  must be bound to a channel to be pushable at all.

So a client can honestly show role and repo-write authority as enforced, and
must not present any per-room chat permission as enforced when it is not.
