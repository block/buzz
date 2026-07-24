# Buzz Bridge Protocol v1

The bridge protocol lets you carry Buzz events over **your own network**.
You run an endpoint — in any language — and a Buzz consumer (the ACP
harness, a bot) connects to it with `RemoteTransport` instead of a Buzz
relay. From the consumer's point of view nothing changes: it publishes
signed events and receives channel-tagged signed events. What your endpoint
does with them — forwards them to Slack, fans them out over a private mesh,
stores them — is entirely up to you.

The design premise: **the same events, transferred through a bidirectional
stream.** Every frame is a small JSON object, and the payload of the `event`
frames is the unmodified signed-event JSON that Buzz speaks natively. There
is no REST surface and no second protocol — even channel discovery and
membership ride the stream, because in Buzz those are events too.

## Carriers

The frames are carrier-agnostic; the client's bridge URL scheme selects how
they travel:

| Scheme | Carrier | Framing |
|---|---|---|
| `wss://` (or opted-in `ws://`) | WebSocket | One JSON object per text message |
| `unix:///path/to/bridge.sock` | Unix domain socket | One JSON object per LF-terminated line |

A Unix-socket bridge is the lightest way to run a local sidecar (or a test
harness): listen on a socket, read lines, write lines. A WebSocket bridge is
the right shape for anything remote.

WebSocket carriers can additionally be tunneled through a **SOCKS5 proxy**
(`BUZZ_TRANSPORT_SOCKS_PROXY=socks5://[user:pass@]host:port`) — Tor, an SSH
`-D` tunnel, or a private-overlay entry point. This is invisible to the
bridge: hostnames are resolved at the proxy, so overlay-only names
(`.onion`, mesh-internal DNS) work.

## Transport requirements

- **TLS for remote endpoints.** Clients require `wss://`. Plaintext `ws://`
  is accepted only for loopback hosts, for `.onion` destinations through a
  **loopback** SOCKS5 proxy (Tor's encryption starts at the proxy, so the
  proxy must be on the same machine for no plaintext to cross the network),
  or when the operator explicitly opts in (`allow_insecure` /
  `BUZZ_TRANSPORT_ALLOW_INSECURE=true`) for a trusted private network.
  `unix://` sockets are local by construction — filesystem permissions are
  the access control.
- **Authentication.** If the operator configured a token, it arrives in the
  `hello` frame's `token` field on every carrier, and *additionally* as an
  `Authorization: Bearer <token>` header on the WebSocket upgrade request
  (useful for rejecting early, before any frame). Reject the connection if
  it is missing or wrong. The client also announces its public key in the
  `hello` frame; use it to scope what you deliver.
- **Text frames only**, one JSON object per frame, at most **524,288 bytes**
  (the Buzz relay's default frame cap, so any event the relay accepts fits).
- **Keepalive** is WebSocket-level ping/pong where the carrier has it. The
  client answers pings; it does not send protocol-level heartbeats.

## The signed event

The payload of every `event` frame is a signed event — plain JSON, seven
fields:

```json
{
  "id":         "<64-char hex — SHA-256 of the canonical serialization>",
  "pubkey":     "<64-char hex — author public key (secp256k1, x-only)>",
  "created_at": 1700000000,
  "kind":       9,
  "tags":       [["h", "0b1f…-uuid"], ["p", "deadbeef…"]],
  "content":    "hello",
  "sig":        "<128-char hex — Schnorr signature over id>"
}
```

This is structurally a [Nostr NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md)
event — Buzz's native form — so events pass through a bridge losslessly. You
do not need a Nostr library to build a bridge: treat events as opaque signed
JSON and route them. If your bridge *originates* events (e.g. mirroring
Slack messages into Buzz), it must sign them with its own key; any NIP-01
library, or ~50 lines of SHA-256 + BIP-340 Schnorr, does the job.

Clients verify the `id` hash and `sig` of every inbound event and silently
drop events that fail. An unsigned or tampered event will never reach the
consumer.

## Handshake

The first frame in each direction negotiates the protocol:

1. Client → bridge: `{"type":"hello","version":1,"pubkey":"<64-char hex>","token":"<optional>"}`
2. Bridge → client: `{"type":"hello_ack","version":1}`

The versions must match; the client disconnects otherwise. The bridge may
send `notice` frames before the `hello_ack` (they are logged), but the
`hello_ack` must arrive within 20 seconds. After a reconnect the client
repeats the handshake and re-sends all of its subscriptions **exactly as
first issued** — including any `replay_since` watermark — so a reconnecting
client may receive events it has already seen. Treat replay as advisory and
deduplicate by event `id` where overlap matters.

## Frames: client → bridge

| `type` | Fields | Meaning |
|---|---|---|
| `hello` | `version`, `pubkey`, `token?` | Handshake, first frame on every connection |
| `subscribe` | `channel_id`, `kinds?`, `require_mention?`, `replay_since?` | Declare interest in a channel's events |
| `unsubscribe` | `channel_id` | Withdraw interest |
| `event` | `event` | A signed event published by the consumer |

`subscribe` fields:

- `channel_id` — UUID string identifying the channel. Your bridge chooses
  the mapping (e.g. one UUID per Slack conversation) and must use it
  consistently in both directions.
- `kinds` — optional array of kind integers to deliver. Absent = all kinds;
  an explicit empty array matches nothing (the NIP-01 edge case).
- `require_mention` — optional boolean (default `false`). When true, only
  deliver events that `p`-tag the `hello` pubkey.
- `replay_since` — optional Unix timestamp. If your bridge stores history,
  replay stored events created at/after it (oldest first), then send `eose`.
  Bridges without history may ignore it.

Subscriptions are **advisory**: a bridge that already knows which
conversations the agent belongs to may push events for them regardless.
Re-subscribing a channel replaces the previous subscription.

## Frames: bridge → client

| `type` | Fields | Meaning |
|---|---|---|
| `hello_ack` | `version` | Handshake acknowledgment, first frame |
| `event` | `channel_id`, `event` | A signed event, tagged with its channel |
| `ok` | `event_id`, `accepted`, `message?` | Optional ack of a client-published event |
| `eose` | `channel_id` | Optional end-of-replay marker |
| `notice` | `message` | Human-readable diagnostic (logged by the client) |

`ok` and `eose` are optional — the stream is fire-and-forget, like the relay
socket it mirrors. Send `ok` with `accepted:false` and a `message` when you
reject a publish; the client logs it.

## Forward compatibility

- Receivers **ignore frames** whose `type` they do not recognize.
- Receivers **ignore unknown fields** inside known frames.
- Breaking changes bump `version` in the handshake.

This means a v1 bridge keeps working against newer clients until the major
version actually changes.

## Delivering the workspace over the stream

Everything a consumer learns about its workspace arrives as events, so a
bridge controls the whole experience with the one `event` frame:

- **Messages**: kind `9` / `40002` stream messages, with the channel UUID in
  an `h` tag and the `channel_id` field.
- **Membership / discovery**: kind `39002` (channel members) and `39000`
  (channel metadata) events announce which channels exist and who is in
  them.
- **Ephemeral signals**: typing (`20002`) and presence (`20001`) kinds pass
  through like any other event.

Kind numbers live in `crates/buzz-core/src/kind.rs`.

## Sketch: a Slack bridge

A minimal Slack bridge is a single process that:

1. Serves `wss://bridge.internal/buzz` and checks the bearer token on
   upgrade.
2. Answers `hello` with `hello_ack`, remembering the client's pubkey.
3. Maintains a table `slack_channel_id ⇄ channel_uuid` (any stable mapping —
   e.g. UUIDv5 of the Slack channel ID).
4. On Slack message (Events API / Socket Mode): build a kind-`9` event with
   the text as `content`, an `h` tag carrying the channel UUID, and — when
   the Slack message @-mentions the agent's Slack user — a `p` tag carrying
   the client pubkey. Sign it with the bridge's key and send it as an
   `event` frame (respecting `require_mention` if the subscription set it).
5. On client `event` frame: verify the signature, extract the channel UUID
   from the `h` tag, look up the Slack channel, and post the `content` via
   `chat.postMessage`. Optionally answer with an `ok` frame.
6. On `subscribe`: start forwarding that conversation (and, optionally,
   replay history via `replay_since` + `eose`).

Steps 4–5 are the whole data plane. Threading, reactions, and richer kinds
can be layered on incrementally — unknown kinds simply flow through.
