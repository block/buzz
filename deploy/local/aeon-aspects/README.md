# AEON Aspect workers

This package defines six disabled-by-default Buzz ACP workers. It renders and
validates configuration only; it does not install, load, start, restart, or
switch any live service.

Each worker uses Buzz only as the conversation transport. `--no-memory` keeps
Gateway as the sole memory and compaction owner, while `--no-base-prompt`
prevents Buzz's generic agent/tool doctrine from replacing the canonical Aspect
instructions. The adapter binds to a pre-created session with
`openclaw acp --require-existing --session ...`. Existing Buzz ACP controls are
pinned rather than inherited from changing defaults: heartbeat prompting and
proactive count-based session rotation are off, thread/DM context is bounded to
12 messages, presence and typing remain on, and turns use a 900-second idle timeout with a two-hour
absolute cap. `bypassPermissions` is an explicit canary posture; it does not
prove that a future interactive approval workflow is preserved.

`--trusted-inbound-envelope` copies one signature-verified triggering event
into the ACP request as `_meta.buzz.inboundEvent` (`schemaVersion: 1`). The
envelope carries the signed event ID, author, kind, exact channel ID, and exact
tag arrays outside the model-visible prompt. It is omitted for invalid,
multi-event, cancelled/merged, or room-ambiguous batches; OpenClaw must preserve
this trusted per-turn context before the Nexus reply tool can be enabled.

Room admission uses two config rules per worker:

- the fixed private-office UUID accepts Architect-authored kind-9 and
  kind-40002 stream posts without
  requiring a mention;
- a newly invited channel is added to the huddle rule only after canonical
  kind-39000 metadata proves it is private, has a positive `ttl`, and has a
  valid future `ttl_deadline`; huddle messages must mention the Aspect.

Therefore Concilium and other ordinary rooms remain excluded. A metadata query
failure also denies admission. The canonical addressable metadata winner is
selected by newest `created_at`, then lowest event ID on a same-second tie.
Eligibility is rechecked before every dynamic-room dispatch and periodically;
expiry, archive, membership removal, or lookup failure unsubscribes the room,
drains queued work, invalidates its session, and cancels an in-flight turn.

Run the source validation without changing live state:

```sh
node deploy/local/aeon-aspects/validate.mjs
node deploy/local/aeon-aspects/render-launchagents.mjs
```

The checked-in `launchagents/` previews are real, deterministic launchd
definitions with `RunAtLoad=false` and `KeepAlive=false`. They contain only
owned private-key file paths and expected public keys, never key or token
values. `/REQUIRES_FLEET/...` paths intentionally make them non-runnable until
Fleet supplies an immutable OpenClaw binary and owned token file. `buzz-acp`
opens its private-key file once with no-follow semantics, validates metadata on
that same handle (absolute path, regular file, current-user owner, mode `0600`),
and verifies that it derives the expected Aspect pubkey. The token-file contract
requires the same path, type, ownership, and permission posture, with Fleet
responsible for the runtime permission readback.

## Nexus resume and rollback packet

Activation remains Fleet-gated. After the runtime lock is released, the
operator must pre-create `agent:main:buzz-private`, provide an owned token file,
provide the immutable Gateway generation and current OpenClaw ACP flag proof,
prove that the fixed Nexus session has a caller-bound outbound Buzz publisher
which signs as Nexus without accepting an arbitrary Aspect selector, verify the
live private room contains exactly Architect and Nexus, confirm the legacy
`aeon-buzz` private bridge is off, confirm no Gateway switch/restart is active,
render the Nexus worker, and request a separate Nexus-only canary authorization.
Rollback targets only
`org.aeon.buzz-acp.nexus`; it does not change Gateway configuration or enable
another reply path.

## Evidence boundary

The Rust runtime now carries the triggering request event ID through the turn,
cryptographically verifies relay reply evidence, requires exactly one kind-9
reply with the exact NIP-10 `reply` anchor instructed in the prompt, and checks
the observed Gateway session key against the worker's fixed expected key. The session key is stable
session-level evidence and is intentionally retained across turns; `runId` is
cleared before each prompt and must be observed anew. Zero or multiple replies,
missing evidence, or a session mismatch produce one failed `turn_receipt`; they
never become a successful receipt or retry an otherwise completed user turn.
The request event ID remains the receipt correlation key even when a follow-up
turn is deliberately flattened onto the thread root; a two-turn contract test
proves that prompt instruction and relay evidence query use that same root.

Current OpenClaw ACP exposes `_meta.sessionKey` but does **not** yet expose its
Gateway `runId` on the ACP wire. Therefore complete receipts remain blocked
until Fleet supplies a version whose per-prompt session evidence includes both
`sessionKey` and `runId`, with a fixture proving the exact wire contract.
Observer frames are encrypted transient evidence transport, not the durable
receipt store; the canary must materialize the verified tuple into the existing
vault receipt.

The package also does not by itself grant OpenClaw the Aspect private key or a
Buzz CLI environment. Before activation, Fleet must prove the caller-bound
Gateway outbound publisher described above on the exact fixed session. Without
that authority there is no demonstrated signed reply path, so this checkpoint
must remain held even if inbound ACP prompting succeeds.

Socket reconnect deduplication is covered by the existing in-memory event-ID
set and replay watermark. Exact-once behavior across a complete worker process
restart is not proven because processed event IDs are not durable. Activation
must keep this under `does_not_prove` until a durable inbox/outbox authority is
selected. `restartOnFailure=false` and launchd `KeepAlive=false` intentionally
block unattended production rather than claiming exactly-once behavior.

Relay observer receipts also admit existing Architect-signed cancel and model
control packets. `!cancel` maps to ACP cancellation, but `!rotate` only recreates
the Buzz ACP layer against the same fixed `--require-existing` Gateway session;
it is not Gateway memory compaction or a canonical session reset.

Huddle audio remains text-mediated through Desktop STT and TTS. Kind-48106
voice guidelines are posted before incremental agent membership and are not
replayed by the stream-message worker subscription, so guideline query/injection
is a named blocker before a spoken huddle canary; no raw-audio capability is
claimed here.

Avatar metadata is not present in the current AEON identity map. Source
validation checks names, pubkeys, owners, private-room IDs, and the identity-map
membership declaration, but that declaration is not live relay evidence. Exact
live membership must be read back before the Nexus canary, and avatar validation
remains an explicit blocker before broad UI rollout.

## Existing-feature parity

| Buzz surface | Worker integration | Remaining proof |
| --- | --- | --- |
| private chat, mentions, threads, deep links | kind-9 and kind-40002 intake; bounded thread/DM context; exact reply anchoring | one live Nexus request/reply and deep-link readback |
| presence, typing, seen reaction | native `buzz-acp` behavior, enabled by the pinned posture | online/typing/clear/offline lifecycle in only the Nexus room |
| agent activity and cancel | encrypted relay observer and turn receipts | Desktop observer readback plus one terminal receipt |
| profiles and avatars | native Desktop kind-0 rendering | six approved profile events; avatars are missing from the identity contract |
| canvas and scoped search | native Buzz context/CLI surfaces | caller-bound Gateway Buzz read tool on the fixed Aspect session |
| media and authored reactions | native relay/Desktop surfaces | caller-bound Gateway publisher and bounded mutation authority |
| huddle STT/TTS | private text path is admitted by the invited-huddle rule | canonical kind-48106 guideline replay before spoken use |
| workflows | native Buzz workflow surface, intentionally not subscribed here | upstream approval/multi-room behavior and explicit AEON authority |

The worker intentionally sets no Buzz MCP, model, system prompt, team
instructions, or initial message. Those would duplicate or override Gateway's
tools, skills, model, and Aspect identity. Concilium, workflows, raw audio,
media mutations, broad A2A speech, and the other five live workers remain held
past the Nexus text canary.
