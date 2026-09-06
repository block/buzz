# Realtime Huddle provider boundary

Status: VOICE 1 discovery decision. This document defines the implementation
boundary for VOICE 2; it does not deliver a realtime provider.

Evidence was taken against `block/buzz` commit
`3c7f288c60d67df78577b237e27c3dfc8831aaa1`. No speech payload, credential,
or provider response body is retained in this document.

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

## Measured current baseline

The repository's ignored `huddle::latency_bench` test drives the production
Desktop STT and TTS pipelines with a zero-delay fake LLM. A checked-in Parakeet
test WAV was converted from PCM16 mono 16 kHz to float32 mono 48 kHz, then fed
in real-time 100 ms batches. On this host, a three-turn debug-profile run at the
evidence commit measured:

| Turn | STT after speech end | First TTS audio after handoff | End to first audio |
| --- | ---: | ---: | ---: |
| short reply | 731 ms | 495 ms | 1,226 ms |
| medium reply | 1,042 ms | 466 ms | 1,508 ms |
| long reply | 2,496 ms | 972 ms | 3,468 ms |

The median end-to-first-audio was 1,508 ms with no model/relay agent delay. This
is a bounded synthetic diagnostic, not a release-performance claim: debug
inference is slower than release, the third turn includes the longest reply, and
three samples do not support a percentile. For comparison, the release-profile
production harness recorded 924–1,087 ms before the exact TTS conditioning cache
and 347–384 ms with all experimental local latency levers enabled in PR #5671.
The current source keeps those levers gated, so neither historical extreme is
claimed as the current default.

Reproduction, after creating the standard Tauri sidecar stubs:

```bash
ffmpeg -i ~/.buzz/models/parakeet-tdt-ctc-110m-en/test_wavs/0.wav \
  -ac 1 -ar 48000 -c:a pcm_f32le /tmp/huddle-baseline.wav
BUZZ_BENCH_WAV=/tmp/huddle-baseline.wav BUZZ_BENCH_TURNS=3 \
  cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib \
  huddle::latency_bench::baseline_stt_fake_llm_tts_first_audio \
  -- --ignored --nocapture
```

Current interruption policy is independently pinned in production-bound tests:
an isolated output route accepts confirmed onset immediately; an acoustically
coupled route requires 20 consecutive 16 ms VAD-positive frames, or 320 ms,
before taking the human floor. Cancel/drain starts a 100 ms output-tail
hangover. PR #6431 records the physical reason for the coupled-route debounce
and the regression evidence. This is a policy bound, not a measured device
silence percentile.

Desktop and Mobile keep the Huddle alive during audio-only reconnect. Both use
the bounded delay sequence `0, 100, 250, 500, 1000, 2000, 2000 ms`; after the
last failed attempt they leave rather than retaining a false-connected session.
The relay and client media paths are lossy and bounded. The current local TTS
text queue has eight entries; the agent publisher input queue has eight packets
and caps expanded pending output at 1,500 20 ms frames (30 seconds), dropping
new overflow rather than growing without limit. VOICE 2 deliberately requires
a much shallower freshness-oriented provider output bound so stale generated
speech cannot consume this whole legacy allowance.

A physical two-endpoint Huddle timing run was not completed in VOICE 1: only
macOS was available as a Flutter target, and no OpenAI or ElevenLabs sandbox
credential was present. This is explicit missing runtime evidence, not inferred
success. VOICE 2 remains gated on a credentialed, spend-capped provider probe;
VOICE 5 owns the physical interruption matrix and VOICE 8 owns Desktop plus
unchanged-Mobile acceptance.

## Provider contract comparison

OpenAI Realtime is the first provider. It already exposes one realtime session
with streamed input/output audio, server or semantic VAD, response cancellation,
conversation-item truncation for unheard audio, function-call proposals, and
per-response usage. Its PCM path is the narrowest fit: Buzz converts local PCM
to the provider's negotiated mono PCM input and converts returned mono PCM once
to the existing 48 kHz Huddle path. The provider WebSocket is owned by a Tauri
backend task; the API credential is read only by that host process and is never
sent to the WebView, relay, Nostr events, logs, or session configuration.

ElevenLabs Agents is a credible second consumer, not a requirement for the first
slice. Its WebSocket similarly negotiates PCM/telephony formats and emits audio,
VAD/speech-state, transcripts, interruption, and client-tool-call events. It
normally uses an agent configuration plus a signed conversation URL. That
additional agent/configuration lifecycle is unnecessary to prove the Buzz seam,
and implementing it now would turn VOICE 2 into a two-provider framework.

| Contract question | OpenAI Realtime | ElevenLabs Agents | VOICE 2 decision |
| --- | --- | --- | --- |
| Media | Negotiated PCM or telephony audio; streamed deltas | Negotiated PCM or μ-law input/output | Use OpenAI mono PCM; no provider Opus |
| Turn detection | `server_vad` or `semantic_vad`; client may commit | Provider VAD/turn state | Start with provider VAD; Buzz still owns floor |
| Interruption | Cancel response and truncate unheard assistant audio | Interruption event and buffered-audio controls | Local Buzz cancellation wins first |
| Tools | Streamed function-call arguments and function result items | Client tool calls and tool results | Advertise none; reject tool events inside the adapter until VOICE 4 |
| Session auth | Host-held API credential on the provider connection | Host API key obtains/opens an authorized conversation | Credential never crosses Buzz media/control planes |
| Retention/region | Account/project policy dependent | Workspace/plan policy dependent | No universal claim; enforce product policy in VOICE 7 |
| Usage | Response usage fields | Conversation metadata/API | Record only redacted counts, audio duration, latency, and cost units |

Official contract references used for this decision are the OpenAI
[Realtime WebSocket](https://platform.openai.com/docs/guides/realtime-websocket),
[conversation](https://platform.openai.com/docs/guides/realtime-conversations),
[VAD](https://platform.openai.com/docs/guides/realtime-vad), and
[function-calling](https://platform.openai.com/docs/guides/realtime-function-calling)
guides, plus the ElevenLabs Agents
[WebSocket](https://elevenlabs.io/docs/agents-platform/libraries/web-sockets)
and [client tools](https://elevenlabs.io/docs/agents-platform/customization/tools/client-tools)
guides. Account-specific zero-retention, data-residency, and regional processing
were not verified and must not be represented as product guarantees.

A no-secret network probe confirmed that the ElevenLabs signed-URL endpoint
rejects an unauthenticated request (`401`, `needs_authorization`). An
unauthenticated OpenAI `POST /v1/realtime/sessions` probe returned `404`; that
observation alone does not prove endpoint lifecycle. VOICE 2 must pin the actual
endpoint, authentication method, event schema, and negotiated audio format from
the then-current official WebSocket guide and a credentialed sandbox probe.
Codec, VAD, cancellation, and function-call behavior remain unverified until
that probe passes.

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

The named boundary contracts for implementation are:

- `RealtimeVoiceSessionKey`: normalized ephemeral channel ID, Huddle generation,
  and normalized managed-agent pubkey;
- `ExternalAudioEgressGrant`: exact session key, local human pubkey, provider,
  grant generation, and explicit enabled state;
- `CapturedHuddlePcm`: exact Huddle and capture generations, capture-source
  identity, and bounded PCM from the human-peer transmission gate;
- `ProviderSessionBudget`: fixed wall-clock, admitted-input, and decoded-output
  audio allowances plus one terminal exhaustion reason;
- `NormalizedAudioFrame`: mono PCM samples plus an adapter-owned validated sample
  rate and one fixed maximum sample count;
- `ProviderSessionState`: `preparing`, `ready`, `active`, `draining`, or `closed`,
  with one terminal reason and generation-fenced transitions;
- `RedactedVoiceMetrics`: session correlation ID, stage timestamps, audio
  duration, drop/cancel counters, provider usage units, and terminal reason—no
  PCM, transcript, instructions, arguments, credentials, or signed events.

Each external/provider input is validated and normalized once before these
contracts are consumed. Provider event names, JSON shapes, and base64 fields do
not escape the adapter.

## Selected minimum seam

There are two sides, but only one is provider-pluggable.

### 1. Buzz-owned realtime media bridge

Reuse the already-proven managed-agent publisher socket for provider output. It
must keep the current socket authentication, Opus settings, bounded queue, and
cancellation conventions. It accepts normalized provider PCM for publication
and remains send-only in VOICE 2; it is not a provider interface.

Provider input comes from a bounded tee after the existing local microphone's
mute/PTT, device, Huddle-generation, and human-peer transmission gates, but
before that same admitted PCM is encoded to Opus. The WebView-to-Tauri IPC
carries one `CapturedHuddlePcm` envelope rather than unversioned raw bytes. Its
Huddle generation, capture generation, and capture-source identity are checked
against the current capture lease before the single fan-out to both relay Opus
encoding and provider input. A delayed callback from a prior device or
leave/rejoin therefore cannot be rebound to the current sender. Raw capture that
the current human peer is not authorized to transmit must have no provider
path. Forwarding requires one named `ExternalAudioEgressGrant`: an explicit
local-user choice
bound to the Huddle lifetime, managed-agent pubkey, provider, and the local
human pubkey held by the Desktop. That pubkey must be a current non-bot room
member allowed by the managed agent's existing `respond_to` policy. Room
membership or provider readiness alone is not consent to third-party audio
processing. The tee closes immediately on mute, device/capture loss, leave,
stale capture generation, or grant loss and remains closed unless the complete
grant is current.

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
session.next_event() -> audio | transcript | speech-state | closed
session.close()
```

PCM chunks have an explicit sample rate, mono channel count, sample format, and
bounded duration. Duration is not the memory bound: each adapter must enforce a
fixed raw WebSocket-message byte limit before JSON or audio decoding, reject an
encoded audio field over its fixed byte limit before base64 decoding, and reject
decoded PCM over fixed byte and sample-count limits before conversion or
resampling. The limits belong to the typed adapter input contract and must be
sized for the largest supported provider frame, not from provider-declared rate
metadata.

Buzz performs exactly one normalization at each media boundary: local captured
PCM to the provider's required PCM input, and provider PCM to 48 kHz mono frames
for Huddle Opus publication. The VOICE 2 input path never decodes Huddle Opus.
Provider wire messages and provider-specific rate/format negotiation stay
inside the adapter.

The session configuration may contain provider model/voice settings and
non-secret dialogue instructions. It does not contain Nostr keys, relay auth,
channel membership authority, or effect capabilities.

VOICE 2 advertises no tools. Any unexpected provider tool event is rejected
inside the adapter without propagating arguments or executing anything. VOICE 4
owns the later introduction of proposal/result contracts at the existing
managed-agent effect boundary, where Buzz rechecks actor, channel, policy,
arguments, and audit requirements before any effect.

Realtime and local TTS also need one Buzz-owned output lease keyed by the same
Huddle lifetime and agent pubkey. Lease acquisition is a generation-fenced
quiescence handshake: it first cancels and drains active/queued local synthesis,
invalidates pending local-publisher packets, confirms that generation can no
longer send, and only then grants realtime ownership. While held, it prevents
new `speak_agent_message` work and local TTS publication for that agent.
Releasing it restores the existing local route without replaying stale queued
speech. This is a mutual-exclusion rule, not a new audio framework, and it must
be atomic with session ownership.

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
egress-grant loss, explicit disable, or local mute/capture loss cancels the
applicable input or both sides. Canonical authority comes from the current
`HuddleState` generation and local controls, an exact managed-agent policy
snapshot, and authoritative channel membership—not a 15-second poll. The audio
roster is attribution only; it does not prove continuing membership or policy.
Existing Nostr membership-change notifications trigger
`fetch_channel_members_with_roles` revalidation, while local managed-agent
record changes and owner-authorization expiry trigger policy revalidation. No
new event kind is needed. Watchers and the expiry timer are armed before the
initial snapshots; events observed during snapshot reads are reconciled before
the session/output lease can activate. Every callback carries the session
generation. Any relevant notification closes the shared media-authorization
gate synchronously before asynchronous revalidation; only an exact-session
snapshot passing under that same gate may reopen it. Loss of the membership
subscription, failed revalidation, managed-record watcher, or expiry timer
closes provider input and publisher output rather than retaining stale
authority. Provider failure closes only that agent publisher and reports
degraded voice state; it must not end the human Huddle.
Publisher failure cancels the provider session so a paid or recording session
cannot survive after it loses its Buzz authority.

All media channels are bounded. Fresh audio may replace/drop stale audio under
backpressure; it must not accumulate unbounded latency. State-bearing control,
authorization, and tool data are never silently dropped: loss or malformed data
closes the relevant session. One Buzz-owned cancellation generation is checked
before provider decode, PCM normalization, queue insertion, Opus encode, and
socket send. Send authorization and cancellation are linearized by one bounded
gate: cancellation advances the generation under the same synchronization
boundary, drains unsent bridge audio, and waits for any previously authorized
socket write before reporting completion. No send authorization may begin after
that completion point. Audio already transmitted to other peers cannot be
recalled; the separate VOICE 5 playout/floor slice must drain or suppress
matching local receive/jitter/playout state to meet the speech-to-silence
threshold.

VOICE 2 is fail-closed and does not add automatic reconnect. VOICE 6 may add
bounded reconnect only after it proves single ownership and no duplicated
speech or effects.

The first slice has a fixed Buzz-owned per-session resource/spend budget:
20 minutes wall clock, 15 minutes of PCM admitted to provider input, and
5 minutes of decoded provider output. Budget is charged from validated frame
sample counts before queueing, so provider metadata cannot reduce it. Reaching
any limit closes input, cancels the active response, drains unsent output,
closes provider and publisher sockets, releases the output lease, and records
terminal reason `budget_exhausted`. It never auto-starts a replacement session.
VOICE 7 may replace these conservative constants only with an approved product
spend policy and equivalent fail-closed tests; telemetry alone is not a budget.

## VOICE 2 implementation slice

The first implementation should be one independently verifiable vertical slice.
Its acceptance run predeclares exactly 20 non-overlapping scripted attempts
after one warm-up turn on an isolated/headset route; failed or cancelled
attempts are not retried. The run manifest pins the exact Desktop commit,
provider endpoint/API revision, model, voice, VAD mode, input and output PCM
formats, machine, route, and network type. `t0` is the local Buzz capture/VAD
speech-end marker before provider endpointing; `t1` is the first non-DTX agent
frame accepted by the local human peer's normal playout. Latency is `t1 - t0`.
Every non-completion is scored as an infinite threshold miss; median and
nearest-rank p95 are computed over all 20 attempts, and completion rate must be
at least 95%. The target is median at most 800 ms and p95 at most 1,500 ms. The
raw retained artifact contains only the pinned manifest, timestamps, durations,
counters, and terminal reasons—no PCM, transcript, or instructions. VOICE 2
additionally requires that no new frame obtains send authorization after
cancellation completes.

The roadmap interruption target, owned by VOICE 5 rather than silently pulled
into this slice, is human speech onset to locally rendered agent silence within
250 ms at p95 on an isolated/headset route. The coupled-speaker route keeps its
current 320 ms anti-echo debounce until VOICE 5 proves a faster safe policy, so
it is reported separately and is not disguised as meeting the headset target.

These are product acceptance thresholds, not claims about the uncredentialed
VOICE 1 run. They require realtime to materially beat the measured turn-based
median while preserving the known speaker-echo trade-off.

The vertical slice is:

1. after explicit local-user enablement, create an
   `ExternalAudioEgressGrant`, acquire the selected enrolled managed agent's
   output lease, and create one OpenAI Realtime session;
2. authenticate a send-only Huddle socket as that agent using the existing
   NIP-42 plus owner-auth path; reuse its auth/encode contract but do not mark
   this peer as locally synthesized TTS, so the host's human socket receives and
   plays provider audio through the same normal agent-peer path as Mobile;
3. carry capture/source generations through the IPC boundary and tee only the
   granted local user's current `CapturedHuddlePcm` to OpenAI after one exact
   lease validation; no raw muted, stale-source, or relay-received media is
   connected to provider input;
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
- only `CapturedHuddlePcm` owned by the current, non-bot local user named by the
  exact-session egress grant reaches relay/provider fan-out; delayed prior
  Huddle, capture-generation, and source callbacks fail before either output;
- relay-received media is not connected to provider input, proving that remote
  humans, bots, unknown indices, and v2 index-reuse races have no egress path;
- muted, wrong-device, stale-generation, and post-cancel local capture never
  reaches the provider; after cancellation starts no new provider frame obtains
  send authorization, unsent queued audio is drained, and completion waits for
  writes already authorized before cancellation;
- local TTS and realtime output cannot simultaneously own one agent/Huddle key,
  including acquisition during active synthesis, queued publication, and
  concurrent-start races; acquisition quiesces old TTS before realtime sends,
  and release neither replays stale speech nor blocks fresh local TTS;
- provider PCM is normalized once and emitted as 48 kHz mono, 20 ms Huddle Opus
  frames whose sequence and 48 kHz timestamp advance by 1 and 960 respectively,
  including the v2 contract's defined integer wrapping boundaries;
- bounded media queues prefer freshness and cannot grow without limit;
- exact and max-plus-one tests cover all three `ProviderSessionBudget` limits;
  exhaustion is terminal, tears down both sockets, and releases the output lease
  without automatic restart;
- provider transport tests accept the exact raw-message, encoded-audio, decoded
  byte, and sample-count maxima, reject each maximum plus one before the next
  allocation/transformation stage, and reject forged rate metadata that would
  otherwise bypass duration-only validation;
- cancellation prevents queued provider audio from being published;
- provider disconnect tears down the agent audio peer without ending the human
  Huddle, while audio authority loss tears down the provider session;
- provider credentials use a secret-bearing type with redacted diagnostics;
  query strings, transport traces, response bodies, surfaced errors, metrics,
  and Nostr events pass a canary-secret non-disclosure test;
- OpenAI tools are disabled and an unexpected tool event is rejected inside
  the adapter without propagating its arguments or producing any effect; and
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
