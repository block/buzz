# Sprout Serverless Mode

> **Status:** Implemented on branch `micn/serverless-mode`.
> Channels, DMs, messages, **agents** (with npub-allowlist permissions), and
> **end-to-end-encrypted private channels + DMs** (NIP-17) all work against any
> generic public Nostr relay — single or multiple, for redundancy — with
> **zero** Sprout server infrastructure. See "Implementation" below.

## TL;DR for testing

1. Build & run the desktop app (`just dev`).
2. On the welcome screen (or **+ Add workspace**), tick **Serverless mode** and
   enter a public relay, e.g. `wss://relay.damus.io` or `wss://nos.lol`.
3. Create a channel, send messages, open a DM. All of it round-trips through
   the public relay over plain WebSocket — no Postgres, no `/query`, no auth
   server.

Server-only surfaces (search, pulse, projects, workflows) are hidden in this
mode. Agents are **not yet** functional serverless (see below).

---

# Sprout Lite — Serverless Relay Mode

A design note for a "no-server" mode in Sprout, modeled on `../slackest`:
point the desktop client directly at public (or named) Nostr relays, show
channels and DMs, and run with **zero Sprout server infrastructure** — no
`sprout-relay`, no Postgres, no Redis, no Typesense, no auth/membership.

---

## The two worlds today

> **Note:** The sections from here down to "Implementation (what shipped)" are
> the original design exploration, including the **rejected** NIP-28/Slackest
> approach. What actually shipped is **Option A** (same app, same native Sprout
> kinds, plain-WS transport) — jump to **"Implementation (what shipped on
> `micn/serverless-mode`)"** for the authoritative description.

### Sprout (server mode — what exists now)

The desktop client is *thin*. It assumes a smart server (`sprout-relay`)
that owns most of the logic:

| Concern | How Sprout does it today |
|---|---|
| Transport | Native WebSocket via the **Tauri Rust backend** (`invoke`, `Channel`), not a browser socket |
| Auth | **NIP-42 AUTH** handshake on every connect (`relayClientSession.ts`); relay rejects unauthed sessions |
| Channels | **NIP-29 groups** — scoped by `h` tags, membership enforced server-side (`migrations/0001_relay_members.sql`, `sprout-auth`) |
| Queries | Relay **p-gate**: a `REQ` with no `kinds` returns 403 (see AGENTS.md gotchas) |
| Messages | Custom kinds: `9`/`40002` stream messages, `45001`/`45003` forum (`sprout-core/src/kind.rs`, `desktop/.../kinds.ts`) |
| Search | Server-side Typesense (`sprout-search`), via NIP-50 `search` filters |
| Threads | `reply_count`/`descendant_count` **materialized in Postgres** by the relay |
| Presence/typing | Redis pub/sub fan-out (`sprout-pubsub`) |
| DMs | Routed through relay membership/auth |

So "Sprout" = a specific opinionated relay + a client that depends on it.

### Slackest (the serverless reference)

`../slackest` is ~1,200 lines of vanilla TS that talks to **any** relay:

| Concern | How Slackest does it |
|---|---|
| Transport | `nostr-tools` `SimplePool` directly in the page (browser WS) |
| Auth | None — local `nsec`, sign-and-publish |
| Channels | **NIP-28** (`kind 40` create, `kind 42` message, `e`-tag root) |
| Queries | Plain relay `REQ` with `kinds` — works on any public relay |
| Search | Client-side only |
| DMs | **NIP-04** (`kind 4`, encrypted) |
| Identity | Generate or import `nsec`, persisted in Tauri store / `localStorage` |
| State | All client-side: joined channels, DM contacts, relays, profiles |

No server. The relay is a dumb event store. State lives in the client and in
whatever public relays you point at.

---

## What "Sprout Lite" means

A mode where Sprout behaves like Slackest: **the relay is just a public Nostr
relay**, all coordination logic moves to (or is skipped in) the client, and
none of the `sprout-*` server crates are involved.

The hard truth up front: **Sprout's current channel model (NIP-29 + AUTH +
server membership + custom kinds) does not work on a dumb public relay.** Lite
mode therefore needs a *second* protocol profile based on open NIPs (NIP-28
channels, NIP-04/NIP-17 DMs, `kind 0` profiles) — exactly Slackest's model.

So this is less "flip a flag" and more "add a relay adapter + protocol profile
behind the existing client UI."

---

## Recommended approach: Adapter behind the Workspace abstraction

Sprout already has a **workspace** concept (`features/workspaces/`) where each
workspace = a relay URL + identity. That's the natural seam. Add a workspace
*kind*:

```ts
// features/workspaces/types.ts
type WorkspaceMode = "sprout" | "lite";

type Workspace = {
  id: string;
  name: string;
  relayUrl: string;       // lite: one of several public relays
  relays?: string[];      // lite: a relay set (SimplePool)
  pubkey: string;
  mode: WorkspaceMode;    // NEW
  addedAt: string;
};
```

Then introduce a **client interface** that both modes implement, so the React
feature hooks don't care which world they're in:

```ts
interface SproutClient {
  connect(): Promise<void>;
  disconnect(): void;
  publish(event: UnsignedEvent): Promise<RelayEvent>;
  subscribe(filters: RelaySubscriptionFilter[], onEvent): Subscription;
  queryHistory(filters): Promise<RelayEvent[]>;
  connectionState: ConnectionState;
}
```

- `RelayClient` (existing, `relayClientSession.ts`) → the **sprout** impl
  (Tauri WS + NIP-42 + NIP-29).
- `LiteRelayClient` (new) → the **lite** impl, a thin wrapper over
  `nostr-tools` `SimplePool`, basically Slackest's `nostr.ts` adapted to the
  `SproutClient` interface (no AUTH, no `h` tags, `kinds`-only REQs).

`useWorkspaceInit.ts` picks the implementation based on `workspace.mode` and
stashes it in the same singleton slot the app already uses. Because the app
already key-remounts on workspace switch and has a `resetWorkspaceState()`
contract, swapping client implementations per workspace is well-supported —
the new lite client just needs its own reset hook registered there.

### Why an adapter and not a fork

- Reuses the entire desktop UI (sidebar, message list, composer, modals,
  themes) — no second app to maintain.
- Reuses workspace switching, drafts, profile caches.
- Keeps the door open to a workspace list that mixes a corporate Sprout relay
  *and* public Nostr channels side by side.

---

## Protocol profile for Lite mode

Lite mode speaks **open NIPs only** (same as Slackest):

| Feature | Lite kind / NIP | Sprout kind (for contrast) |
|---|---|---|
| Profile | `0` (NIP-01 metadata) | `0` |
| Channel create | `40` (NIP-28) | `39000` channel metadata + NIP-29 group |
| Channel message | `42` (NIP-28), `e`-tag root | `9` / `40002`, `h`-tag |
| DM | `4` (NIP-04) or `1059`+`14` (NIP-17 gift wrap) | server-routed |
| Reaction | `7` (NIP-25) | `7` |
| Profile lookup | `0` subscription by author | server-side |

A small **kind-mapping layer** lets the existing message components render
either profile. The renderer cares about `{author, text, createdAt, roomKey}`
— the lite adapter normalizes NIP-28 events into that shape, just like
`ingestMessage` does in Slackest's `main.ts`.

Channel identity in lite mode is the **kind-40 event id**, not a name (per
Slackest's README) — so the sidebar shows display labels but joins/dedupes on
the event id, and the "add channel" modal accepts a pasted channel id.

---

## What you gain and what you lose

**Gain**
- Run Sprout against `wss://relay.damus.io`, `wss://nos.lol`, etc. with no
  backend at all.
- Zero infra to stand up for demos, personal use, or interop testing.
- **Multi-relay redundancy** — a serverless workspace holds a *list* of relays;
  publishes fan out to all, reads merge + dedup, survive any single relay
  outage.
- **Agents work** — same respond-to / npub-allowlist permission model as the
  server, enforced in the `sprout-acp` harness (see below).
- **Private channels + DMs are end-to-end encrypted** (NIP-17), so privacy
  doesn't depend on a trusted server.

**Lose (vs. server mode)**
- No server-side **access control on public channels** — an open channel is
  open to anyone (which is the point). Privacy for closed groups is provided by
  *encryption* instead (NIP-17), not relay enforcement.
- No server-side search (client-side only, over what you've fetched).
- No materialized thread counts, presence fan-out, or read-state sync across
  devices (could be reintroduced later via NIP-29-capable public relays or
  client-side computation).
- No workflows / huddle audio / git hosting — those are `sprout-relay` features
  with no dumb-relay equivalent. Serverless mode **hides** these surfaces
  rather than breaking on them.

---

## Implementation sketch (incremental, low-risk)

1. **Add `mode` to `Workspace`** and default everything existing to `"sprout"`
   (`workspaceStorage.ts` migration — mirror the existing `nsec`-strip
   migration pattern).
2. **Vendor `nostr-tools`** into `desktop/package.json` (it's already used in
   slackest; Sprout doesn't ship it client-side today).
3. **Add `LiteRelayClient`** in `shared/api/` implementing the `SproutClient`
   interface over `SimplePool`. Port the publish/subscribe/DM-decrypt logic
   from `../slackest/src/nostr.ts`.
4. **Extract a `SproutClient` interface** and make `RelayClient` conform; wire
   client selection in `useWorkspaceInit.ts` by `workspace.mode`.
5. **Add a kind-mapping/normalization layer** so message + channel hooks
   consume a profile-agnostic shape.
6. **Add an "Add public workspace" flow** in onboarding/workspace UI: enter a
   relay set, generate or import `nsec`, pick/create NIP-28 channels.
7. **Feature-gate server-only surfaces** (agents, workflows, huddle, search-
   server, presence) behind `mode === "sprout"`.
8. **Register the lite client's reset** in `resetWorkspaceState()`.

Steps 1–3 are independently shippable and unlock a Slackest-equivalent in a
single new workspace, without touching the server-mode path at all.

---

## Quick prototype option

If the goal is just to *see it working* fast, the lowest-effort path is to
keep `../slackest` as a separate tiny app and treat it as the lite reference
client — it already does exactly this. The doc above is the path to folding
that capability **into** the Sprout desktop app so there's one client with a
mode switch, rather than two apps.

---

## Implementation (what shipped on `micn/serverless-mode`)

We took **Option A**: same app, same native event kinds (39000 channel
metadata, 39002 membership, kind 9 messages, `h`-tag scoping), just pointed at
a generic relay. "Serverless" is a transport + auth concern, not a different
protocol. Nothing on the existing Sprout-server path changed.

### Rust backend (`desktop/src-tauri`)

- **`AppState.serverless`** (atomic flag) + `AppState::is_serverless()`. Set by
  `apply_workspace(relay_url, nsec, serverless, …)`.
- **`ws_relay.rs`** — `query_relay_ws` (REQ → collect until EOSE → CLOSE) and
  `submit_event_ws` (EVENT → wait for OK). NIP-42 AUTH is answered only if the
  relay challenges; timeouts degrade to best-effort rather than hard-failing.
- **`relay.rs`** — `query_relay` / `submit_event` now branch: serverless →
  `ws_relay::*` (plain WS), otherwise → the existing HTTP bridge (`/query`,
  `/events` + NIP-98). Every caller (channels, DMs, messages) is unchanged.
- **`events.rs`** — `build_channel_metadata_serverless` (kind 39000) and
  `build_channel_members_serverless` (kind 39002). In server mode the relay
  materialises these from command kinds (9007 create, 41010 dm-open); on a
  dumb relay the client publishes them directly.
- **`commands/channels.rs` `create_channel`** and **`commands/dms.rs`
  `open_dm`** branch on `is_serverless()`: publish 39000 + 39002 directly
  instead of a server command. DM channel ids are derived deterministically
  (UUIDv5 over the sorted participant set) so both sides converge without a
  server assigning one.

### Frontend (`desktop/src`)

- **`Workspace.mode: "sprout" | "serverless"`** (+ `workspaceMode()` /
  `isServerlessWorkspace()` helpers; legacy entries default to `sprout`).
- **`relayClientSession.ts`** — `setServerless(true)` makes `connect()` resolve
  as soon as the socket opens (no blocking on a NIP-42 challenge). Late
  challenges are still signed so writes succeed on relays that require auth.
- **`useWorkspaceInit.ts`** — calls `relayClient.setServerless()` and passes the
  flag to `applyWorkspace()` before AppShell preconnects.
- **`ServerlessContext`** + `useIsServerless()` — feature-gates **search**
  (Typesense), **pulse**, **projects** (git hosting), and **workflows** in the
  UI. They degrade to hidden, not broken.
- **Add-workspace + welcome** UIs gained a **Serverless mode** toggle with a
  public-relay default (`wss://relay.damus.io`).

### Multi-relay (redundancy)

A serverless workspace's `relayUrl` may be a **comma-separated list** of relays
(seeded in the UI with the public defaults below). The Rust transport fans out:
`submit_event_ws` publishes to all relays (succeeds if any accepts);
`query_relay_ws` queries all concurrently and **merges + dedups events by id**
(succeeds if any responds). The live WebSocket connects to the first (primary)
relay. A single dead relay therefore doesn't break reads, writes, or history.

Default public relays (`desktop/src/features/workspaces/defaultRelays.ts`,
sourced from `deez`): `relay.damus.io`, `nos.lol`, `relay.nostr.band`,
`nostr.land`, `nostr.wine`.

### Agents (same permission model as the server)

Agents work in serverless mode with the **same npub-allowlist / respond-to
gate** as the Sprout server. The gate lives in the `sprout-acp` **harness**, not
the agent, and runs *before* any event reaches the agent subprocess — so an
agent in a public channel only ever responds to blessed npubs, exactly as
configured in the GUI (owner-only / allowlist / anyone / nobody). This is
unchanged from server mode; serverless only swaps the transport:

- `sprout-acp`'s `RestClient` / `HarnessRelay` got a serverless mode that
  swaps the HTTP bridge (`/query`, `/events` + NIP-98) for plain-WS REQ/EVENT.
  Plumbed via `SPROUT_SERVERLESS`, set by the desktop when launching the agent.
- `sprout-cli` (what agents shell out to for writes) got the same WS transport
  and a `--serverless` flag.
- Attaching an agent publishes its pubkey into the channel's kind-39002 member
  list (so in an **encrypted** channel the sender wraps a copy to the agent and
  it can decrypt + respond).

### Encrypted private channels + DMs (NIP-17)

On a dumb relay, "private" can't mean server-enforced access — every stored
event is world-readable. So in serverless mode, **DMs and `private`-visibility
channels are made private by encryption** (NIP-17 / NIP-59 gift wrap) instead:

```text
message (kind 9 rumor, with the channel `h` tag)
  → seal      (kind 13, nip44-encrypted to one recipient)
    → gift wrap (kind 1059, ephemeral key, `#p` = recipient, random timestamp)
```

One gift wrap is published **per member** (including the sender, so it can read
back its own messages). The relay only ever stores opaque `kind 1059` blobs
addressed by `#p` — it never sees the channel id, content, or real author. On
read, the client subscribes to `kind 1059 #p=me`, decrypts each in Rust
(`crate::encrypted` + the `decrypt_gift_wrap` command), and routes the recovered
kind-9 rumor (which carries the `h` tag) into the normal message pipeline. The
agent harness does the symmetric thing: it subscribes to its own gift-wrap
inbox, unwraps, and feeds the inner event through the respond-to gate unchanged.

Open/public channels stay **plaintext kind 9** (that's the point of a public
channel). Encryption applies only to DMs and private channels.

This is the **small-group** model: O(N) gift wraps per message (which naturally
caps practical group size), no shared group key, and no forward secrecy on
member removal (a removed member keeps any messages they already received).
Suitable for small trusted groups; a shared-key / MLS scheme would be required
for large groups or forward-secrecy guarantees.

Files: `desktop/src-tauri/src/encrypted.rs` (crypto + tests),
`commands/encrypted.rs` (`decrypt_gift_wrap`), `commands/messages.rs`
(`encrypted_recipients` + `send_encrypted_message` routing),
`shared/api/relayClientSession.ts` (encrypted history/live read),
`crates/sprout-acp/src/relay.rs` (agent gift-wrap inbox).

### What works serverless today

Channels (create/list/join via 39000+39002), channel messages (kind 9, live +
history), DMs and private channels (NIP-17 encrypted), profiles (kind 0),
reactions/edits/deletes (standard kinds), agents (npub-allowlist gated),
multi-relay redundancy — all over plain WS against any public relay, with no
Sprout server.
