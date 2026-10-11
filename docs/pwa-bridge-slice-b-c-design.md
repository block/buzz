# Slices B and C — design (nothing built)

Status: design for review. Revised Oct 11, 2026 with JM's decisions (slice B reworked, file viewer/watching dropped, dev-server preview deferred, new "remote browser" slice R added); slice C unchanged. Both slices reuse the slice-A envelope (`docs/pwa-bridge-protocol.md` §4, §6, §8): same `{v,type,id,re,seq,ts,exp,body}`, same command/ack/result semantics, same allow-list and replay rules, same session key `K` for end-to-end encryption. Transport changes, trust does not.

## 0. What B and C add over A
Live terminals (keystroke latency) and, later, smoother remote-browser video. **Dropped (JM, Oct 11): file viewer, file watching, file allow-list** (terminal access plus OpenClaw cover it). **Deferred: dev-server preview.** Slice A stays the fallback and control plane.

Desktop facts the design builds on: terminals already run on the desktop through `portable-pty` (`src-tauri/src/commands/term_session.rs`); `axum` and `tokio-tungstenite` are already dependencies; a loopback media proxy already exists (`media_proxy.rs`), so a local listener is an established pattern. No Rust change is needed for A; B and R need new Rust.

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

### Work items (estimate 2–3 weeks)
Rust: loopback listener + WS framing, Tailscale detection/serve lifecycle (incl. stale-entry cleanup), challenge verification, PTY bridge with resize/backpressure, herdr launch/attach/list, switches UI. Web: transport selector, desktops list, xterm.js tab, error states, challenge signing flow. Tests: Rust unit tests for challenge/replay/lifecycle; Playwright e2e with loopback standing in for the tailnet and a mock signer.

### Needs JM's approval before build
- New Rust code that opens a loopback listener in the desktop app (default off).
- Remote terminals at all (off by default, per-launch signed challenge).
- Shelling out to the `tailscale` CLI for detect + serve (read-only detect; `serve` only on user enabling).

## 1b. New slice R — remote browser ("Open browser" from the web app)
### Goal
From the web app, JM sends an **Open browser {url}** command; the desktop opens a browser session with its existing browser features; agents can drive it with the existing desktop browser tools; the web shows what the session looks like.

### Design
- **Command path:** the existing slice-A command path — signed by the member key, destructive-class so the web asks for explicit confirmation, validated by the desktop (own key, unexpired, not replayed, known device). New types `browser.open {url}` and `browser.close {sessionId}`; the result carries a `sessionId`. URL policy: `https`/`http` only, no `file:`/`javascript:`/credentials-in-URL; the desktop may refuse private/loopback targets unless the user allows it.
- **Desktop:** opens the session through the app's existing browser capability (see unknowns). Agents drive it with the existing browser tools, unchanged.
- **Frames:** the desktop captures screenshots and publishes them over slice A as **compressed, encrypted chunks** (`browser.frame`, JPEG/WebP, downscaled, target ~1 frame per 1–2 s, only when the page changed, hard size cap per chunk with multi-chunk frames if needed, same global relay-event budget as transcripts). The web shows the latest frame; no history is kept.
- **Phase 1:** view-only plus agent-driven (the agent clicks/types; the web watches). **Later:** web-sent click/type/scroll commands (each a validated command; coordinates mapped from the frame).
- **Safety:** a visible, non-dismissible **"this session is remotely visible"** indicator on the desktop for as long as frames are published; a limit on concurrent remote sessions (default 1–2); idle timeout; closing on the desktop stops frames immediately. Frame content is page pixels, so slice A's text redaction doesn't apply — the confirmation + indicator + per-session opt-in are the controls; a per-site block list is a possible addition.
- **Smooth video:** ~1 fps over relay events is a slideshow. Smooth video needs slice B's direct connection (WebSocket frames or WebRTC) and is a later enhancement.

### Unknowns to resolve before building (not yet read)
- How **pinned sites / Stage sessions** actually work (which webview/engine, how agents' browser tools attach, whether a session can exist headlessly or needs a visible window, how screenshots are captured today and at what cost).
- Whether capture works for a hidden/occluded window on macOS and whether it triggers the OS screen-recording permission prompt.
- Relay event budget and chunk-size limits at 1 frame/1–2 s per session (bandwidth, retention, relay rate limits) — needs measurement.
- Privacy: whether pages may contain secrets (logged-in sessions); the confirmation and indicator are required but may not be sufficient.
- How click/type coordinates map reliably through scaled frames (phase 2).

### Needs JM's approval before build
Reading/adopting the Stage/pinned-site machinery (read-only investigation first), publishing page pixels over the relay (privacy), and the default concurrent-session limit.

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
Order B → C → A for live features (terminals). The web probes B (reachability of the advertised tailnet address) and falls back silently, showing the error states in §1. Remote-browser frames use A first (slideshow) and B/C later for smooth video. Any transport failure never loses commands: they are re-sent over A with the same id (idempotent).

## 4. Open questions and unknowns (honest list)
1. **herdr**: multi-attach behaviour and a non-interactive session-list API are unverified.
2. **Tailscale detection**: the exact CLI/JSON fields and macOS App Store vs standalone client differences (CLI path, `serve` syntax, sandboxing) are unverified; behaviour under Tailscale "serve" ACLs/HTTPS-cert settings not enabled on the tailnet is unknown.
3. **Challenge via remote signer**: assumed Primal can show a Face ID prompt for a signing request; NIP-46 signer latency (seconds) makes terminal launch slow; whether the signer will sign a kind-22242 challenge from a web origin is untested.
4. **Remote browser (slice R)**: Stage/pinned-site internals, capture cost and permissions, relay budget at ~1 fps, privacy of page content — all unknown (see §1b).
5. **Slice C**: all server-side questions in §2 remain open; no production changes without approval and a security review.
6. Multi-desktop selection UX and per-desktop naming rules.
7. Whether the per-launch terminal challenge is too heavy for frequent use (a short-lived "terminal session grant" per connection is a possible relaxation, not adopted).
