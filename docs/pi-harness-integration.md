# Pi harness integration for Buzz managed agents

- Status: live production canary and rollback validated; fleet rollout remains explicitly out of scope
- Kanban: `228ca147-4522-43b1-8222-7864db1fe9fa`
- Scope: one canary managed agent; no fleet rollout

Implementation evidence:

- SDK adapter: [`tools/pi-acp`](../tools/pi-acp/README.md);
- automated ACP lifecycle, steering, cancellation, usage, routing, typed-tool, replay, budget, and isolation tests;
- real AgentSession readiness plus QUESTION, Kanban, bounded UI, and fake-publication diagnostics;
- benchmark evidence: [`pi-acp-benchmark-2026-08-24.md`](pi-acp-benchmark-2026-08-24.md);
- live production evidence and rollback: [`pi-acp-production-canary-2026-08-25.md`](pi-acp-production-canary-2026-08-25.md).

## Decision summary

Introduce a Node.js sidecar named `pi-acp` that implements the ACP server expected by `buzz-acp` and embeds `@earendil-works/pi-coding-agent` through its SDK. Keep `buzz-acp` as the relay, identity, authorization, thread-routing, presence, deduplication, timeout and NIP-AM authority.

The first implementation is a sidecar, not a Pi fork and not a direct Pi process invocation from every Buzz message. A short-lived RPC prototype may validate protocol mapping, but the production candidate uses `createAgentSession()` so that the adapter can install typed Buzz tools, enforce per-event budgets, expose exact lifecycle events and control session resources.

Only one canary identity may select `pi-acp`. The existing `agent_command=codex-acp` remains the rollback path.

## Problem and evidence

The current path is:

```text
Buzz relay -> buzz-acp -> ACP/NDJSON -> codex-acp -> Codex -> shell Buzz CLI
```

`crates/buzz-acp/src/acp.rs` already owns ACP initialization, session creation, prompts, cancellation, steering negotiation, bounded NDJSON and usage normalization. `crates/buzz-acp/src/usage.rs` publishes normalized NIP-AM turn metrics.

Recent Linza canaries showed two different outcomes:

- a bounded Kanban question improved to 3 model calls, 2 tools and 68,056 processed tokens;
- one small UI workflow consumed 53 model calls, 53 tools and 2,980,336 processed tokens before visual review.

The latter was dominated by sequential model continuations, generic skill/bootstrap reads, repeated discovery and shell-based publication. Context limits and session rotation did not constrain the internal model/tool loop for one inbound event.

Pi exposes explicit `turn_start`, `turn_end`, tool lifecycle, `agent_settled`, usage, steering, abort, compaction, custom tools and resource loading. These are suitable enforcement points, but Pi is not inherently cheaper: a long-lived Pi session can also replay a large cached context. The pilot therefore requires task-scoped sessions and a restricted resource loader.

## Goals

1. Preserve Buzz identity, relay, authorization, thread semantics and at-most-once publication.
2. Enforce model/tool/token budgets inside the agent runtime rather than through prompt advice alone.
3. Replace shell publication and broad Kanban payloads with typed bounded tools.
4. Use a fresh or compact task-scoped Pi session instead of a long-lived workspace conversation.
5. Produce usage and cost telemetry compatible with current NIP-AM/dashboard behavior.
6. Support immediate per-agent rollback to `codex-acp`.

## Non-goals

- Fleet-wide migration in the pilot.
- Replacing `buzz-acp` relay orchestration with Pi.
- Sharing provider credentials across owners or hosts.
- Treating Pi's interactive TUI as the managed-agent process.
- Depending on a user's existing long-running interactive Pi session.
- Weakening Buzz permission, allowlist, audit, deduplication or deployment controls.

## Proposed components

```text
                                    +--------------------------+
Buzz relay <-> buzz-acp <-> ACP <-> | pi-acp Node.js sidecar   |
                                    |                          |
                                    | ACP facade               |
                                    | Session registry         |
                                    | Budget controller        |
                                    | Resource policy          |
                                    | Typed Buzz tools         |
                                    | Usage mapper             |
                                    +------------+-------------+
                                                 |
                                    Pi AgentSession SDK
                                                 |
                                      configured model/provider
```

### `buzz-acp`

Remains authoritative for:

- Nostr identity and owner authorization;
- relay subscription, mention filtering and thread context;
- event deduplication and concurrent-event policy;
- ACP process lifecycle and hard wall-clock deadlines;
- presence/typing and final NIP-AM publication;
- adapter-independent rollback.

No Pi provider credential is passed through Buzz events or persisted in relay data.

### `pi-acp`

A new packaged Node.js executable that:

- speaks strict ACP JSON-RPC/NDJSON on stdin/stdout;
- owns one `ModelRuntime` per process;
- maps ACP session IDs to Pi `AgentSession` instances;
- creates task-scoped sessions with a restricted `DefaultResourceLoader`;
- registers typed tools and a budget extension;
- translates Pi streaming, tool and usage events into ACP updates;
- never writes logs or diagnostics to stdout outside ACP frames.

### Pi session policy

Use `createAgentSession()` with:

- an explicit `cwd` supplied by the validated ACP session context;
- a dedicated `SessionManager` per Buzz thread/task;
- selected tools rather than all globally discovered tools;
- `DefaultResourceLoader` overrides for the exact system prompt, context files and allowed skills;
- automatic compaction enabled with a pilot threshold below provider context exhaustion;
- `thinkingLevel` and model selected from managed-agent policy.

Question sessions should be ephemeral or aggressively rotated. Implementation sessions may persist for one task thread, but never silently merge unrelated assignments.

## ACP mapping

| ACP operation/update | Pi operation/event | Required behavior |
|---|---|---|
| `initialize` | adapter capability declaration | Advertise protocol v2, filesystem/terminal capabilities actually supported, and explicit steering metadata only after tests pass. |
| `session/new` | `createAgentSession()` | Validate cwd/model/tool policy, create a fresh task session, return a stable adapter session ID. |
| `session/prompt` | `session.prompt()` | Accept one inbound Buzz event, attach budget state, stream updates, settle only on `agent_settled`. |
| steering extension | `session.steer()` | Queue after the current tool set; never drop the inbound Buzz event. |
| `session/cancel` | `session.abort()` | Abort model and tools, drain events, return `cancelled` within the Buzz deadline. |
| agent text deltas | `message_update:text_delta` | Emit ACP agent-message chunks without publishing directly to Buzz. |
| thinking deltas | `message_update:thinking_delta` | Emit only when ACP/Buzz policy permits; never persist hidden reasoning as a channel message. |
| tool start/update/end | Pi tool lifecycle | Preserve tool ID, name, status, bounded output and error state. |
| turn completion | `turn_end` | Increment the per-event turn counter and evaluate budgets. |
| prompt settled | `agent_settled` | Return ACP stop reason after retries/compaction/follow-ups are actually finished. |
| usage update | assistant usage/session stats | Emit cumulative input, cache-read, cache-write, output, provider total, cost, model and pricing identity where known. |

The adapter must not use Node `readline` for Pi RPC or ACP framing because Pi RPC specifies LF-only JSONL and permits Unicode line separators inside JSON strings. The SDK path avoids the extra RPC framing layer.

## Typed tools

### `buzz_reply`

Input:

```ts
{
  channelId: string;
  replyTo: string;
  content: string;
  mentionPubkeys?: string[];
}
```

Contract:

- reject blank/whitespace-only content before network access;
- bind `channelId` and `replyTo` to the inbound event context rather than trusting model-supplied alternatives;
- use an idempotency key derived from agent identity plus inbound event ID;
- allow one successful publication per inbound event;
- return a typed receipt containing the published event ID;
- reject a second semantically different publication unless the workflow explicitly authorizes a checkpoint followed by completion;
- preserve forum-vs-stream kind inference in Buzz, not in the model.

### `kanban_tasks`

Input permits only bounded filters (`project`, `sprint`, `status`, `channel`, `search`, `limit`). Output contains task ID, status, priority and title by default. Full descriptions/comments require an explicit task ID and separate read. Cached/stale provenance is returned as structured metadata.

### Coding tools

Question mode receives read-only tools. Assignment mode receives the smallest approved coding tool set. A mode transition replaces `session.agent.state.tools`; it does not rely only on a prompt saying not to mutate.

## Budget controller

Budget state is keyed by inbound Buzz event, not by ACP process lifetime.

Initial pilot limits:

| Mode | Model turns | Tools | Processed-token target | Action at threshold |
|---|---:|---:|---:|---|
| `QUESTION` | 3 | 3 | 75k | stop tools and produce one bounded answer/error |
| `INTAKE` | 6 | 8 | 200k | publish budget checkpoint/blocker and settle |
| UI `ASSIGN` before visual review | 12 | 15 | 800k | finish only an active targeted command, checkpoint, then abort/settle |

Implementation:

1. Increment turns on Pi `turn_end` and tools on `tool_execution_start`.
2. Aggregate provider usage from authoritative completed assistant messages; do not sum streaming snapshots repeatedly.
3. Before each tool, an extension `tool_call` handler checks the current budget and may return `{ block: true, reason }`.
4. On the first threshold, queue exactly one `session.steer()` budget instruction.
5. If another model turn begins without a checkpoint/settlement, call `session.abort()` and return `max_turn_requests` or a documented adapter-specific budget stop mapped to it.
6. Emit a structured budget metric so the dashboard distinguishes normal completion, checkpoint stop and forced abort.

Prompt guidance remains useful but is not the enforcement mechanism.

## Identity, auth and security boundaries

- Buzz private keys remain in the `buzz-acp`/Buzz CLI boundary. Pi tools receive a scoped capability or local broker handle, not raw key material.
- Provider credentials remain host-local and resolve through a dedicated Pi `ModelRuntime`. The pilot must document whether it uses the host owner's existing OAuth credential or a separate API key without copying it into managed-agent JSON.
- `pi-acp` must inherit the managed agent's effective permission profile and cwd. It must not treat Pi's lack of built-in permission popups as authorization.
- Project-local Pi extensions/skills are disabled by default for non-interactive operation unless the project is explicitly trusted and the resource is allowlisted.
- Tool output, ACP lines and error messages are bounded and credential-redacted.
- Typed publication tools validate owner, channel membership, reply target and idempotency server-side.
- Crash recovery must never replay a successful write because the model did not observe its receipt.

## Session and data ownership

| Data | Owner/source |
|---|---|
| Buzz channel/thread events | relay/Buzz |
| Agent identity and authorization | Buzz managed-agent record and local secret store |
| ACP session mapping | in-memory `pi-acp` registry, reconstructible |
| Pi task history | dedicated local session directory or in-memory for questions |
| Usage/cost truth | provider-reported Pi message usage, normalized by adapter/Buzz NIP-AM |
| Publication receipt/idempotency | Buzz-side typed tool/broker |
| Kanban state | Kanban AI |

A Pi session file is diagnostic local data, not workflow authority. A crash may recreate reasoning context from the Buzz thread and task checkpoint, but must not infer that an unreceipted write succeeded.

## Failure modes and recovery

| Failure | Required response |
|---|---|
| `pi-acp` missing/outdated | Desktop readiness reports adapter missing/outdated; do not start agent. |
| Pi provider auth unavailable | Stable setup payload; no model attempt or shell fallback. |
| ACP protocol/frame error | Fail closed, terminate sidecar, preserve inbound event for bounded retry. |
| Model/provider transient error | Pi retry policy within one bounded attempt budget; report final failure through ACP. |
| Tool hangs | AbortSignal plus Buzz hard turn deadline; kill descendants during cleanup. |
| Budget exceeded | Block new tools, steer checkpoint once, abort if not settled. |
| Empty answer | `buzz_reply` rejects it; final ACP completion without a publication becomes an observable delivery failure. |
| Duplicate/replayed prompt | Buzz dedup plus publication idempotency returns the prior receipt. |
| Sidecar crash after publish | Recreated session claims the existing receipt and does not republish. |
| Usage missing/non-monotonic | Preserve unknown fields and NIP-AM reliability semantics; never fabricate totals. |
| Pi session corruption | Start a fresh task session from authoritative Buzz/Kanban/git checkpoints. |

## Packaging and desktop discovery

Add a Pi runtime descriptor beside existing known ACP runtimes. Discovery must verify:

- exact `pi-acp --version` output and a minimum supported version;
- compatible Pi package version pinned in the sidecar lockfile;
- provider auth separately from adapter availability;
- no fallback from an invalid Pi adapter to a different executable under the same identity.

The release bundle includes the adapter or installs it through the existing reviewed runtime installer. Version probing must be process-bounded and free from the startup race previously observed with stale adapter readiness.

## Implementation phases

### Phase 0 — contract fixtures

- Capture ACP initialize/new/prompt/cancel/steer fixtures from `buzz-acp` tests.
- Define typed tool schemas and publication idempotency contract.
- Define usage mapping fixtures, including cache-read/write and unknown totals.
- Add benchmark corpus and immutable acceptance thresholds.

Exit: architecture and threat model reviewed; no runtime mutation.

### Phase 1 — RPC spike

Implement a disposable adapter over `pi --mode rpc --no-session` to prove:

- LF-only JSONL handling;
- prompt, stream, abort and usage translation;
- one bounded question with no Buzz write.

Do not ship the RPC spike. Its purpose is to remove protocol uncertainty quickly.

### Phase 2 — SDK sidecar

- Create `pi-acp` package and ACP facade.
- Embed `createAgentSession()` with restricted resources.
- Implement session registry, typed tools, budget controller and usage mapper.
- Add protocol/golden tests and subprocess cleanup tests.

Exit: local fake-relay and fake-model tests pass.

### Phase 3 — canary identity

- Add one non-critical canary managed agent or explicitly switch one approved identity.
- Preserve the same relay policy and use a separate diagnostic channel initially.
- Run QUESTION and INTAKE corpus before allowing coding tools.
- Enable one bounded UI assignment only after read-only gates pass.

Exit: benchmark report and exercised rollback.

### Phase 4 — decision

Independent review compares quality, safety, latency, calls, fresh/cache/output tokens, cost, failures and operator burden. Expansion requires explicit owner approval. Failure to meet any safety or duplicate-write gate rolls back the canary and leaves `codex-acp` as default.

## Test matrix

### Protocol

- initialize capability negotiation and unsupported version;
- new session with valid/invalid cwd and model;
- prompt streaming order and one final stop reason;
- steer while a tool is active;
- cancel during model, tool, retry and compaction;
- malformed/oversized NDJSON and stdout contamination;
- child/descendant cleanup.

### Publication safety

- blank/whitespace reply rejected;
- same event replay returns the same receipt;
- concurrent duplicate publish commits once;
- crash immediately before/after publication;
- owner/channel/reply rebinding rejected;
- forum comment kind preserved;
- checkpoint/completion multi-write policy explicit and tested.

### Budget

- threshold below/equal/above limits;
- multiple streaming usage snapshots counted once;
- cache tokens separated from fresh input;
- blocked tool receives a stable reason;
- one steering warning only;
- forced abort after ignored warning;
- budget reset for a new inbound event, not for a retry of the same event.

### Session/recovery

- question uses ephemeral/fresh session;
- unrelated task cannot inherit previous task context;
- compacted implementation session preserves branch/SHA/DoD;
- corrupted session starts from authoritative checkpoints;
- adapter restart does not duplicate a reply.

## A/B benchmark

Run the same prompts and repository bases through current `codex-acp` and `pi-acp`. Each case records:

- wall-clock pickup and completion latency;
- model turns and tool calls;
- fresh input, cache-read, cache-write and output tokens;
- provider-reported total and USD cost;
- tool-output bytes and largest output;
- correctness/DoD result;
- empty, duplicate or wrong-thread replies;
- unauthorized mutation attempts;
- operator interventions.

Corpus:

1. tool-free status question;
2. compact Kanban question;
3. Conductor intake/routing;
4. bounded two-file UI change through pre-review checkpoint;
5. cancellation during a long targeted test;
6. duplicate inbound event and crash-after-publish recovery.

Use a clean session per case and at least three repetitions where provider variability matters. Report both processed tokens and estimated cost; high cache hit rate does not excuse excessive turns.

## Rollout and rollback

1. Default remains `codex-acp`.
2. Canary changes only `agent_command` to `pi-acp` and records exact adapter/Pi versions.
3. Stop the canary before switching command; verify a single runtime for the identity.
4. Rollback restores `codex-acp`, restarts the identity and runs a one-question smoke test.
5. Never run `pi-acp` and `codex-acp` simultaneously for the same identity and relay.
6. Fleet expansion requires independent review, green benchmark gates and explicit approval.

## Open decisions

1. Whether the canary is a new identity or an existing low-risk Linza specialist.
2. Whether implementation sessions persist to a dedicated Pi directory or remain in memory with explicit Buzz checkpoints.
3. Provider credential strategy for packaged desktop installs.
4. Whether typed Buzz tools call a local broker API or a scoped CLI wrapper.
5. Whether budget stop maps to ACP `max_turn_requests` or a new negotiated metadata reason while retaining protocol compatibility.
6. Minimum supported Pi package and `pi-acp` adapter versions.

## Recommended next action

Review this architecture and choose the canary identity plus credential strategy. Then implement Phase 0 fixtures and a non-shipping RPC spike before adding the SDK sidecar to desktop discovery or release packaging.
