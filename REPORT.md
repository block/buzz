# Serverless Mode — Design Report

**Branch:** `micn/serverless-mode`
**Status:** feature-complete, verified end-to-end against live public relays.

This report explains what serverless mode is, the design decisions behind it,
how each piece works, and the evidence that it works end-to-end without
affecting the existing server-bound ("sprout relay") behaviour.

---

## 1. What it is

Serverless mode (a.k.a. "sprout-lite") lets the Sprout desktop app and the agent
harness talk **directly to generic public Nostr relays** with **no Sprout
infrastructure** — no `sprout-relay`, no Postgres, no Redis, no Typesense, no
auth/membership server. It is a **degraded subset** of Sprout: channels, DMs,
private channels, messages, threading, and agents — all expressed as plain Nostr
events over standard relays.

A workspace is either:

- **`sprout` mode** — talks to a Sprout relay via the HTTP bridge (`/query`,
  `/events`) + NIP-98 auth + NIP-42 AUTH WebSocket. (Unchanged, the default.)
- **`serverless` mode** — talks to a comma-separated **list** of public relays
  over plain WebSocket (NIP-01 `REQ`/`EVENT`/`EOSE`/`OK`). No AUTH required.

Default relays when you enable serverless:
`wss://relay.damus.io, wss://nos.lol, wss://relay.nostr.band`.

---

## 2. Guiding design decisions

### 2.1 "It's still relays at the end of the day"

The core insight that shaped everything: **serverless is the same product with a
different transport.** The agent's brain (respond gate, threading, prompt
rendering, reply mechanism) and the desktop's feature surface are shared code.
Only the bottom transport layer swaps:

| Concern | `sprout` mode | `serverless` mode |
|---|---|---|
| Respond gate (owner-only default, allowlist, siblings) | shared | **shared** |
| Threading decision (NIP-10) | shared | **shared** |
| Prompt rendering / reply via `sprout` CLI | shared | **shared** |
| **Transport** | HTTP bridge + NIP-98 + NIP-42 | plain WS, multi-relay |
| Channel discovery | one relay (HTTP) | multi-relay union |
| Reply / message publish | one relay | multi-relay, first-accepts |
| Membership push (kind 44100) | relay side-effect | periodic re-discovery |

Everything new is **gated** behind `is_serverless()` / `if serverless`. Server
mode runs the exact same code paths it always has.

### 2.2 The standard Nostr client model (no exotic tricks)

Relays **do not gossip** — an event stored on relay B is invisible to a
subscriber on relay A. Every production Nostr client therefore:

1. **publishes to many** relays (succeed if any accepts), and
2. **subscribes to many** relays and **dedups by event id**.

This is exactly what damus (`RelayPool.swift`), nostr-tools (`SimplePool`), and
0xchat do. Serverless mode follows the same model. There is nothing bespoke.

### 2.3 Model the operations as Nostr events, not new endpoints

Per the project's "prefer Nostr events over new REST endpoints" rule, serverless
membership/channel operations are done by **writing the same addressable events**
the relay would have produced (kinds 39000 / 39002), rather than inventing new
flows. The server's command kinds (9007 create-channel, 9021 join, 44100
member-added, …) are **server side-effects** that a dumb relay never executes, so
serverless writes the resulting state events directly.

### 2.4 `nostr-relay-pool` for the agent, hand-rolled pool for the desktop

The **agent** (`sprout-acp`, a standalone binary) uses the official
`nostr-relay-pool` crate (rust-nostr SDK) for its relay layer — free
auto-reconnect, auto-resubscribe, and dedup. The **desktop** deliberately keeps
its own hand-rolled connection pool (`ws_pool.rs`) to avoid pulling a *second*
TLS/crypto stack into the Tauri process (which already runs aws-lc-rs rustls for
reqwest/media). Two rustls providers in one process risk a `CryptoProvider`
install panic. The desktop pool is serverless-only and already self-heals, so the
agent's old missed-message bug does not apply there. This asymmetry is documented
in `ws_pool.rs`.

---

## 3. How it works, piece by piece

### 3.1 Event kinds used

All standard Sprout kinds (defined in `sprout-core/src/kind.rs`):

- **39000** — channel metadata (addressable; `d` = channel id, `name`, `t`,
  `public`/`private`).
- **39002** — channel members (addressable; `p` tags, role as the 4th tag
  element: `["p", <pubkey>, "", "owner"|"member"]`).
- **9** (`KIND_STREAM_MESSAGE`) — a channel message, scoped by the `h` tag
  (NIP-29 group tag).
- **1059** — NIP-59 gift wrap (encrypted DMs / private channels).
- **20002** — typing indicator (ephemeral).
- **5** — NIP-09 deletion (used for channel delete, since the dumb relay won't
  process a delete command).

### 3.2 Channels & membership (desktop)

- **Create** (`commands/channels.rs`): publish 39000 (metadata) + 39002 (members,
  creator recorded as `owner`) directly to all relays. The `ChannelInfo` is built
  **locally** and returned immediately — no re-query race (this fixed the early
  "nothing happened" bug).
- **Join / leave / add / remove**: read the current 39002, modify the `p` list
  (preserving roles), and re-publish (read-modify-write). NIP-29 command kinds are
  no-ops on a dumb relay, so direct state writes are the only thing that works.
- **`get_channels`**: in serverless, skips the server's `limit:5000` network
  discovery (which pulled hundreds of unrelated public channels) and scopes to the
  user's own membership.
- **Critical fix:** nostr 0.44 strips self-`p` tags unless
  `.allow_self_tagging()` is set. Without it, the creator's own membership tag was
  dropped, so `get_channels` (`#p:[me]`) found nothing. All serverless membership
  builders set `.allow_self_tagging()`.

### 3.3 DMs & private channels (encryption)

- **DMs**: `open_dm` derives a deterministic UUIDv5 channel id from the two
  pubkeys (so both sides converge on the same id), then publishes 39000 + 39002.
- **Encryption (NIP-17 / NIP-59)**: a DM or private channel message is built as a
  normal kind-9 **rumor** (unsigned, carries the `h` tag), then **gift-wrapped
  (kind 1059) once per recipient** — every channel member plus self. Only the
  gift wraps hit the relay; the plaintext kind-9 never does. The `h` tag lives
  *inside* the encrypted rumor, so a relay query by `#h` returns nothing — the
  channel itself is hidden.
- **Threaded encrypted replies** (the privacy fix in this branch): replies in a
  private channel are **also** gift-wrapped, with the NIP-10 thread `e` tags
  placed *inside* the rumor. Previously replies fell through to the plaintext
  path and leaked the reply content to public relays. Now threading is preserved
  *and* encrypted. The thread root is resolved locally from already-decrypted
  messages (the parent rumor isn't queryable in plaintext on the relay).

### 3.4 Multi-relay transport

- **Desktop** (`ws_pool.rs` + `ws_relay.rs`): one long-lived WebSocket per relay,
  multiplexing all queries/publishes. Reads fan out and **merge + dedup by event
  id**; writes fan out and **succeed if any relay accepts** (with a retry if all
  rate-limit). Dropped connections are detected and reconnected on next use. This
  fixed the rate-limit storm (the original code opened a fresh socket per op) and
  the "message shows then disappears" split-brain (publish landed on relay B,
  live-read was on relay A).
- **Agent** (`serverless_relay.rs` wrapping `nostr-relay-pool`): a persistent
  multi-relay subscription with built-in auto-reconnect + auto-resubscribe. On
  reconnect the SDK re-sends the `REQ`, and the relay replays its stored matching
  events — so a message that arrived during a blip is recovered. Events are merged
  and deduped across relays automatically.

### 3.5 Agents (the headline capability)

Agents work the **same** as in server mode — the permission model is identical;
only the transport differs.

- **Launch**: managed agents launch with the **current workspace** relay list and
  `SPROUT_SERVERLESS=true`. A startup gate (`workspace_applied`) makes agent
  restore wait (up to 15s) for the frontend to apply the workspace, so agents
  never launch with stale relays or in the wrong mode.
- **Serverless detection**: the agent infers serverless from a comma-list relay
  URL (`SPROUT_RELAY_URL=wss://a,wss://b`) in both `sprout-acp` and the `sprout`
  CLI, so even the agent's reply (`sprout messages send`) takes the serverless
  path.
- **Discovery**: the agent finds channels it belongs to by querying 39002
  (`#p` = agent) across all relays. Because a dumb relay never emits the
  membership-added push (kind 44100), the agent **re-runs discovery every 20s** and
  subscribes to any newly-joined channels.
- **Receiving messages**:
  - *Public channels*: live `REQ` for kind 9 scoped by `#h` = channel.
  - *Private channels / DMs*: a **gift-wrap inbox** subscription (kind 1059,
    `#p` = agent). On each wrap the agent decrypts to the inner rumor, reads its
    `h` tag, and routes it to the right channel.
- **The respond gate** (unchanged from server mode): default **`owner-only`** —
  the agent only acts on messages from its **owner** (the npub that added it,
  resolved from the `SPROUT_AUTH_TAG` NIP-OA attestation), plus "siblings" (other
  agents launched by the same owner). Other modes: `allowlist`, `anyone`,
  `nobody`. The gate runs in the harness **before** the LLM sees the event.
- **Replying**: the agent posts its reply via the `sprout messages send` CLI
  (multi-relay, first-accepts). It addresses the asker by `@name` in the channel —
  it does **not** DM the owner. Threading mirrors the trigger: a reply to a
  top-level message is top-level; a reply inside a thread uses `--reply-to` and
  stays threaded.
- **Working cues**: while a turn is in flight the agent publishes a typing
  indicator (kind 20002, every 3s) and a `💬` reaction, so the UI shows "working…"
  during the (30–90s) LLM latency.

---

## 4. Bugs found and fixed on this branch

In rough order, each reproduced and then fixed (and most now covered by a test):

1. **"Join did nothing"** — membership via server command kinds is a no-op on a
   dumb relay → switched to direct 39002 read-modify-write; then `.allow_self_tagging()`.
2. **Rate-limit storm** — fresh socket per op → persistent connection pool.
3. **"Message shows then disappears"** — split-brain reads → multi-relay
   read/write/subscribe.
4. **Query flood** — `limit:5000` discovery pulled the whole public network → skip
   in serverless.
5. **Channel delete no-op** → NIP-09 kind-5 deletion.
6. **Agent DNS crash** — comma-list fed to a single connect → multi-relay connect.
7. **Agent didn't see new channels** (no 44100 push) → 20s re-discovery.
8. **Agent reconnect storm** — a paid relay's `auth-required` → drop that sub,
   don't reconnect-loop; removed paid relays from defaults.
9. **Agent launched in server mode / stale relays** → `workspace_applied` gate +
   serverless inference from comma-list.
10. **Agent reply lost to rate-limit** → multi-relay fan-out + backoff retry.
11. **Agent relay layer fragile** (missed messages on relay drop) → replaced with
    `nostr-relay-pool` (auto-reconnect/resubscribe/dedup).
12. **`💬` working-cue never landed** (500ms timeout too tight for a real WS
    round-trip) → 4s.
13. **Encrypted replies leaked plaintext** → gift-wrap replies too, NIP-10 tags
    inside the rumor.

---

## 5. Verification — evidence it works end-to-end

All run on `micn/serverless-mode` at the time of this report.

### Automated

- **Unit tests:** `sprout-acp` 272 ✅, `sprout-cli` 114 ✅, `sprout-sdk` 138 ✅,
  desktop backend 427 ✅ (all pass).
- **Clippy:** clean on every touched crate (acp, cli, sdk, desktop backend).
- **End-to-end against live public relays** (`crates/sprout-acp/tests/e2e_agent_responds.rs`,
  runs the **real** `sprout-acp` + **real** `sprout` CLI, no LLM stub):
  - `agent_responds_in_channel_e2e` — proves: receive @mention → reply #1 →
    **typing indicator (kind 20002)** during the turn → second message → reply #2
    (**cancel/redispatch** path). ✅
  - `agent_responds_in_private_channel_e2e` — proves: a **gift-wrapped (kind
    1059)** message is received, **decrypted**, routed by the inner `h` tag,
    gated, and replied to. ✅
  - Both pass reliably (run repeatedly) **even while damus returns 503** — the
    multi-relay fan-out fails over to another relay.
- **Encryption unit test** (`desktop/src-tauri/src/encrypted.rs`): a threaded
  reply in a private channel is gift-wrapped (kind 1059), the plaintext never
  appears on the wire, and the NIP-10 root/reply tags survive decryption. ✅

### Manual (live)

- Confirmed on the relays directly that a user's **private** channels show **zero
  plaintext kind-9** and only un-`h`-tagged gift wraps.
- Confirmed goose replies in **public** channels live (addresses the user by
  display name).

---

## 6. Will it break `main` (server mode)?

**No.** Risk is contained:

- **Every serverless behaviour is gated** behind `is_serverless()` (desktop) or
  `if serverless` / `Option<ServerlessRelay>` being `None` (agent). The
  `ServerlessRelay`/`nostr-relay-pool` path is only constructed inside
  `if serverless`. Server mode runs the original code paths unchanged.
- **New modules** (`ws_pool.rs`, `ws_relay.rs`, `encrypted.rs`,
  `serverless_relay.rs`, `ServerlessContext.tsx`, `defaultRelays.ts`) are reachable
  only from serverless branches.
- **The single shared, non-gated change** is `REACTION_TIMEOUT` (500ms → 4s). It
  only makes a best-effort reaction *more* tolerant; server mode's HTTP bridge
  responds well under 4s, so it cannot regress server behaviour.
- **Dependency additions** (`nostr-relay-pool`, rustls feature on the CLI) resolve
  to the same `nostr 0.44.3` already in the tree — no version conflict.
- The one failing workspace unit test (`git-sign-nostr::test_parse_envelope_rejects_invalid_oa_pubkey`)
  **also fails on `main`** — it is pre-existing and unrelated to this branch.

---

## 7. Known limitations / follow-ups

- **Public relays are unreliable stores.** They rate-limit, 503, and may not
  retain low-traffic events indefinitely. The multi-relay fan-out + auto-reconnect
  mitigates this, but serverless is inherently best-effort — appropriate for a
  "lite" mode, not a system of record.
- **Profile/display names** depend on the relays carrying kind-0 metadata; absent
  that, the agent may address users by raw npub.
- **Desktop `ws_pool.rs` is hand-rolled** (by deliberate choice, §2.4). If the
  desktop ever shows a concrete live-subscription recovery bug, that targeted path
  can be hardened without adopting a second TLS stack.
- **Server-mode integration suite** (`just test`, needs Postgres + Redis) was not
  run for this report; the serverless changes are gated so server paths are
  unaffected, but running it is the recommended final gate before merge.

---

## 8. Key files

| Area | File |
|---|---|
| Serverless detection / mode | `desktop/src-tauri/src/app_state.rs`, `commands/workspace.rs` |
| Desktop relay transport | `desktop/src-tauri/src/relay.rs`, `ws_relay.rs`, `ws_pool.rs` |
| Channels / DMs / messages | `desktop/src-tauri/src/commands/{channels,dms,messages}.rs` |
| Encryption (gift wrap) | `desktop/src-tauri/src/encrypted.rs`, `commands/encrypted.rs` |
| Agent relay (SDK pool) | `crates/sprout-acp/src/serverless_relay.rs`, `relay.rs` |
| Agent main loop / re-discovery / gate | `crates/sprout-acp/src/lib.rs` |
| Agent + CLI serverless detection | `crates/sprout-acp/src/config.rs`, `crates/sprout-cli/src/lib.rs` |
| CLI multi-relay client | `crates/sprout-cli/src/client.rs` |
| Shared event builders | `crates/sprout-sdk/src/builders.rs`, `desktop/src-tauri/src/events.rs` |
| Frontend serverless UI | `desktop/src/features/workspaces/*` |
| E2E proof | `crates/sprout-acp/tests/e2e_agent_responds.rs` |
| Deeper implementation map | `docs/SPROUT_LITE_MODE.md` |
