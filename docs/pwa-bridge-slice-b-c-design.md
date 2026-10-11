# Slices B and C — design (nothing built)

Status: design for review. Revised Oct 11, 2026 with JM's decisions (slice B reworked, file viewer/watching dropped, dev-server preview deferred, new view-only "Open on desktop / See screenshot" slice R added; revised again for slice R scope); slice C unchanged. Both slices reuse the slice-A envelope (`docs/pwa-bridge-protocol.md` §4, §6, §8): same `{v,type,id,re,seq,ts,exp,body}`, same command/ack/result semantics, same allow-list and replay rules, same session key `K` for end-to-end encryption. Transport changes, trust does not.

## 0. What B and C add over A
Live terminals (keystroke latency). **Dropped (JM, Oct 11): file viewer, file watching, file allow-list** (terminal access plus OpenClaw cover it). **Deferred: dev-server preview.** Slice A stays the fallback and control plane.

Desktop facts the design builds on: terminals already run on the desktop through `portable-pty` (`src-tauri/src/commands/term_session.rs`); `axum` and `tokio-tungstenite` are already dependencies; a loopback media proxy already exists (`media_proxy.rs`), so a local listener is an established pattern. No Rust change is needed for A; B needs new Rust; R needs small desktop changes.

## 1. Slice B (revised) — direct connection over the user's own Tailscale
### Principles (JM)
- The desktop listens on **loopback only**. It is reachable from a phone/browser only through the user's own tailnet.
- **The user manages Tailscale. The desktop never installs, starts or signs in to Tailscale.** It only *detects* whether Tailscale is up and, if so, publishes the listener with `tailscale serve`.
- No LAN/Internet exposure: never `0.0.0.0`, never `tailscale funnel`.

### Desktop: "Allow remote access" switch
1. The switch (Settings) is **enabled only when the desktop detects Tailscale up** (CLI present, `tailscale status --json` says running with a tailnet address). Otherwise it is disabled with the reason ("Tailscale isn't running on this Mac").
2. Turning it **on** starts the loopback listener and runs `tailscale serve` for it. It does not bring Tailscale up or change any other tailnet setting.
3. If Tailscale goes down (polled, ~10 s), the switch **turns itself off and stays off** until the user re-enables it. Turning it off, or quitting the app, removes the serve entry (`tailscale serve … off`) and stops the listener. Crash recovery: on next launch any stale serve entry created by this app is removed before anything is enabled.
4. Terminals are a **second, separate toggle** (below), also off by default.

### Discovery: desktops advertise themselves over slice A
Each desktop's signed bridge heartbeat (slice A, already authenticated by the member key and encrypted under the session key) gains `remote: {name, tailnetAddr, port, status: "on"|"off", caps: [...]}`. The web lists the member's **sharing** desktops (status on and fresh heartbeat); "off" desktops appear greyed with the reason. Several desktops per member are supported: each is a separate entry and verifies independently. (This also resolves slice A's "latest desktop only" limitation for B.)

### Authentication: one-time challenge signed by the member key
1. Browser opens `wss://<tailnet-host>/v1/ws` (Tailscale supplies TLS) and receives `{challenge}`: 32 random bytes, bound to this connection id, desktop id and a ≤ 60 s expiry, single use.
2. The browser asks its **remote signer (Primal)** to sign a kind-22242-style event containing the challenge, desktop id and purpose (`connect` or `terminal:<launchId>`). Primal can require Face ID. This costs a signer round trip per connection (and per terminal launch), by design.
3. The desktop verifies signature, `pubkey == its own member key`, challenge freshness/single use, and that the desktop id is its own. Only then does it accept frames on that connection. Each desktop verifies on its own, so a signature for desktop A is useless on desktop B.
4. A connection stays valid for its lifetime (idle timeout, 30 min default); no per-frame signing.

### Terminals (separate toggle, off by default)
- **Each terminal launch needs a fresh signed challenge** (purpose `terminal:<launchId>`); a connection challenge is not enough. After that it is normal SSH-like interactive access: **no per-command prompts**, with an idle timeout, a cap on concurrent remote terminals (default 3), a local notification on open, and a Kill list on the desktop.
- It **respects the user's existing terminal setting**: normal shell, or **herdr**.
- With herdr, the desktop launches or attaches using the **session name**, so a session started on the desktop can be **resumed from the phone** and the web lists the herdr sessions it can attach to (`herdr` session list via the desktop).
- Multi-attach caveats to verify before build (unknown today): behaviour when the desktop and phone are attached to the same herdr session at once (size negotiation, input interleaving, detach semantics), and whether herdr exposes a session list API we can call non-interactively. Plain-shell sessions are always new PTYs owned by the bridge and are never attached to an agent's own terminal.

### Security summary
Loopback bind + user-run `tailscale serve` (tailnet-only); member-key challenge per connection and per terminal launch; capabilities off by default; `Origin` check on WS upgrade; terminal bytes are never logged or sent over slice A; slice-A allow-list/replay rules apply to every control frame; the desktop makes no outbound connection for B.

### Web error states (explicit UI copy required)
| Situation | What the web shows |
|---|---|
| Tailscale off/not connected **on this device** | "Connect to Tailscale on this device to reach your desktop." (the tailnet host doesn't resolve/connect) |
| Mac asleep or app closed | "Your desktop isn't reachable (asleep or app closed)." (heartbeat stale, direct connect fails) |
| Remote access off on the desktop | "Remote access is off on <desktop name>. Turn it on in Settings on that Mac." (advertised `status: off`) |
| Tailscale down on the desktop | same as off, plus "Tailscale isn't running on that Mac." |
| Challenge refused/cancelled in the signer | "Approval was declined in your signer." |
| Terminals disabled | "Remote terminals are off on <desktop name>." |

### Status (Oct 11): B1 built, not wired into the app yet
`crates/buzz-remote-access` (workspace member, depends on nothing in the app): loopback-only WS listener (127.0.0.1, Origin allow-list, 64 KiB frames, 8 connections, auth timeout, idle timeout), member-key challenge verification (kind 22242; tags challenge/conn/desktop/purpose; single use, 60 s, bound to connection and desktop), Tailscale detect + `serve` lifecycle on **https port 9443 only** (never `serve reset`, never funnel; the Mac's existing serve entries on 443/8443 are untouched; stale 9443 entry removed first; auto-off when Tailscale drops), terminals off by default, fresh signed `terminal:<launchId>` challenge per launch, 3-terminal cap, kill list, real PTY backend (clean environment; plain login shell, or `herdr session attach <name>` when the terminal setting is herdr and a session name is given). 7 Rust tests (incl. a real PTY). Not yet done: Tauri wiring + Settings switches, `remote` field in the heartbeat, web transport/xterm.js tab/error states, herdr session listing, desktop notification on terminal open.

### Work items (estimate 2–3 weeks)
Rust: loopback listener + WS framing, Tailscale detection/serve lifecycle (incl. stale-entry cleanup), challenge verification, PTY bridge with resize/backpressure, herdr launch/attach/list, switches UI. Web: transport selector, desktops list, xterm.js tab, error states, challenge signing flow. Tests: Rust unit tests for challenge/replay/lifecycle; Playwright e2e with loopback standing in for the tailnet and a mock signer.

### Needs JM's approval before build
- New Rust code that opens a loopback listener in the desktop app (default off).
- Remote terminals at all (off by default, per-launch signed challenge).
- Shelling out to the `tailscale` CLI for detect + serve (read-only detect; `serve` only on user enabling).

## 1b. Slice R — desktop built-in browser: "Open on desktop" + "See screenshot" (view-only)
### Scope (JM, Oct 11)
- **Preview sites built on OpenClaw are ordinary links**: the web app opens them in a normal new browser tab, fully interactive there. Nothing special, nothing in this slice.
- Slice R is only about the **desktop client's built-in deep browser** (console, network, agent driving). From the web/mobile app there are exactly two actions, and the web side is **strictly view-only**:
  1. **Open on desktop** — a confirmed command that adds a link as a browser session in the desktop client.
  2. **See screenshot** — like the AWS console's button: an on-demand request that shows **one fresh screenshot** of that session; press again for another.
- **Not in scope, by decision:** periodic refresh, streaming, any frame-rate target, and any click/type/scroll/take-control from web or mobile.
- Agents drive the browser on the desktop side with its console/network tools and post screenshots to chat in the normal way, visible to anyone with access to that chat. That path is unchanged and unrelated to the bridge.

### Design
- **`browser.open {url}`** uses the existing slice-A command path unchanged: signed by the member key, treated as destructive-class (explicit web confirmation, like Stop/Send), validated by the desktop (own key, unexpired, not replayed, id claimed once). The result carries a `sessionId`. URL policy: `http`/`https` only; no `file:`, `javascript:` or credentials in the URL; the desktop may refuse private/loopback targets unless the user allows them.
- **`browser.screenshot {sessionId}`** — one request, one image. The desktop validates (own key, known `sessionId`, rate limit) and replies with one `browser.shot` result: a single compressed (JPEG/WebP), downscaled image published as an encrypted chunk(s) under the slice-A session key, shown once on the web. No state is kept beyond the latest shot per session (replaceable event), and it expires. Rate limit: at most one screenshot per session per ~3 s and a small per-minute cap; one image size cap (value to be set after measuring, see unknowns).
- The web lists the member's open desktop browser sessions (id, title, URL host) via a slice-A snapshot topic (`snap/browser`), read-only.
- **Safety:** while any session is open to remote screenshots the desktop shows the visible **"this session is remotely visible"** indicator; a **limit on concurrent sessions** (default 1–2); idle timeout closes remote exposure; closing the session on the desktop makes screenshot requests fail with a clear message. Command results never include page text, only the image.
- **Privacy notes (page pixels):** a screenshot can contain anything on the page, including logged-in content and secrets, and text redaction does not apply to pixels. Controls are the confirmation, the per-request nature (nothing is sent unless the user asks), the indicator, the rate limit, and the desktop being able to refuse (per-site block list is a possible addition). Anyone holding the member key can request shots, the same trust boundary as the rest of the bridge.

### Remaining unknowns (not yet investigated)
- **How desktop browser sessions / Stage / pinned sites work**: which webview/engine, how a session is created from a URL programmatically, how agents' console/network tools attach, whether a session can exist without a visible window.
- **How the desktop captures a screenshot** today (API, cost, resolution), and whether capture works for hidden/occluded windows on macOS or triggers the OS screen-recording permission prompt.
- **Screenshot size cap** over relay events (relay event size limits, chunking, encryption overhead) — to be measured; the cap and compression settings are undecided.
- Whether "Open on desktop" should focus the app/window or open silently, and where the new session appears in the desktop UI.
- Whether a per-site block list or per-session opt-in is wanted for the privacy controls.

### Needs JM's approval before build
A read-only investigation of the Stage/pinned-site machinery first; publishing page pixels over the relay (privacy); the default concurrent-session limit.

## 2. Slice C — hosted tunnel through buzz.huladesk.com
### Shape
The desktop opens an **outbound** WebSocket to a tunnel service and the browser connects to the same service; the service pairs them and forwards opaque frames. Payloads are end-to-end encrypted (NIP-44 under `K`, the slice-A session key), so the server only sees ciphertext, sizes and timing.

### Server changes needed on buzz.huladesk.com (NOT made; production relay untouched)
1. A new authenticated WS route, e.g. `GET /tunnel/<memberPubkey>/<role>` with `role = desktop | web`. A **separate service/process** next to the relay is preferred over changing `buzz-relay` (no change to relay event handling). If it must live in the relay: new route module only, no changes to ingest or kinds.
2. Auth: NIP-98 per connection, `pubkey == <memberPubkey>`, plus relay-membership check (same rule the relay uses for `/events`). Rate limits per pubkey; max 1 desktop and 3 web connections per member; idle timeout 5 min; max frame 256 KiB; per-member bandwidth cap.
3. Pairing and forwarding: byte-for-byte frame relay between the member's desktop connection and web connections; no storage, no logging of payloads; a 30 s presence ping; backpressure with drop-and-close (never unbounded buffers).
4. Deployment: reverse-proxy WS upgrade config, metrics (connections, bytes), kill switch (env flag).
5. **Security review before launch** (as in the plan): threat model for abuse as a generic relay/proxy (frames are only forwarded between a member's own two endpoints), DoS limits, key-handling review, logging review.

### Client work
Desktop: outbound tunnel client (reconnect with backoff, same frame handler as B). Web: transport adapter. Frames: `{kid, nonce, ciphertext}` wrapping the B frames; replay protection by per-direction counters. Estimate 3–4 weeks plus review, as in the plan.

### Needs JM's approval before build
- Any change on the production server (new route/service on buzz.huladesk.com), its hosting cost and abuse limits.
- Whether C is allowed to carry live terminal traffic (higher abuse/risk than status) or only status/files at first.
- The security review owner and sign-off.

## 3. Transport selection and failure behaviour
Order B → C → A for live features (terminals). The web probes B (reachability of the advertised tailnet address) and falls back silently, showing the error states in §1. Slice R uses slice A only (single on-demand screenshots). Any transport failure never loses commands: they are re-sent over A with the same id (idempotent).

## 4. Open questions and unknowns (honest list)
1. **herdr**: multi-attach behaviour and a non-interactive session-list API are unverified.
2. **Tailscale detection**: the exact CLI/JSON fields and macOS App Store vs standalone client differences (CLI path, `serve` syntax, sandboxing) are unverified; behaviour under Tailscale "serve" ACLs/HTTPS-cert settings not enabled on the tailnet is unknown.
3. **Challenge via remote signer**: assumed Primal can show a Face ID prompt for a signing request; NIP-46 signer latency (seconds) makes terminal launch slow; whether the signer will sign a kind-22242 challenge from a web origin is untested.
4. **Slice R (desktop browser)**: how desktop browser sessions/Stage work, how screenshots are captured, and the screenshot size cap — all unknown (see §1b).
5. **Slice C**: all server-side questions in §2 remain open; no production changes without approval and a security review.
6. Multi-desktop selection UX and per-desktop naming rules.
7. Whether the per-launch terminal challenge is too heavy for frequent use (a short-lived "terminal session grant" per connection is a possible relaxation, not adopted).
