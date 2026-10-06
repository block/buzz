# Shared Huddle audio client

`buzz-audio-client` contains the Huddle v2 audio handshake, wire format, Opus
sender, and per-peer NetEQ jitter buffer used by Buzz desktop clients. It does
not depend on Tauri, the v0 desktop application, a relay server, identity stores,
or speech models. A consumer can select this package from a pinned `block/buzz`
Git revision without building the rest of the workspace.

The host validates the destination and supplies an asynchronous NIP-42 signer
to `connection::connect`. The crate bounds connection and admission, validates
the initial roster, and returns a socket owned by the host. Dropping the pending
future or socket releases it; the crate creates no detached tasks. Hosts retain
room creation, membership checks, mute, device routing, connection supervision,
and application teardown. Audio admission alone does not grant chat access.

`AudioEncoder` accepts up to 960 finite mono samples at 48 kHz and produces a
20 ms v2 packet. `PeerJitterBuffer` consumes the sequence/timestamp/Opus fields
from `wire::parse_relay_frame` and produces 480 samples per 10 ms playout tick.
Hosts maintain one receiver per peer, discard its state on leave or roster
reassignment, bound playback queues, and stop idle peers after buffered audio
drains. V2 has no occupancy epoch in media, so a delayed packet from a previous
occupant cannot be distinguished after index reuse.

The default `native-roots` feature selects system TLS trust. Hosts that use
WebPKI roots select `default-features = false, features = ["webpki-roots"]`;
hosts can also pass their configured TLS connector. Signing and trust policy
remain host-owned.

Run `cargo test -p buzz-audio-client` and
`cargo clippy -p buzz-audio-client --all-targets -- -D warnings` from the root
with Hermit active. Tests use real loopback WebSockets and Opus audio, without a
live community or microphone. Client integration still requires a native
two-person call, mute, leave, cancellation, and device-switching tryout.

STT/TTS providers, model installation, and speech HTTP endpoints are intentionally
outside this transport crate.
