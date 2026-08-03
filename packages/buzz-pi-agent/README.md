# Buzz Pi Agent

`buzz-pi-agent` is a production ACP adapter that runs the Pi coding-agent SDK as
a first-class Buzz agent. Buzz owns identity, thread routing, and authenticated
session resets; Pi owns the model loop, tools, session transcript, compaction,
extensions, skills, and prompt templates.

The package is intentionally private to the Buzz monorepo. It pins
`@earendil-works/pi-coding-agent` to exactly `0.83.0` because the durable-session
and final-provider safety boundaries are version-sensitive.

## What it provides

- One durable Pi session and context per Buzz thread.
- An authenticated `/new` flow: Buzz issues a new reset token and the adapter
  atomically replaces only that thread's Pi context.
- A 150,000-token logical context ceiling with proactive compaction, final
  provider-context inspection, and visible lifecycle notices.
- A hard per-thread JSONL transcript quota. Pi compaction is append-only, so
  storage exhaustion fails closed with an explicit `/new` recovery path.
- `/context`, `/compact`, and `/reload` commands, plus ACP command descriptors
  for loaded Pi extension commands, prompt templates, and skills.
- Regular Pi global extensions, skills, prompts, context files, providers, and
  model configuration, with fail-closed project trust.
- Protocol isolation: Pi and third-party extensions run in a child process, so
  a stray extension write to stdout cannot corrupt ACP's NDJSON stream.
- Session-local model and thinking changes. The adapter reads normal Pi settings
  but discards all writes, leaving `~/.pi/agent/settings.json` byte-for-byte
  unchanged.

## Build and expose the command

From the Buzz repository root:

```bash
. ./bin/activate-hermit
pnpm install
pnpm --filter buzz-pi-agent build
cd packages/buzz-pi-agent
npm link
buzz-pi-agent --version
```

`npm link` is only a convenient way to put the locally built binary on `PATH`.
The desktop harness discovery also recognizes the `buzz-pi-agent` command.

For development:

```bash
pnpm --filter buzz-pi-agent check
pnpm --filter buzz-pi-agent test
```

The test suite includes actual Pi SDK session creation/resume, title persistence,
global and trusted/untrusted project resources, extension command/tool execution,
settings immutability, adversarial context guards, image payloads, compaction
lifecycle events, a real SIGKILL at the post-compaction/pre-notification crash
boundary, active-branch recovery, manifest/lease races, capacity, worker
timeout/crash recovery, and ACP wire fixtures.

## Thread and `/new` lifecycle

Buzz supplies a stable `_meta.buzz.conversationId` on `session/new`. The adapter
maps that identity to Pi's JSONL session file in a private manifest under
`~/.pi/agent/buzz/<account-namespace>/`. Reopening the thread resumes the same Pi
session, even after its live worker has been evicted or the adapter restarts.
ACP sessions without a Buzz `conversationId` are intentionally transient: they
have no durable route and cannot be resumed. Their JSONL is therefore deleted
on explicit disposal, LRU/TTL eviction, and graceful adapter shutdown, while
mapped thread transcripts remain available for later resume.
Unless `BUZZ_PI_NAMESPACE` is explicit, the account namespace is derived from
the canonical relay URL and canonical 32-byte Nostr secret. Equivalent `nsec`
and hexadecimal spellings, host casing, default ports, and root URL spellings
therefore cannot fork persistence or bypass an acknowledged reset barrier.

`/new` is deliberately not executed as an ordinary Pi slash command. Buzz
authenticates the user action and supplies an idempotent reset token on the next
`session/new`. A changed token creates a fresh file and atomically repoints the
thread. The same token is safe to replay. Late touches, releases, or forgets from
the old session are conditional and cannot overwrite the replacement.

Each mapping also carries a bounded durable lifecycle generation. Ordinary
continuity recovery may replace Pi's session ID because the workspace changed,
the JSONL became stale, or a lease was taken over; it preserves this generation,
so every unacknowledged notice still replays. Only an authenticated reset rotates
the generation. Late event writes include the emitting handle's captured
generation and are rejected after that reset boundary, even if an old worker is
still draining.

An acknowledged destructive reset also writes an atomic reset tombstone before
the old route is removed. If Buzz or the adapter restarts before the next
message, the tombstone forces the next token-less `session/new` to create fresh
Pi context and returns `_meta.buzz.skipRelayHistory: true`; old relay messages
therefore cannot silently repopulate the cleared context. The tombstone is
transitioned to a retained marker in the same manifest write that installs the
fresh mapping. That mapping carries its own durable `relayHistoryCleared`
anchor. Retained markers are independently pruned by age and count; if an
anchored mapping is later pruned, its marker is reactivated before the mapping
is removed. Pending barriers are never expired while their safety is needed.
They have a strict capacity bound, and a reset fails before ACK when that bound
is full; opening any pending cleared thread installs its mapping and frees a
slot.

On macOS and Linux, manifest writes fsync the temporary file, atomically rename
it, and fsync the containing directory. Node cannot open Windows directories
for fsync, so Windows retains the file fsync and atomic rename but has weaker
directory-entry durability across sudden power loss; corrupt or missing
initialized state still fails closed on restart.

Buzz commits this barrier through the idempotent
`_buzz/conversation/reset { conversationId, resetToken }` RPC before ACKing a
reset. It works even after the hot ACP session was LRU-evicted, and is advertised
as `_meta.buzz.threadSessions.resetCommit: true` during initialization.

Cross-adapter resets are supported even while the old handle still owns a live
lease. The old JSONL is retained until that handle is actually disposed, then
removed only after the manifest is verified to point elsewhere.

The manifest and lock directories are `0700`; manifests and session files are
`0600`. Writes are atomic and byte-bounded before rename. Corrupt or oversized
manifests fail closed without being replaced, so acknowledged reset boundaries
can never silently become empty state. Session paths are confined to approved
roots with symlink checks, and live-PID leases prevent two runtimes from writing
one JSONL concurrently. Leases include hashed host and
boot identities: a hard-killed local adapter is recovered immediately, while a
foreign-host or legacy lease is conservatively honored until its TTL expires.

Each Pi JSONL also has a byte-accurate 64 MiB ceiling by default. The guard is
installed at Pi's append/rewrite seam and checked before a persisted file is
opened. Ordinary transcript bytes and a 64 KiB Buzz control reserve are
accounted as separate partitions, so rollback, compaction-attempt, lifecycle
watermark, pending-delivery, and ACK markers cannot steal ordinary headroom.
Every control type has its own schema and byte bound; both partitions and their
combined size are checked before every append/rewrite. Reaching the limit
returns a non-retryable `session_storage_limit` error with `/new` as the
recovery action; compaction cannot reclaim these bytes because Pi transcripts
are append-only.

## Context limit and compaction

The default logical ceiling is 150,000 tokens. With the default 16,384-token
reserve, proactive compaction begins around 133,616 tokens. For a model with a
smaller window, the effective ceiling and compaction settings are automatically
clamped to that model; retained recent context is capped at 75% of the effective
threshold so the summary itself always has meaningful room.

Before a turn, the adapter estimates the full provider-facing context, including
the system prompt, transcript, tool schemas, incoming text, and a conservative
4,096-token budget per image. It compacts when the next request approaches the
threshold. It also guards Pi's final transformed context and serialized provider
payload immediately before dispatch, so extension-added context and tools cannot
bypass the policy. Raw `before_provider_request` hooks may observe a payload but
may not mutate it.

There is no provider-independent exact tokenizer for every Pi provider and
custom provider extension. Accordingly, 150k is an enforced logical safety
ceiling based on Pi/provider usage for completed turns plus conservative
full-request estimation; it is not a claim of mathematically exact tokenization
for every possible provider. The reserve and final guard are intentional safety
margin. Billing and accumulated usage remain provider-native.

An intrinsically oversized request is rejected before attempting a meaningless
empty-session compaction. Context refusals use JSON-RPC error `-32042` with:

```json
{ "kind": "context_limit", "retryable": false }
```

Blocked user/error entries are branched out of active Pi context, preventing
retry accumulation. Image binary is size/count checked but is not miscounted as
base64 text.

Buzz receives typed `_buzz/session/event` schema-v2 notifications for:

- compaction completed or failed, including a UUID, reason, before/after usage,
  retry status, and whether an extension supplied the result;
- `/context` status;
- session reset;
- extension/resource reload.

For a mapped Buzz thread, each notice is written to a private durable outbox
*before* it is published. The notification carries a stable UUID `eventId`, the
durable `conversationId`, the current outer ACP `sessionId`, and the typed event.
Initialization advertises
`_meta.buzz.sessionEvents = { supported: true, durableReplay: true, ack: true,
schemaVersion: 2 }`. Buzz first commits the notice to its own durable relay
outbox, then calls the idempotent
`_buzz/session/event_ack { conversationId, eventId }` RPC. Until that ACK, an
adapter restart or a newly opened outer ACP session replays the same event ID
under the new `sessionId`; Buzz can therefore deduplicate without losing the
user-visible compaction/reset status.

Compaction completion additionally uses a child-side handoff inside Pi's JSONL.
Before the isolated runtime publishes a completion, it appends a bounded
pending marker keyed by a deterministic delivery UUID. The parent persists that
same UUID in its outbox, publishes it, and only then ACKs the child marker. A
successful prompt waits for this handoff. If the child is killed immediately
after Pi appends its intrinsic compaction entry but before it can append or send
the notice, the next runtime reconciles that active-branch entry into the same
durable flow. A feature watermark prevents legacy compactions from being
backfilled, and orphaned Pi branches are never synthesized as current notices.
Losing a child ACK can only cause an idempotent replay; it cannot lose the
parent-durable event. This child ACK is distinct from Buzz's later relay-outbox
ACK described above.

The outbox never expires or silently drops an unacknowledged notice. It has a
hard global capacity (256 by default) and a 64 KiB per-record bound; reaching
either limit fails the live session closed. A mapping with any unacknowledged
notice is pinned against TTL and capacity pruning until Buzz ACKs it. An
authenticated reset supersedes all pending notices from the older lifecycle
generation. Manifest epoch checks also fence a late write from an older adapter
process, and replay retires only records from a reset-superseded epoch; a Pi ID
change inside the current epoch remains replayable. Legacy manifests derive a
stable epoch, and legacy outbox records without one are conservatively adopted
by the current mapping instead of being dropped during upgrade. Because schema
v2 is negotiated globally, an ACP session without a durable Buzz
`conversationId` suppresses typed lifecycle notifications instead of emitting an
incompatible schema-v1 frame. Its context accounting and compaction still run
normally; once Buzz supplies a stable conversation identity, notices use the
durable v2 path.

The adapter also emits provider-native usage updates. When current context is
temporarily unknown after compaction, it omits `used` instead of reporting a
misleading zero.

## Pi extensions and resources

By default the adapter uses the same Pi agent directory as regular Pi:
`~/.pi/agent`. Global extensions, skills, prompts, `AGENTS.md` context, custom
providers, `models.json`, credentials, and read-only settings are therefore
available. `PI_CODING_AGENT_DIR` can point at an isolated directory for testing.

This child-process boundary is **not an OS security sandbox**. Any loaded Pi
extension is trusted code with the agent process's filesystem/network
permissions and inherited credentials. This is intentional: Pi's bash/tools
must be able to run the `buzz` CLI to publish its responses and use other
agent-facing operations. Extensions share that same process and must therefore
be treated as fully trusted code. Reuse your regular extensions only when you
fully trust them. For a more isolated agent profile, set
`PI_CODING_AGENT_DIR` to a dedicated directory containing only reviewed
resources (and separately configure the credentials and providers that profile
needs). Project trust protects only project-local resource discovery; it cannot
make a malicious global extension safe. Isolating untrusted extensions while
retaining privileged Pi tools would require a separate parent credential broker.

Project-local `.pi` resources are executable configuration and fail closed in
headless mode. They load only when the regular Pi `ProjectTrustStore` already
contains an affirmative decision for that workspace, or when an operator sets
`BUZZ_PI_TRUST_PROJECT=true`. `/reload` re-evaluates trust immediately before
discovery, so a project cannot gain trust merely by creating resources after
startup.

The adapter resolves the requested workspace to one canonical directory before
it creates persistence, trust, settings, or Pi services. It retains both that
directory's device/inode identity and the original lexical path. If the path is
later repointed (including a trusted-to-untrusted symlink swap), `/reload` fails
closed. A Buzz conversation also cannot join a live or pending session created
for a different canonical workspace.

Pi's synchronously discovered non-code inputs are preflighted with per-file,
aggregate-byte, file-count, directory-entry, and recursion-depth budgets. This
includes settings/model/trust and package manifests, context files, skills,
prompts, themes, ignore manifests, configured paths, and package resource
trees. Every bounded file receives a content and filesystem-identity
fingerprint before the SDK load; the complete discovery snapshot must match
afterward, catching growth, rewrites, additions/removals, inode replacement,
and symlink swaps in the load window. Any reload/trust/resource failure poisons
and disposes that Pi generation. Further prompts are rejected until Buzz
creates a fresh session, so a partially reloaded extension runner is never
reused. This boundary is reported as JSON-RPC code `-32045` with
`{ "kind": "session_invalidated", "retryable": true }`, allowing Buzz to
discard the dead outer session and requeue work onto a newly created one.

Compatibility details:

- Extension tools, event hooks, provider registration, commands, prompts, and
  skills work. `/reload` hot-reloads them for subsequent turns. The adapter
  publishes ACP `available_commands_update` descriptors when the session is
  created. The current Buzz UI does not turn those descriptors into input
  autocomplete or refresh them in the middle of a session.
- Extension requests to create/fork/switch sessions are rejected because Buzz
  owns thread topology. Tree navigation inside the current Pi session remains
  available.
- Pi's interactive `ctx.ui` surface is intentionally absent (`hasUI=false`) in
  this headless adapter. Extensions that require terminal dialogs or widgets
  need a headless fallback.
- Provider-payload mutation hooks are rejected to preserve the context ceiling;
  observation-only hooks work.
- Pi caches imported extension modules by cwd/path. State created inside the
  exported extension factory is per session, but mutable module-top-level state
  can be shared across Buzz thread sessions, just as in regular Pi. A reload can
  create a new module instance while existing handlers retain the old one. Do
  not use module globals for thread-private data.

## Operational defaults

| Environment variable | Default | Purpose |
| --- | ---: | --- |
| `BUZZ_PI_CONTEXT_LIMIT` | `150000` | Logical context ceiling (maximum 150,000) |
| `BUZZ_PI_COMPACTION_RESERVE` | `16384` | Reserved output/safety space (maximum 149,999 and must remain below the ceiling) |
| `BUZZ_PI_KEEP_RECENT_TOKENS` | `24000` | Recent context retained by Pi compaction (maximum 149,999 and must remain below the threshold) |
| `BUZZ_PI_MAX_SESSIONS` | `12` | Adapter live-session backstop (maximum 128) |
| `BUZZ_PI_SESSION_TTL_MS` | `2700000` | Live idle TTL (45 minutes; maximum 24 hours) |
| `BUZZ_PI_SWEEP_INTERVAL_MS` | `300000` | Lease/TTL/prune sweep cadence (maximum 1 hour) |
| `BUZZ_PI_STATE_DIR` | `~/.pi/agent/buzz` | Durable mapping state |
| `BUZZ_PI_MAX_PERSISTED_CONVERSATIONS` | `512` | Persisted mapping capacity (maximum 512) |
| `BUZZ_PI_PERSISTED_CONVERSATION_TTL_MS` | `7776000000` | Mapping TTL (90 days; maximum 365 days) |
| `BUZZ_PI_MAX_PENDING_RESET_TOMBSTONES` | `512` | Uninstalled reset-barrier capacity (maximum 512) |
| `BUZZ_PI_MAX_RETAINED_RESET_TOMBSTONES` | `512` | Retained installed-marker capacity (maximum 512) |
| `BUZZ_PI_RESET_TOMBSTONE_TTL_MS` | `2592000000` | Retained installed-marker TTL (30 days; maximum 365 days) |
| `BUZZ_PI_CONVERSATION_LEASE_MS` | `3600000` | Advisory lease heartbeat window (maximum 24 hours) |
| `BUZZ_PI_MAX_SESSION_FILE_BYTES` | `67108864` | Hard per-thread Pi JSONL ceiling (64 MiB; 1 MiB minimum, 512 MiB maximum) |
| `BUZZ_PI_MAX_PENDING_SESSION_EVENTS` | `256` | Durable unacknowledged lifecycle-event capacity (maximum 512; records are never TTL-pruned) |
| `BUZZ_PI_RUNTIME_REQUEST_TIMEOUT_MS` | `6600000` | Prompt timeout (110 minutes; maximum 6 hours) |
| `BUZZ_PI_RUNTIME_CONTROL_TIMEOUT_MS` | `120000` | Create/model/reload control timeout (maximum 30 minutes) |
| `BUZZ_PI_RUNTIME_INTERRUPT_TIMEOUT_MS` | `1200` | Abort/steer/dispose/shutdown timeout (maximum 60 seconds) |
| `BUZZ_PI_MAX_LINE_BYTES` | `10000000` | ACP NDJSON frame bound (maximum 64 MiB) |
| `BUZZ_PI_MAX_ACTIVE_REQUESTS` | `64` | Concurrent non-control ACP request bound (maximum 512) |
| `BUZZ_PI_MAX_OUTPUT_QUEUE_MESSAGES` | `2048` | Ordered ACP stdout queue bound (maximum 8192) |
| `BUZZ_PI_MAX_OUTPUT_QUEUE_BYTES` | `16777216` | Ordered ACP stdout byte bound (maximum 64 MiB) |
| `BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_MESSAGES` | `1024` | Ordered runtime IPC queue bound (maximum 4096) |
| `BUZZ_PI_MAX_RUNTIME_IPC_QUEUE_BYTES` | `16777216` | Ordered runtime IPC byte bound (maximum 64 MiB) |
| `BUZZ_PI_MAX_RESOURCE_FILE_BYTES` | `1048576` | Per-file non-code Pi resource bound (1 MiB; maximum 16 MiB) |
| `BUZZ_PI_MAX_RESOURCE_TOTAL_BYTES` | `16777216` | Aggregate non-code resource bytes per discovery generation (16 MiB; maximum 64 MiB; must be at least the per-file bound) |
| `BUZZ_PI_MAX_RESOURCE_FILES` | `512` | Unique lexical non-code resource file count per generation (maximum 4096) |
| `BUZZ_PI_MAX_RESOURCE_ENTRIES` | `4096` | Directory entries inspected during resource/package discovery (maximum 16384; must be at least the file count) |
| `BUZZ_PI_MAX_RESOURCE_DEPTH` | `16` | Recursive skill/package/glob discovery depth (maximum 64) |
| `BUZZ_PI_TRUST_PROJECT` | unset | Explicit project trust override |
| `PI_CODING_AGENT_DIR` | `~/.pi/agent` | Pi resource/credential root; use a dedicated directory to avoid loading regular global extensions |
| `BUZZ_PI_LOG_LEVEL` | `info` | `debug`, `info`, `warn`, or `error` |
| `BUZZ_PI_NAMESPACE` | derived | Optional durable-account namespace |

Buzz's outer ACP pool normally keeps at most 8 hot sessions for 30 minutes. The
adapter's 12-session/45-minute limits are a slightly larger backstop, so normal
outer disposal happens first while durable mappings preserve every thread.

Invalid relationships (for example, reserve greater than the context ceiling,
recent tokens greater than the threshold, or interrupt timeout greater than the
control timeout) fail startup with a specific configuration error.

## Protocol notes

The command serves ACP v2 JSON-RPC as newline-delimited JSON on stdin/stdout.
Logs are stderr-only. Pi runs in an IPC child process and every non-interrupting
mutation is serialized per session; separate sessions remain concurrent. Abort
and steering can interrupt an active prompt. A worker crash or timeout invalidates
the whole worker generation, releases its conversation leases, and allows a
subsequent `session/new` to recover from the durable JSONL.

ACP stdout and runtime IPC are both callback/drain-driven, strictly ordered,
and bounded by message count and bytes. A disconnected transport, write error,
or saturated queue poisons that adapter/worker generation and terminates it;
responses and tool lifecycle frames are never reordered or silently dropped to
make room.

Disposal marks a session unavailable for prompts and reuse immediately, while
retaining its immutable conversation/generation route until buffered runtime
events have reached the parent outbox. Even a failed worker drains those pending
parent handoffs before its route is retired; a failed child ACK can therefore
cause a later idempotent replay, never loss of the parent copy.

`_buzz/conversation/reset` can be committed by a helper adapter process because
the manifest lock is cross-process. That commit invalidates the durable route,
but it cannot interrupt a prompt executing in a different owning process. Buzz
must cancel/drain that owner or suppress its stale generation before ACKing a
helper-committed reset; when the owner is in the same process, the registry
aborts and disposes it before returning success.

Adapter tests pin suppression for unmapped compaction/context events as well as
the mapped schema-v2 persist-before-publish, replay, generation-fencing, and
idempotent-ACK contract. Buzz's harness contract tests independently exercise
that same v2 boundary and ACK only after its own durable outbox commit.
