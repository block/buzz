# Buzz bridge protocol v1 (slice A — relay bridge)

Status: v1. Implemented: key wrap, agent snapshot, heartbeat, transcript chunks (read-only), commands agent.start / agent.stop / agent.message with ack/result. Not implemented: prompt.approve/deny (the desktop has no interactive permission prompts; reported as `unsupported`), command.run, file.get, snapshot.<topic>. Everything else below is specified here so slices A-rest, B and C reuse one envelope. See `pwa-plan.md` §11.

## 1. Goals / non-goals
- A browser signed in as member M can see (and later control) the desktop client that M runs, through the Buzz relay, with **no new relay code, ports or server state**.
- No plaintext on the relay. Only M's own key (or the bridge session key derived from it, §3) can read payloads.
- Minimise signer round trips: the web signer is often a slow remote NIP-46 signer on a phone. Steady-state decrypt cost must be **zero signer calls**.
- Non-goals for A: streaming terminals, screen streams, large files (slices B/C).

## 2. Transport: event kind and d-tags
All bridge events are **kind 30078** (NIP-78 application-specific data, parameterized-replaceable on `(pubkey, kind, d)`), authored by M, global (no `h` tag).

Why 30078 and not a new kind: the relay rejects unknown kinds (`required_scope_for_kind` in `crates/buzz-relay/src/handlers/ingest.rs`, `ALL_KINDS` in `buzz-core/src/kind.rs`), so a new kind needs a relay release. Buzz already multiplexes many features on 30078 by d-tag (read state, channel stars/mutes/sort, sections, theme — `desktop/src/shared/constants/kinds.ts`), `KIND_READ_STATE` is already accepted as `UsersWrite` and global-only. Using a distinct d-tag namespace `buzz.bridge.v1/` collides with none of them. (Checked free: 30180-30199, 44201-44299 are unused if we later want dedicated kinds; that requires adding them to `ALL_KINDS`, `required_scope_for_kind`, `is_global_only_kind`.)

d-tag namespace `buzz.bridge.v1/<topic>`:

| d-tag | Direction | Semantics | Replaceable? |
|---|---|---|---|
| `buzz.bridge.v1/key` | desktop → web | Wrapped session key (§3) | yes (head only) |
| `buzz.bridge.v1/agents` | desktop → web | Agent list/status snapshot | yes (head only) |
| `buzz.bridge.v1/hb` | desktop → web | Heartbeat / presence | yes |
| `buzz.bridge.v1/tx/<agentPubkey>` | desktop → web | Transcript head: latest chunk index + summary | yes |
| `buzz.bridge.v1/tx/<agentPubkey>/<seq>` | desktop → web | One transcript chunk (immutable, ≤ 16 KiB plaintext) | no (unique d) |
| `buzz.bridge.v1/cmd/<cmdId>` | web → desktop | One command | no (unique d) |
| `buzz.bridge.v1/res/<cmdId>` | desktop → web | Ack/result for a command | yes (ack → final) |
| `buzz.bridge.v1/snap/<topic>` | desktop → web | Other snapshots (projects, notifications, saved commands) | yes |

Standard tags on every bridge event: `["d", …]`, `["t","buzz-bridge"]`, `["kid", <keyId>]` (not on `key` events), `["expiration", <unix>]` (NIP-40; the relay may ignore it, so **clients enforce it too**).

Relay filter used by the browser: `{kinds:[30078], authors:[M], "#t":["buzz-bridge"]}` (one REQ; live + backfill `limit` small).

## 3. Encryption and the session key (zero signer calls in steady state)
NIP-44 v2 to-self needs the member private key (ECDH(M,M)). A remote NIP-46 signer would be asked on every event. Instead:

1. On start, the desktop generates a random 32-byte **bridge session key** `K` and `kid = hex(sha256(K))[0..16]`.
2. It publishes `buzz.bridge.v1/key`, content = `nip44EncryptToSelf(JSON{v:1, kid, key:hex(K), createdAt, expiresAt})` (desktop signer is local: free).
3. Every other bridge event's content is `nip44.v2.encrypt(JSON(envelope), K)` — i.e. K is used directly as the NIP-44 v2 conversation key. Pure local crypto on both sides.
4. The browser decrypts the `key` event **once per kid** through its signer (1 NIP-46 call), caches K (`sessionStorage`, keyed by key event id, so a reload doesn't re-ask the signer), and decrypts everything else locally. New events with an unknown `kid` wait for the new key event (no signer call is made for them).
5. K is rotated every desktop launch and at most every 7 days (`expiresAt`); the old key head is replaced by the new one. Compromise of K exposes bridge payloads only, not the Nostr key.

Trust note: any client that can obtain the member's decrypted K can read the bridge, and any holder of the member key (or K, for commands: see §6) can drive the desktop — the same trust boundary as the member key itself.

## 4. Envelope (plaintext inside the encrypted content)
```json
{ "v": 1, "type": "agents.snapshot", "kid": "ab12…", "dev": "<deviceId>", "seq": 42,
  "ts": 1760000000, "exp": 1760000090, "id": "<uuid>", "re": "<cmdId?>", "body": { … } }
```
- `v` protocol major version (integer). Receivers ignore envelopes with a higher `v` and surface "update the desktop app/PWA".  Minor additions are additive optional fields only.
- `type` message type (below). Unknown types are ignored (forward compatible).
- `dev` stable per-install desktop device id (random, stored locally). Multiple desktops for one member are distinguished by it; the web view lists devices. (v1 cut shows the most recently active device.)
- `seq` monotonic per `(dev, type-stream)`; browser drops seq ≤ last seen for the same stream (replay/out-of-order guard).
- `ts` creation time (unix s); `exp` expiry (unix s). Receivers drop `now > exp`, and drop `ts` more than 5 min in the future.
- `id` unique message id (idempotency key). `re` correlates a response to a command id.

## 5. Message types
Desktop → web:
- `agents.snapshot` — body `{ agents: AgentSummary[], truncated?: boolean, generatedAt }`, `AgentSummary = { pubkey, name, status: "running"|"stopped"|"deployed"|"not_deployed", pid?, runtime?, model?, personaId?, lastStartedAt?, lastError?, activeTurns: [{ channelId, channelName?, since }], currentTask?: string }`. No secrets, env vars, system prompts, log paths or command lines. `currentTask` is a short (≤ 140 char) label (active channel name, or last turn summary) — never message bodies. Published on change (debounced 2 s, ≥ 5 s apart) and forced every 5 min. exp = ts + 24 h (so a long-offline desktop still shows its last state, dimmed).
- `presence.heartbeat` — body `{ appVersion, platform, agentsRunning, uptimeSec, bridge: "v1" }`, every 30 s, exp = ts + 24 h; freshness is judged by `ts`, not `exp`. Web shows **"Desktop offline"** if no heartbeat with `ts` newer than 90 s (stale = now − max(hb.ts, snapshot.ts)).
- `transcript.head` / `transcript.chunk` — body `{ agentPubkey, seq, entries: [{t, role, text|tool}] }`, ≤ 16 KiB plaintext per chunk, backfill by fetching chunk d-tags `tx/<agent>/<n>` for the last N. (Not in first cut.)
- `snapshot.<topic>` — projects (branches/worktrees/PRs/status), notifications/unread, saved commands/routines. (Not in first cut.)
- `cmd.result` — see §6.

Web → desktop (`buzz.bridge.v1/cmd/<id>`, not in first cut):
- `agent.start {agentPubkey}`, `agent.stop {agentPubkey}`, `agent.message {agentPubkey, channelId, text}`, `prompt.approve|deny {agentPubkey, promptId}`, `command.run {name, args}`, `file.get {path, range}`.

## 6. Commands, ack/result, replay and idempotency
- A command envelope is `{type, id=cmdId, ts, exp (≤ ts+120 s), body}` encrypted under K, event author = M (signed by the web signer — one signer call per user command, acceptable because it is user-initiated; priority 0).
- Desktop accepts only if: event.pubkey == its own member pubkey; envelope decrypts with a current-or-previous K; `now ≤ exp`; `id` not seen (persistent LRU of ≥ 1,000 ids over 24 h); `type` in the allow-list for the signed-in capability. Everything else is dropped silently (logged locally).
- Dedup: desktop keeps the result keyed by `id`; a repeated command returns the stored result and does **not** re-execute (idempotent).
- Desktop answers with `buzz.bridge.v1/res/<cmdId>`: first `{status:"accepted"}` (ack, within ~1 s), then `{status:"ok"|"error", code?, message?, result?}`. Browser shows pending → ok/error, and times out at `exp` with "desktop did not respond".
- Destructive commands (stop, message, approve) require the desktop to show a local notification; an optional "ask on desktop" mode is a later decision.

## 7. Versioning and capabilities
- `v` in every envelope; the `presence.heartbeat` body advertises `bridge:"v1"` and `caps: ["agents.snapshot", …]`. Browser disables controls the desktop doesn't advertise.
- Breaking change → `buzz.bridge.v2/…` d-tag namespace, v1 kept alongside during migration.

## 8. Reuse by slices B and C
Slice B (direct WS over Tailscale) and C (hosted tunnel) carry the **same envelope** (§4) as WebSocket text frames — `{v,type,id,re,seq,ts,exp,body}` — minus the Nostr event wrapper; body types are shared, plus new `term.*` / `file.*` binary stream types (binary frames carry `{streamId, seq}` header + bytes). Auth for B/C: NIP-98 event (or a short-lived token minted over A: `cmd: transport.grant`, answered in `res/…` with `{url, token, expiresAt}`), checked `pubkey == own`. The browser's transport list prefers B, then C, falling back to A; the same `cmd` / `result` semantics (ids, ack/final, idempotency, exp) apply on every transport. For C, payloads stay end-to-end encrypted with K (the server only sees ciphertext) — decision at C go.

## 9. Limits and failure modes
- Relay delay: seconds. Snapshot ≤ 64 KiB plaintext (truncate agent list with `truncated:true`).
- Desktop offline → heartbeat stale → UI says "Desktop offline", shows last snapshot dimmed with its age.
- Desktop asleep/laptop closed: same as offline.
- Relay accepts events only from authed members; bridge events are readable by relay operators as ciphertext + metadata (d-tags, timing, sizes, `kid`). Metadata leak (when the member's desktop is online) is accepted for A.
- Clock skew: presence compares the desktop's `ts` with the browser clock; ±5 min tolerated for replay checks, but skew > 90 s would show a false "offline" (mitigation planned: a `cmd.ping` round trip that reports the desktop clock).


## 10. Decisions (JM, Oct 10 2026) and implementation notes
1. **Event type:** kind 30078 with `buzz.bridge.v1/…` d-tags. No relay change.
2. **Bridge key in the browser:** `sessionStorage` only (one signer prompt per app launch). Never IndexedDB.
3. **Destructive commands** (stop, message, approve/deny) need an explicit confirmation on the web side: `BridgeCommander.send` throws `confirmation_required` unless `confirmed: true`; the UI shows a confirm step. `agent.start` is not destructive. The desktop additionally enforces everything in §6 and never trusts the web UI.
4. **Multiple desktops:** v1 shows the most recently active one.

### 10.1 Transcripts (as built)
- d-tag `tx/<agentPubkey>/<seq>` (seq per agent, per desktop launch). **No `tx/<agent>` head event** was built: the web learns of transcripts from the chunks themselves (fewer relay events). Reserved for later.
- Body `{agentPubkey, entries:[{id,t,role,text,tool?,channelId?}]}`. Entries are keyed by `id`; a later chunk with the same id replaces the earlier entry (streaming messages grow). Chunks may arrive out of order: the web only lets a higher chunk `seq` replace an entry.
- First publish for an agent sends only the last 40 entries; afterwards only new/changed entries. Per agent: debounce 1.5 s, at least 4 s between chunks. Global cap 24 chunk events per rolling minute (extra waits). Chunk ≤ 16 KiB plaintext, entry text ≤ 1,500 chars, tool result head ≤ 300 chars. Chunk events expire after 6 h. Web keeps ≤ 300 entries per agent.
- Only agents present in the latest snapshot are published.
- **Redaction** (`redact.ts`, applied to every entry): nsec/ncryptsec, PEM private keys, Bearer/Basic tokens, `sk-…`, `ghp_…`, `github_pat_…`, `xox*-`, AWS keys, JWTs, Google API keys, URL credentials, any `UPPER_SNAKE=value` assignment, JSON fields named key/token/secret/password/credential/authorization, home paths (`/Users/x/…`, `/home/x/…`, `C:\Users\x\…` → `~/…`), deep absolute paths (→ `…/last/two`). Raw prompt sections ("metadata" items) and tool **arguments** are never published; tool entries carry name, status and a short redacted result head only.

### 10.2 Commands (as built)
- Web: `agent.start`, `agent.stop`, `agent.message {agentPubkey, channelId, text ≤ 4000}`. Heartbeat `caps` lists what the desktop supports; the web hides/refuses anything not listed. `prompt.approve|deny` are in the protocol but the desktop answers `unsupported` and executes nothing.
- Desktop (`BridgeCommandHandler`) executes only if ALL hold: kind 30078 + `t=buzz-bridge` + d-tag `cmd/<id>`; `event.pubkey` == member; valid event id+signature; event `created_at` within ±(120+300) s; `kid` tag == the current session key id; decrypts under the current key; envelope `id` == d-tag id; `v` ok; `now ≤ exp`; `ts ≤ now+300`; `exp − ts ≤ 120`; type in the allow-list; id not seen (persisted, 1,000 ids); `agentPubkey` is in the latest snapshot; type-specific body valid (`message` needs a running agent). Anything failing a crypto/authorship/time check is dropped silently (logged locally with the reason). A valid-but-unknown agent gets an `error/unknown_agent` result.
- Idempotency: the id is claimed before execution. A replay in the same run republishes the stored result without re-executing; an id seen in a previous run is dropped. `start` on a running agent and `stop` on a stopped one succeed with a note and do nothing.
- Result `res/<cmdId>`: `accepted` (ack) then `ok|error` (`code`: unknown_agent, bad_request, agent_not_running, unsupported, failed). Created-at of the final result is forced after the ack. The web shows pending → accepted → done/failed, or "desktop did not respond" after `exp`.
- Message execution: sends a normal channel message as the member (`send_channel_message` with the agent as mention).

### 10.3 Live subscription
The web subscribes with `limit: 150` (relay replay + live atomically) instead of a one-off fetch followed by `limit: 0`, which could miss events published during a slow-signer reconnect after a reload. A relay reconnect triggers a re-fetch; a failed subscribe retries with backoff.
