# Buzz web / PWA — scoping and architecture plan

Status: draft, uncommitted. Evidence gathered 2026-10-10 on `merge/upstream-0.5.26` (77fb01a6d).
Spike: worktree `/Users/jm/Documents/buzz-pwa`, branch `pwa/spike` (uncommitted).

## 1. How coupled is `desktop/src` to Tauri? (counts from a scan of 2,100 non-test .ts/.tsx files)

| Surface | Count |
|---|---|
| Distinct `invoke("cmd")` / `invokeTauri("cmd")` literal command names | **403** across 101 files |
| Rust handlers registered in `generate_handler!` (lib.rs) | ~456 entries |
| `@tauri-apps/api/core` imports (`invoke` 38, `isTauri` 27, `Channel` 3) | 49 files |
| `@tauri-apps/api/event` (`listen` 29, `emit` 3) | 34 files |
| `@tauri-apps/api/window` (`getCurrentWindow` 12, `UserAttentionType`) | 12 files |
| `app` (`getVersion`) / `path` (`homeDir`) / `webview` / `webviewWindow` | 3 / 3 / 1 / 1 |
| plugin-opener (`openUrl`) / plugin-process (`relaunch`) / plugin-notification / plugin-updater | 15 / 3 / 1 / 1 |
| Distinct event names (`listen`/`emit`) | 41 (huddle-*, pairing-*, deep-link-*, pin-webview-*, browser-agent-*, tray-*, media-upload-*, agents-data-changed, ...) |
| Raw plugin IPC: `plugin:websocket\|connect/send/disconnect/disconnect_all` | used by `relayClientSession.ts` |
| Test-only mock layer | `src/testing/e2eBridge.ts` (15.6k lines, `mockIPC`) |

Chokepoints (good news):
- `src/shared/api/tauri.ts` (`invokeTauri`, 1068 lines) + `tauriChannels/Canvas/Messages/Events/Identity.ts` wrap nearly all IPC.
- `relayClientSession.ts` and `readOnlyRelayClient.ts` import `invoke`/`Channel` directly for the WebSocket plugin, and `createAuthEvent`/`signRelayEvent` from the wrapper.
- Only ~101 files touch IPC and the `@tauri-apps/*` import surface is ~12 module specifiers with ~20 distinct symbols, so a module-alias shim works without editing the 49+ call sites (this is what the spike does).
- `isTauri()` is already used in 27 places as a guard.

## 2. Classification of the 403 commands (regex-bucketed; approximate, needs a manual pass)

| Class | ≈Count | Notable |
|---|---|---|
| **A. Browser-direct** (crypto/nostr in JS; `nostr-tools` already a dependency) | 6 + most of the 96 relay-data commands | `sign_event`, `create_auth_event` (NIP-42), nip44, WS to relay |
| **A/B. Relay data reads/writes now done in Rust** (channels, messages, members, reactions, profiles, feed, notes, workflows, forum, canvas, search, unread) | ~96 | `send_channel_message` builds tags + signs + POSTs `/events` with NIP-98 in Rust (`relay.rs`). Must be re-implemented in TS over relay `/query`, `/events`, `/count` and WS REQ. Mechanical but large. |
| **B. Needs a server (BFF) or relay HTTP** | ~44 | media upload/fetch/proxy (`/media/upload`, `buzz-media:` scheme), link-preview metadata (CORS blocks arbitrary sites), admin_* (relay admin API), downloads/pickers (browser `<input type=file>` replaces pickers) |
| **C. Native-only, no browser equivalent** | ~253 | |
| – managed agents / ACP sidecars (`buzz-acp`, `buzz-agent`) / OpenClaw MCP / mesh-llm / personas+teams runtime / Bestie | ~96 | spawn local processes; can only be *remote-controlled* via a host machine |
| – native webviews: pinned sites / Stage / Drive / playground / html-preview / browser-agent | ~26 | WKWebView; iframes are blocked by X-Frame-Options/CSP on most sites |
| – huddle / audio devices / TTS / voice / local models | ~32 | LiveKit + cpal; partly replaceable by WebRTC + Web Audio |
| – local git / `gh` / checkout / repo fs | ~25 | needs a host or GitHub API instead |
| – OS integration: tray, haptics, idle/sleep, vibrancy, pop-out windows, deep links `hulabuzz://`, updater, archive sync, native notifications, clipboard images | ~34 | Web Push / Notification API / Clipboard API / `display-mode` cover a few |
| – keys at rest: OS keyring, ncryptsec backups, recovery pairing, builderlab | ~29 | replaced by WebCrypto/IndexedDB (see §3) |
| – terminals / PTY | ~15 | would need a server-side PTY over WS |

Events: of 41, ~36 are native-originated (huddle, tray, pairing, deep-link, webview, agent runtime) and simply never fire on web; the web adapter's `listen` is an in-page EventTarget.

## 3. Identity, keys and the relay

- Desktop stores the nsec in the **OS keyring** (`app_state_keyring.rs`, `identity_storage.rs`, feature `system-keyring`), with NIP-49 ncryptsec local backups. **Signing is in Rust** (`sign_event`, `create_auth_event`, `nostr` crate 0.44 with nip44/nip49). JS never sees the key; an egress guard blocks key-backup strings from leaving over websocket frames.
- Relay protocol (crates/buzz-relay): a generic Nostr relay — WebSocket with **NIP-42 AUTH**; HTTP `POST /events` and `/query`, `/count` authenticated with **NIP-98**; plus `/upload`, `/media/upload`, `/api/invites`, `/api/join-policy`, `/moderation/*`, `/health`, `/info`. The websocket is **already driven from JS** (`relayClientSession.ts`) via plugin-websocket, so the browser `WebSocket` is a drop-in (spike proves it compiles; not yet tested against a live relay).
- CORS: `build_cors_layer` in `crates/buzz-relay/src/router.rs` is **permissive when `BUZZ_CORS_ORIGINS` is empty**, otherwise an allow-list — the PWA origin must be added on any relay that sets it. Still to verify: any WebSocket `Origin` check (not found in this scan), mixed content (an https PWA can only open `wss://` — the default `ws://localhost:3000` only works on http://localhost), and the Hula relay's actual URL (not defined in the repo; `DEFAULT_RELAY_WS_URL = ws://localhost:3000`).
- Web key storage options: (1) paste nsec → store NIP-49 ncryptsec in IndexedDB, unlock with a passphrase via WebCrypto/`nostr-tools/nip49`; (2) NIP-07 browser extension; (3) NIP-46 remote signer (bunker) — best security, no key in the page; (4) server-held key (no). Recommend 1 as default + 3 as option. The spike keeps a raw hex key in localStorage (insecure, spike only).

## 4. Recommended architecture

1. **One React codebase, two shells.** Keep `desktop/src` as-is. Add `src/platform/` with a *platform adapter* interface (`invoke`, events, window, opener, notifications, updater, websocket). Two implementations: `tauri` (existing) and `web`. Short term: **Vite aliases** for `@tauri-apps/*` (done in spike, zero call-site edits). Long term: migrate call sites to `@/platform` and delete aliases.
2. **Command registry for web** (`platform/web/commands.ts`): each of the 403 commands is either implemented in TS (class A), proxied to a BFF (class B), or throws a typed `NativeOnlyError` (class C). A **capability map** (`platform.has("terminal")`) lets the UI hide/replace native-only surfaces with a clear "Available in the desktop app" placeholder instead of an error toast.
3. **Data layer in TS** for class A/B: port the Rust relay HTTP helpers (`relay.rs`, `commands/*`) to a `relayHttp.ts` (NIP-98 signer, `/query`, `/events`, `/count`) — the biggest chunk of work.
4. **PWA**: `manifest.webmanifest`, service worker (app shell cache-first for hashed assets, network-first navigations; never intercept relay traffic), installable, `display: standalone`; later Web Push (requires a push server/relay hook), IndexedDB for caches (replaces `channel_head_cache_*`, archive).
5. **Optional BFF** (small Node/Rust service) for: link-preview metadata, media proxy, push subscriptions, relay admin proxy — and later, a "remote host" bridge to a user's desktop for agents/terminals/git.
6. **Feature flags**: `import.meta.env.VITE_BUZZ_PLATFORM === "web"` (defined in spike) + `VITE_BUZZ_BESTIE=1` still chooses protected Hula UI.

What "pixel-for-pixel" can mean: all shared React/Tailwind/shadcn UI renders with identical CSS, fonts (Inter, JetBrains Mono bundled) and tokens → same pixels in Chromium/Safari for shared components, modulo font rendering, scrollbars, and the absence of native vibrancy/window chrome (traffic lights, `title_bar_double_click`, vibrancy) which become a plain web layout. What cannot be pixel-identical: native WKWebView surfaces (pinned sites, Stage, Drive, playground), terminals unless a server PTY + xterm.js is built, huddle audio UI backed by native devices, tray/menu bar, pop-out windows (become `window.open` / PWA windows). "Feature-for-feature" is achievable for roughly: chat, channels, DMs, threads, forum, notes, canvas, search, profiles, reactions, workflows (view/trigger), onboarding, settings, moderation/admin, media. It is **not** achievable standalone for managed agents, terminals, local git/Projects checkout, huddles with local models, OpenClaw MCP — those need a host machine (proxy) or are hidden.

## 5. Effort (one engineer-equivalent, rough)

| Slice | Scope | Effort |
|---|---|---|
| **S0 spike** (done) | alias shims, web build, manifest+SW, WS shim, local key, boots to real onboarding + key import | ~1.5 h |
| **S1** Boot + login + read/send | key import/unlock (NIP-49 in IndexedDB), relay URL config, NIP-42 auth over WS, communities, channel list, message history + live, send message/reactions/edit/delete over `/events`, installable PWA, hide AI-provider/native onboarding steps | 1.5–2.5 wk |
| **S2** Rest of chat data | threads, DMs, forum, notes, canvas, profiles/presence, search, unread/feed, members & channel admin, invites/join-policy | 2–3 wk |
| **S3** Media & previews | `/media/upload`, media fetch w/o `buzz-media:`, file pickers, drag/drop, link-preview BFF, clipboard | 1–1.5 wk |
| **S4** Capability gating | `platform.has()`, placeholders for native-only surfaces (≈250 commands, ≈26 webview, terminals, projects), settings pages trimmed | 1 wk |
| **S5** PWA hardening | offline shell + cache, Web Push (needs server), badge, share target, update flow replacing the updater, Safari/iOS quirks, CSP/headers | 1–1.5 wk |
| **S6** Workflows/admin/moderation/projects-read-only (GitHub API instead of `gh`) | 1.5–2 wk |
| **S7** Optional host bridge (agents, terminals, git via a paired desktop) | 4+ wk, separate product decision |
| **S8** Visual parity QA: Playwright screenshot diff web vs desktop for shared screens | 1 wk, runs through S1–S6 |

Total for the "web-capable subset": ~8–11 weeks; with the host bridge 12+.

## 6. Slice acceptance criteria
- **S1**: `vite build -c vite.config.web.ts` clean; Lighthouse "installable"; paste nsec → authenticates (NIP-42) against a relay; channel list matches desktop for same key; history loads; live messages arrive; a sent message appears on the desktop client; refresh keeps session; no uncaught `NativeOnlyError` toasts on the S1 path.
- **S2**: parity checklist of ~96 relay-data commands has TS implementations and unit tests reusing the existing `.test.mjs` suites; desktop and web agree on unread counts for the same key.
- **S3**: image/file upload + display works; link previews render through BFF.
- **S4**: every native-only surface shows a labelled placeholder; zero "command not found" console errors in a scripted tour.
- **S5**: app works offline for the shell, install on iOS/Android/desktop, push opt-in works.
- **S8**: screenshot diffs ≤ agreed pixel threshold for the shared screens list.

## 7. Spike result (see worktree)
- `vite build -c vite.config.web.ts` → success (~2 s, 6,312 modules, 22 MB assets), emits `dist-web/index.html`, manifest, `sw.js`.
- Dev server on http://127.0.0.1:5199 (index served at `/`); the real shared UI renders the real **Enter your private key** onboarding screen, accepts an nsec, calls `import_identity`, advances to the "Connect your AI provider" step (native-only → must be hidden on web).
- Known issues: `discover_acp_providers` and similar throw `NativeOnlyError`; no live relay was available locally (port 3000 closed) so relay AUTH/WS was not exercised; nip44 self-encrypt is a pass-through stub; key in localStorage; dev-only 403 for fonts via symlinked node_modules (fixed with `server.fs.allow`).
- Spike files: `desktop/src/platform/web/{core,event,misc,commands,identity,websocket,main.web}.ts(x)`, `desktop/vite.config.web.ts`, `desktop/index.web.html`, `desktop/public/{manifest.webmanifest,sw.js,pwa/}`.

## 8. Open questions
1. Where will it be hosted, and which relay URL/origin (must be `wss://` + in `BUZZ_CORS_ORIGINS` if set)?
2. Key model: passphrase-encrypted key in the browser, NIP-07/NIP-46 signer, or both?
3. Native-only features: hide, placeholder, or proxy through a paired desktop host?
4. Who is the audience (Hula team on desktop browsers vs iOS install)? Drives Web Push and Safari work.
5. Licensing/IP: reuse only this repo's code (no Kiingo/launchpad-26 code).

## 9. S1 status (updated Oct 10, 2026) — DONE (uncommitted, on `pwa/spike`)
Decisions: hosted beside the relay at `wss://buzz.huladesk.com` (HTTP base `https://buzz.huladesk.com`; build-time `VITE_BUZZ_RELAY_URL`, dev default `ws://localhost:3000`, `?relay=` override); keys via NIP-07 extension only (no nsec stored anywhere; the spike's localStorage key path was deleted); desktop browsers + iOS; native-only = labelled placeholder now, decided per feature later.

Delivered (all under `desktop/src/platform/web/` unless noted):
- `signer/{types,nip07,nip46,index}.ts` — pluggable `Signer` (NIP-07 complete incl. nip44 when offered; NIP-46 placeholder). Session persists only `{kind,pubkey}`.
- `UnlockScreen.tsx` — detects `window.nostr`; clear "no extension found" + disabled "NIP-46 remote signer — coming soon".
- `relayConfig.ts`, `relayHttp.ts` (NIP-98 via signer), `relayWsQuery.ts` (single NIP-42-authed WS for reads, HTTP fallback for bridge-only filters), `nostrConvert.ts` (TS port of `nostr_convert.rs`), `chat.ts` (chat-path command handlers), `commands.ts` registry (86 handlers).
- `capabilities.ts` + `NativeOnly.tsx` — capability map (below).
- `web.css` — 100dvh, safe-area insets, 16px inputs on touch.
- PWA: `index.web.html` (iOS meta, viewport-fit=cover, theme-color), `public/manifest.webmanifest`, `public/sw.js` (shell cache only; never /query /events /media or WS), icons incl. maskable + apple-touch-icon. "Connect your AI provider" onboarding step is skipped on web (machine onboarding auto-marked done).
- Tests: `desktop/scripts/pwa/{s1-e2e,pwa-check,capability-report,mock-nip07,relay-client,...}.mjs`.

Capability map (408 distinct commands invoked by the client): web-implemented 86; placeholder: agents 96, webviews 27, huddle_audio 28, git_projects 25, terminals 13, os_integration 10, chat_data_s2 56 (needed for S2); hide: keys_at_rest 31; proxy (needs server): media_and_previews 15, relay_admin 21. Regenerate: `node scripts/pwa/capability-report.mjs` (`LIST=1` for per-command lists).

Verification: vite build OK; `tsc --noEmit` clean; Playwright e2e against a local relay 22/22 (mock NIP-07, NIP-42, channel list, history, live updates, extension-signed send via WS and NIP-98 REST, resume, 390x844); `pwa-check.mjs` 10/10 on the prod build (manifest, icons, SW control, shell cached, API not cached, offline shell, prod bundle has relay default and no dev hooks).
Not verified: real browser extension (e.g. nos2x/Alby), real iOS Safari, production relay/CORS, Lighthouse score.

## 10. S1b status — NIP-46 remote signer (Oct 10, 2026) — DONE (uncommitted, on `pwa/spike`)
- `signer/nip46.ts` — client: ephemeral client keypair, kind 24133 over the signer's own relay(s), NIP-44 (NIP-04 responses understood), `connect`/`get_public_key`/`sign_event`/`nip44_*`, `auth_url` handling, 30 s request timeout (120 s connect, 180 s after auth_url), per-relay reconnect with backoff, republish of pending requests on reconnect, resubscribe with `since`, foreground/`pageshow`/`online` triggers a reconnect. Returned events are verified (sig, pubkey == user, kind/content unchanged).
- `signer/nip46Store.ts` — IndexedDB `{kind,clientSecret,remotePubkey,relays,userPubkey}`. `signer/signerStatus.ts` — vanilla-DOM "Waiting for your signer…" toast (+ tap-to-open auth_url link).
- Unlock screen: `bunker://` paste, `nostrconnect://` QR + "Open in signer app" deep link, signer-relay field (`?nip46relay=` overrides), "Forget saved signer". NIP-07 button stays when an extension exists; on iOS it is hidden and NIP-46 is primary.
- Tests: `scripts/pwa/mock-nip46-signer.mjs` (in-process ws relay + signer), `scripts/pwa/s1b-nip46-e2e.mjs`.
- Not verified: real nsec.app / Amber / Primal / iOS Safari. Next: Slice 2 (remaining chat commands), media/link-preview proxy.

## 11. Bridge slices A / B / C (added Oct 10, 2026) — PLANNED, nothing built until JM says "go" per slice
Queue order: (1) signer performance fix, (2) built version served on its own HTTPS port, then A → B → C. Native-only cleanup list: see [pwa-native-only-cleanup.md](./pwa-native-only-cleanup.md). Goal: let a browser signed in as a member see and control that member's running desktop client (agents, projects, terminals) without putting the native code in the browser.

**Shared design.** Slice A defines the message format (versioned envelope, request/response with ids, event streams, capability advertisement). Slices B and C reuse it unchanged as a transport swap: the same JSON envelopes ride over a direct WebSocket (B) or the relay-hosted tunnel (C). The browser keeps a transport list and **prefers direct (B), then hosted tunnel (C), falling back to the Nostr relay (A)**; A is always available when the desktop is online, and live-only features (terminal, preview) simply show "unavailable on this path" when only A is reachable. Auth is the member's own key throughout (NIP-98/NIP-42 or NIP-44 to self); the desktop only accepts requests from its own pubkey (plus an explicit allow-list later).

### Slice A — Relay bridge (NIP-44 encrypted Nostr events)
- Desktop publishes, as NIP-44 encrypted events addressed to the member's own pubkey (e.g. a dedicated kind range, replaceable for snapshots, regular for logs/commands): agent list and status, current task, recent activity, transcripts (chunked), project panel snapshots (branches, worktrees, PRs, commit status), notifications/unread counts, saved commands and routines, basic file view and diff (chunked text).
- Browser (same member, via NIP-07/NIP-46 signer) subscribes, decrypts and renders; sends command events: start/stop/message agent, approve/deny permission prompts, run a saved command/routine, request a file/diff.
- Desktop validates author == own pubkey, replay protection (id + created_at window), idempotent commands, acks with result events.
- Needs: encrypt/decrypt cost via remote signer is a perf risk (NIP-46 round trips) → derive a per-session symmetric key once (NIP-44 conversation key exchanged via one signer call) and cache.
- Limits: delay of seconds, no streaming terminal, only works while desktop is online. Effort estimate: ~3–4 wk (format + desktop publisher + web agent view + controls).
- Acceptance: browser shows the same agent list/status as the desktop within a few seconds; start/stop/message round-trips; approve/deny works; commands from any other pubkey are ignored; no plaintext on the relay.

### Slice B — Direct connection over Tailscale
- Desktop exposes a small local authenticated API (HTTP + WebSocket) bound to localhost and published with `tailscale serve` (HTTPS on the tailnet).
- Features: live terminal tabs (WS PTY stream + xterm.js), dev-server preview (reverse proxy to a local port), file watching, large file transfers, direct API calls for anything needing a plain URL.
- Auth: browser presents a NIP-98 event (or a short-lived token minted via slice A); desktop checks pubkey == own. Tailnet membership is the outer gate.
- Reuses slice A envelopes; adds binary frames for terminal/file data. Only works on the tailnet and while the Mac is awake. Effort: ~2–3 wk.
- Acceptance: interactive shell from iOS Safari with sub-100 ms echo on tailnet; preview of a local dev server; reconnect after sleep; unauthenticated requests rejected.

### Slice C — Hosted tunnel on buzz.huladesk.com
- Desktop holds an outbound authenticated WebSocket to the relay host; the server forwards authenticated browser requests down it (and streams back). Same live features as B, from anywhere, no Tailscale.
- Needs **new server code** in the relay (tunnel endpoint, per-pubkey routing, NIP-98/NIP-42 auth on both ends, rate limits, size caps, audit log, kill switch) and a security review, since it is a path from the internet to a member's machine.
- Reuses slice A envelopes. Effort: ~3–4 wk plus review.
- Acceptance: terminal and preview work from a non-tailnet network; only the owner's pubkey can reach their desktop; server never sees plaintext if end-to-end encryption of tunnel payloads (NIP-44 to self) is enabled (decide at go).

Dependencies: A before B and C. B and C are independent of each other. Open decisions at go time: event kinds, snapshot cadence, whether tunnel payloads are end-to-end encrypted in C, allow-list for non-owner devices.


## 12. Slice A status (Oct 10, 2026) — transcripts and controls BUILT on `pwa/spike-on-release`
- Branch `pwa/spike-on-release` = release `0.5.26-hula.1` (`f8e0ca19d`, identical to `main`) + PWA commits; `pwa/spike` is kept as the old line.
- Done: key wrap, agents snapshot, heartbeat, transcript chunks + agent detail view, start/stop/message with web confirmation and desktop-side validation. Details and decisions: `docs/pwa-bridge-protocol.md` §10.
- Not done / limits: approve/deny (no desktop mechanism), `command.run`, file view, project snapshots, multi-desktop picker, channel picker UX (the message box takes a channel id, suggested from the agent's active channels).
- Decisions (JM): kind 30078 + d-tags; bridge key in `sessionStorage` only; destructive commands need web confirmation; desktop ignores expired / replayed / non-member / unknown-agent commands.
- Next: slice B and C designs in `docs/pwa-bridge-slice-b-c-design.md` (nothing built; approvals listed there).
