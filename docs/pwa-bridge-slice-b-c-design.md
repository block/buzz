# Slices B and C — design (nothing built)

Status: design for review, Oct 10, 2026. Both slices reuse the slice-A envelope (`docs/pwa-bridge-protocol.md` §4, §6, §8): same `{v,type,id,re,seq,ts,exp,body}`, same command/ack/result semantics, same allow-list and replay rules, same session key `K` for end-to-end encryption. Transport changes, trust does not.

## 0. What B and C add over A
Live terminals (keystroke-latency), dev-server preview (HTTP on the Mac), file watching and larger file transfer. Slice A stays the fallback and the control plane that hands out access (`transport.grant`).

Desktop facts the design builds on: terminals already run on the desktop through `portable-pty` (`src-tauri/src/commands/term_session.rs`); `axum` and `tokio-tungstenite` are already dependencies; a loopback media proxy already exists (`media_proxy.rs`), so a local listener is an established pattern. No Rust change is needed for A; B and C need new Rust.

## 1. Slice B — direct connection over Tailscale
### Shape
- A new Rust module `bridge_local` starts an axum server on **127.0.0.1:<random port>** (never 0.0.0.0). Exposure to the tailnet is done by the user's own Tailscale (`tailscale serve --https=<port> http://127.0.0.1:<port>`), which terminates TLS with the machine's `*.ts.net` cert and restricts to tailnet peers. The app never binds a tailnet or LAN address itself.
- Endpoints: `GET /v1/ws` (WebSocket; text frames = envelopes, binary frames = `{streamId u32, seq u32}` + bytes), `GET /v1/preview/<port>/…` (reverse proxy to a dev server on localhost, only for ports the user allow-listed), `GET /healthz`.
- The app learns whether it is reachable from `tailscale status --json` (best effort) and publishes the URL in the slice-A heartbeat as `caps: ["transport.b"]` plus a `transport.grant` result.

### Security model
1. **Network:** tailnet-only by construction (loopback bind + `tailscale serve`, not `funnel`). A Tailscale ACL can narrow it further. No port is opened to the LAN/Internet.
2. **Authentication of the person:** the browser proves it holds the member key with a NIP-98 event (kind 27235) over `GET /v1/ws` (`u`=URL, `method`=GET, nonce, ≤ 60 s old); the server checks the event signature and `pubkey == member`. Nonces are remembered (replay guard). That costs one signer round trip per connection, so the grant flow below is preferred.
3. **Grant flow (fewer signer prompts):** the web sends `transport.grant` over slice A (already confirmed/validated like any command). The desktop returns `{url, token, expiresAt}` where `token` is a random 256-bit value valid ≤ 10 minutes, bound to the browser's device id; the WebSocket presents it in the `Sec-WebSocket-Protocol` header. Tokens are single-connection, revocable, and never logged.
4. **Authorization:** every frame goes through the same allow-list as slice A (`BridgeCommandHandler` generalised); terminal and file commands are **off by default** and need an explicit per-capability toggle in desktop Settings ("Allow remote terminals", "Allow remote preview", "Allow file read"), plus a path allow-list (project roots only, no dotfiles/keychain paths) and the slice-A redaction on anything returned that is not a raw terminal stream.
5. **Terminals:** a remote session is a new PTY owned by the bridge (labelled "remote" on the desktop, listed in a local Settings panel with a Kill button, hard idle timeout, max 3). It does **not** attach to an agent's own terminal. The destructive-confirmation rule applies to *opening* a session (web confirm); keystrokes are not confirmed individually. A local notification fires on open.
6. **Origin:** the PWA origin must be in an allow-list (`https://*.ts.net` of this machine); `Origin` is checked on WS upgrade, preview responses get `Content-Security-Policy: sandbox` and are served from a separate path prefix.
7. **Confidentiality on the wire:** TLS from Tailscale; frames are additionally NIP-44-encrypted under `K` only in slice C (the relay/server is untrusted there). In B the tailnet is the trust boundary.

### Work items
Rust: listener + WS framing (~1 wk), PTY bridge with resize/backpressure (~1 wk), preview proxy with port allow-list (~3 d), file watch + read API (~3 d), capability toggles UI (~2 d). Web: xterm.js terminal tab, transport selector (B → C → A), preview iframe (~1 wk). Tests: Rust unit tests for auth/allow-list/replay; Playwright e2e with a fake tailnet (loopback) and a mock signer. Estimate 2–3 weeks as in the plan.

### Needs JM's approval before build
- New Rust code that opens a (loopback) network listener in the desktop app.
- Remote terminals at all (shell access from a browser): off by default, per-capability toggle. Confirm that default.
- Whether to rely on the user-run `tailscale serve`, or have the app run it (`tailscale serve` needs the CLI and changes tailnet config).
- Path allow-list policy for file access.

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
Order B → C → A. The web probes B (`/healthz` over the tailnet) and falls back silently; capability and offline UI use the same `caps`/presence logic as A. Any transport failure never loses commands: they are re-sent over A with the same id (idempotent).

## 4. Open questions
Multi-desktop selection UI; whether file watching needs per-project opt-in; whether the preview proxy should be limited to ports registered by agents' dev-server tooling.
