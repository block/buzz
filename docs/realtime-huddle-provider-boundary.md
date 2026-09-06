# Realtime Huddle provider boundary

Status: VOICE 1 discovery decision. This document defines the implementation
boundary for VOICE 2; it does not deliver a realtime provider.

## Scope

VOICE 1 answers one question: where can a realtime speech provider attach
without replacing Buzz Huddle transport, identity, authorization, or effects?

The decision is to add a Buzz-owned realtime media bridge beside the existing
human peer and local TTS publisher. For the first safe slice, Buzz tees the
consenting local user's captured PCM to the provider and publishes returned
audio through a send-only Huddle peer authenticated as the managed agent. Buzz
converts between Huddle media and a small PCM/event provider contract. The
provider never joins a Huddle directly and never receives Buzz credentials or
authority.

The following are deliberately not part of this slice:

- provider runtime code or credentials;
- changes to the relay or Huddle wire format;
- a new SFU, media gateway, or general plugin framework;
- ElevenLabs support before the OpenAI adapter proves the seam;
- provider-originated tool execution before VOICE 4;
- redesign of barge-in/floor control before VOICE 5;
- reconnect orchestration beyond fail-closed teardown before VOICE 6;
- the provider-hosted audio/video sessions proposed by PR #7217; and
- the turn-based OpenAI-compatible STT backend proposed by PR #7232.

## Audited current flow

### Huddle lifecycle and membership

`desktop/src-tauri/src/huddle/mod.rs` owns the Huddle state machine. Starting a
Huddle creates a private ephemeral channel, enrolls selected agents with the
`bot` role, publishes the start event, and then connects audio. Joining reuses
the same post-connect path. Leaving or ending cancels the audio socket before
resetting state and shutting down STT/TTS. Agent add/remove operations update
relay membership before changing local state.

The ephemeral channel ID plus the local `huddle_generation` identify one local
Huddle lifetime. `session_generation` separately fences transcript work and is
not sufficient to identify a realtime voice session.

### Audio admission and transport

The audio endpoint is the existing authenticated WebSocket
`/huddle/{channel_id}/audio` in
`crates/buzz-relay/src/audio/handler.rs`. Admission requires:

1. a valid NIP-42 challenge response signed by the connecting peer;
2. protocol-version agreement with the room;
3. an existing channel in the same tenant/community;
4. channel membership, or an existing documented auto-add path for an eligible
   human member; and
5. a successful bounded room admission.

A managed agent additionally supplies its owner authorization (`auth` tag). The
Desktop already exercises this path in
`desktop/src-tauri/src/huddle/agent_tts_publisher.rs`: it loads the matching
managed-agent identity, verifies that its public key is the requested speaker,
checks active `bot` membership, and opens a publisher socket signed by that
agent.

The relay assigns each admitted socket a peer index, publishes authoritative
roster control messages, and fans binary frames to every other peer. It treats
Opus payloads as opaque bytes. Per-peer audio queues are bounded and use
best-effort delivery; control queue loss terminates the affected connection so
it must obtain a fresh roster snapshot.

### Codec and wire contract

The released Desktop speaks Huddle protocol v2:

- client to relay: `[8-byte header][Opus payload]`;
- relay to client: `[peer index][8-byte header][Opus payload]`;
- the header carries a wrapping sequence, a 48 kHz media timestamp, client-owned
  dBov telemetry, and a DTX flag; and
- audio is mono Opus at 48 kHz in 20 ms / 960-sample frames. Current Desktop
  encoders use the VoIP application, 32 kbit/s, and DTX.

The control roster maps peer index to authenticated pubkey. The newer relay
code can carry an occupancy epoch in protocol v3, but the current Desktop pins
v2. A v2 frame cannot distinguish a prior occupant after an index is reassigned,
and media/control use separate queues without a cross-queue ordering guarantee.
VOICE 2 must therefore not use relay-received frames as external-provider input.
It takes input from the local user's Buzz-owned capture path before Opus and
keeps the managed-agent Huddle socket send-only. Supporting consented remote
speakers later requires a separately proven ordering/non-reuse contract or v3
occupancy epochs; it must not be inferred from the v2 roster.

The human Desktop peer already owns mic PCM -> Opus send, per-peer jitter and
Opus decode, playback, remote-human STT mixing, active-speaker updates, and
human-floor signals. The local agent TTS publisher already proves that a second
socket can publish audio under the managed agent's identity, but it intentionally
ignores received binary audio and is not a realtime provider bridge.

## Ownership boundaries

| Concern | Owner after VOICE 2 | Reason |
| --- | --- | --- |
| Tenant, channel, and Huddle lifecycle | Buzz | Existing authoritative state and relay events |
| Human and agent identity | Buzz | Nostr keys and NIP-OA owner authorization must not leave Buzz |
| Room admission and membership | Buzz relay | Existing authenticated Huddle endpoint is authoritative |
| Peer roster and speaker authorization | Buzz | Relay-authenticated pubkeys are the only trusted attribution |
| External audio egress consent | Buzz | Room membership alone does not authorize third-party processing |
| Huddle Opus encode/decode and jitter | Buzz | It is the existing room media contract |
| Provider protocol, model, and session I/O | Provider adapter | This is the only provider-specific behavior |
| Dialogue audio generation | External provider | OpenAI first; ElevenLabs only after the seam is proven |
| Tool/effect authorization and execution | Existing managed-agent path | Provider output is never authority |
| Barge-in and floor policy | Buzz | Shared across providers and tied to Huddle participants |
| Credentials, spend policy, and telemetry | Buzz | Provider must not decide operational policy |

## Selected minimum seam

There are two sides, but only one is provider-pluggable.

### 1. Buzz-owned realtime media bridge

Reuse the already-proven managed-agent publisher socket for provider output. It
must keep the current socket authentication, Opus settings, bounded queue, and
cancellation conventions. It accepts normalized provider PCM for publication
and remains send-only in VOICE 2; it is not a provider interface.

Provider input comes from a bounded tee at the existing local microphone PCM
boundary, before the human peer encodes that same capture to Opus. Forwarding
requires one named `ExternalAudioEgressGrant`: an explicit local-user choice
bound to the Huddle lifetime, managed-agent pubkey, provider, and the local
human pubkey held by the Desktop. That pubkey must be a current non-bot room
member allowed by the managed agent's existing `respond_to` policy. Room
membership or provider readiness alone is not consent to third-party audio
processing. The tee is closed unless the complete grant remains current.

VOICE 2 does not decode or forward any audio received by the agent socket, so
remote-human and bot streams have no path to the provider. Multi-participant
consent, disclosure controls, and a safe remote-media attribution prerequisite
remain VOICE 7 work. Removing the agent's `bot` membership or invalidating the
local user's grant cancels the provider session and publisher.

This gate is structural and identity-based. Voice activity, transcript text,
keywords, or model judgment must not grant authorization or consent.

### 2. Provider session

The provider contract is a session-scoped stream, not a registry or plugin
framework. Its provider-independent vocabulary is intentionally small:

```text
start(session configuration) -> session
session.send_audio(PCM chunk)
session.cancel_response()
session.next_event() -> audio | transcript | speech-state | tool-proposal | closed
session.close()
```

PCM chunks have an explicit sample rate, mono channel count, sample format, and
bounded duration. Buzz performs exactly one normalization at each media
boundary: local captured PCM to the provider's required PCM input, and provider
PCM to 48 kHz mono frames for Huddle Opus publication. The VOICE 2 input path
never decodes Huddle Opus. Provider wire messages and provider-specific
rate/format negotiation stay inside the adapter.

The session configuration may contain provider model/voice settings and
non-secret dialogue instructions. It does not contain Nostr keys, relay auth,
channel membership authority, or effect capabilities.

A `tool-proposal` is inert data. VOICE 2 must advertise no tools and reject an
unexpected tool proposal without executing it. VOICE 4 may connect proposals
to the existing managed-agent effect boundary, where Buzz rechecks actor,
channel, policy, arguments, and audit requirements before any effect.

Realtime and local TTS also need one Buzz-owned output lease keyed by the same
Huddle lifetime and agent pubkey. Acquiring realtime ownership prevents
`speak_agent_message` and its local TTS publisher from speaking for that agent;
releasing it restores the existing local route. This is a mutual-exclusion
rule, not a new audio framework, and it must be atomic with session ownership.

## Why this option

Three alternatives were rejected:

1. **Connect the provider directly to the relay.** This would disclose agent
   signing material or require a new credential/gateway authority and would let
   an external service participate in Buzz admission.
2. **Create provider-hosted rooms.** That duplicates PR #7217 and abandons the
   existing Huddle Opus room rather than extending it.
3. **Generalize STT/TTS backends into a broad voice plugin framework.** The local
   pipeline is turn-based, while realtime providers expose session events and
   full-duplex audio. A framework is not needed to prove OpenAI, and ElevenLabs
   is the deliberate second-adapter test in VOICE 3.

The selected seam is the smallest safe change that satisfies the roadmap: one
bounded local-capture tee, the existing shape of a send-only authenticated agent
publisher, and one session interface around provider media/dialogue I/O. No
relay change is required. A receive-capable agent socket is intentionally not
part of VOICE 2.

## Lifecycle and failure contract

A realtime session is keyed by `(ephemeral_channel_id, huddle_generation,
agent_pubkey)`. At most one local session may own that key.

It may start only when all of these facts are current:

- the local Huddle is `Connected` or `Active`;
- the agent is an active `bot` member of the ephemeral channel;
- the matching locally managed identity and owner authorization are available;
- provider configuration passes its typed readiness check;
- an `ExternalAudioEgressGrant` exists for the local user and exact session;
- the managed-agent `respond_to` policy allows that local user; and
- no realtime session or local TTS output lease already owns the same key.

Huddle leave/end, generation change, agent removal, local-user membership or
egress-grant loss, or explicit disable cancels both sides. Provider failure
closes only that agent publisher and reports degraded voice state; it must not
end the human Huddle. Publisher failure cancels the provider session so a paid
or recording session cannot survive after it loses its Buzz authority.

All media channels are bounded. Fresh audio may replace/drop stale audio under
backpressure; it must not accumulate unbounded latency. State-bearing control,
authorization, and tool data are never silently dropped: loss or malformed data
closes the relevant session. Cancellation wins over queued provider audio, so
no stale speech is published after teardown.

VOICE 2 is fail-closed and does not add automatic reconnect. VOICE 6 may add
bounded reconnect only after it proves single ownership and no duplicated
speech or effects.

## VOICE 2 implementation slice

The first implementation should be one independently verifiable vertical slice:

1. after explicit local-user enablement, create an
   `ExternalAudioEgressGrant`, acquire the selected enrolled managed agent's
   output lease, and create one OpenAI Realtime session;
2. authenticate a send-only Huddle socket as that agent using the existing
   NIP-42 plus owner-auth path;
3. tee only the granted local user's captured PCM to OpenAI; no relay-received
   media is connected to provider input;
4. convert returned audio to the existing 48 kHz mono Opus room contract and
   publish it as the authenticated agent peer; and
5. tear both connections and the output lease down together on any authority or
   lifecycle loss.

Likely touch points are the Desktop Huddle modules (`mod.rs`, `state.rs`,
`pipeline.rs`, `relay_api.rs`, and the existing agent publisher/auth helper)
plus narrowly named OpenAI adapter code and focused tests. This is an expected
change map, not permission to refactor the existing playout or local STT/TTS
pipelines.

### Acceptance criteria

VOICE 2 is ready only when focused local tests demonstrate all of the following:

- the provider peer appears in the existing roster under the managed agent's
  pubkey and remote participants hear its returned audio;
- a missing/mismatched managed identity, invalid owner authorization, absent
  `bot` membership, wrong room protocol, absent/mismatched egress grant, or
  denied `respond_to` policy prevents startup or forwarding;
- only capture owned by the current, non-bot local user named by the
  exact-session egress grant reaches the provider;
- relay-received media is not connected to provider input, proving that remote
  humans, bots, unknown indices, and v2 index-reuse races have no egress path;
- post-cancel local capture and provider audio never cross the bridge;
- local TTS and realtime output cannot simultaneously own one agent/Huddle key,
  including concurrent-start races, and local TTS becomes available after the
  realtime lease is released;
- provider PCM is normalized once and emitted as 48 kHz mono, 20 ms Huddle Opus
  frames whose sequence and 48 kHz timestamp advance by 1 and 960 respectively,
  including the v2 contract's defined integer wrapping boundaries;
- bounded media queues prefer freshness and cannot grow without limit;
- cancellation prevents queued provider audio from being published;
- provider disconnect tears down the agent audio peer without ending the human
  Huddle, while audio authority loss tears down the provider session;
- OpenAI tools are disabled and an unexpected tool proposal has no effect; and
- focused tests cover normal bidirectional media, each authorization/consent
  refusal, malformed provider messages, backpressure, provider/publisher
  failure, output-lease exclusion, and teardown races.

The complete-diff architecture audit must separately verify that no relay or
Huddle wire change was introduced, local STT behavior was not changed, local
TTS changed only for the named output-lease exclusion, PR #7232 code was not
copied or made a dependency, and no kind 48200/48201 or PR #7217
provider-hosted media path is used. These are provenance/scope checks, not
claims that behavioral tests can prove.

The final implementation gate is the repository's full locally available
Desktop checks plus the normal repository-wide gate required by `TESTING.md`.
Physical-device, Mobile, privacy/spend observability, mature reconnect, and full
barge-in release evidence remain assigned to VOICE 5–8 and are not silently
pulled into VOICE 2.
