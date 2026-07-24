# HMAS Supply Daily Command Brief Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task with fresh implementer and reviewer agents.

**Goal:** Turn the macOS Command Console into a truthful, offline-capable Daily Command Brief that sequentially commissions five specialist advisers through the local LM Studio-native runtime, asks a tool-free Chief of Staff to consolidate their validated evidence, schedules one idempotent 0600 local brief, and preserves every result in an encrypted owner-only signed Buzz audit record.

**Architecture:** The trusted Tauri/Rust boundary owns source collection, the frozen knowledge snapshot, model egress, scheduling, orchestration, validation, persistence, encryption, and signed relay publication. React requests work and renders immutable view models only. Five specialists run in a bounded local queue using catalog-admitted Memory and RAG MCP integrations; the Chief of Staff receives only validated structured contributions and a source ledger and has no tools. Every run is `OFFICIAL`, freezes one RAG snapshot, fails soft per unavailable input, retains dissent, creates only `pending` workspace proposals, and writes an encrypted append-only lifecycle record before the UI can report completion.

**Tech Stack:** Rust, TypeScript/React, Tauri 2, Buzz `buzz-agent` LM Studio-native client, LM Studio `/api/v1/chat`, Nostr/NIP-44, SQLite, Tokio, Chrono, React Testing Library-compatible DOM fixtures, Node test runner, Hermit.

## Global constraints

- Every brief, contribution, source, prompt, model route, lifecycle event, schedule record, and proposed action is `OFFICIAL`. Phase 4 has no `PUBLIC` brief path and no cloud fallback.
- Only the Rust boundary may construct adviser prompts, choose personas, load secrets, admit MCP integrations, call LM Studio, validate model output, persist a brief, or publish an audit event.
- Specialists are exactly `Operations`, `Navigation`, `Daily Routine`, `Reporting`, and `Plans`. `Chief of Staff` runs after them and receives no MCP integrations.
- Default model concurrency is one. The only configurable alternative is two; it must still use one configured local model and preserve deterministic specialist result order.
- The run freezes the active signed RAG snapshot ID before collection. A snapshot change during generation invalidates the run; one automatic restart is allowed, after which the run fails visibly.
- Retrieved and Apple/file text is untrusted evidence. It is delimited, bounded, source-labelled, and never concatenated into a system prompt as instructions.
- Every factual finding must reference source IDs that exist in the run's source ledger. Unsupported findings are rejected rather than displayed.
- Missing, denied, stale, conflicted, malformed, or unavailable sources degrade the relevant section and become limitations. They do not silently disappear and do not block unrelated sections.
- The Chief of Staff may consolidate and rank validated contributions, but cannot remove dissent, change evidence, add new factual claims, or execute tools.
- Proposed workspace actions remain `pending`. Phase 4 does not create tasks, edit canvases, schedule external work, send messages, route work, or mutate any external system.
- The Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.
- Scheduling is local-time based, defaults to 0600, uses the IANA/macOS timezone, is idempotent per `schedule_id:local_date`, and performs at most one same-day catch-up after wake or app start.
- A helper or launch agent may wake/open the app but cannot bypass identity unlock, source admission, LM Studio readiness, or any other authorization gate.
- Signed brief events are regular stored owner-`p`-gated events, NIP-44 encrypted to the owner, excluded from full-text search, and readable by event ID only to the authenticated owner.
- Encrypted event payloads contain the bounded final brief and lifecycle metadata, not raw retrieved passages, model reasoning, credentials, bearer tokens, or arbitrary prompts.
- Public APIs are documented; production Rust contains no new `unwrap()` or `expect()`; payloads, strings, arrays, retries, timeouts, response bodies, queue sizes, and persisted rows are bounded.

## Verified starting state

- Phase 3 is complete at `257ee8ee` and proves authenticated loopback Memory/RAG admission, a signed active RAG snapshot, conflict-aware local Memory, read-only Apple inputs, and offline service status.
- `buzz-agent` exposes `LmStudioRuntimeConfig`, `LmStudioNativeClient`, `LmStudioChatRequest`, native structured tool-call parsing, and the catalog-owned command evidence gate. `buzz-desktop` already depends on `buzz-agent`.
- The TypeScript domain has strict Phase 1 `SourceReference`, `AdviserContribution`, `CommandBrief`, and `ProposedWorkspaceAction` contracts plus a bounded capacity-one-or-two `LocalRunScheduler`; these are not yet trusted execution code.
- The current Command Console UI truthfully reports infrastructure status but labels all six adviser roles as non-operational placeholders.
- Kind `44210` is unused in the authoritative `crates/buzz-core/src/kind.rs` registry. Kind `44200` demonstrates the required encrypted owner-`p`-gated, regular stored, unsearchable audit pattern.
- Phase 4 branch `codex/phase-4-daily-command-brief` is stacked on `codex/phase-3-knowledge-productivity`.

---

### Task 1: Define the canonical native brief, provenance, and lifecycle contracts

**Files:**

- Add: `desktop/src-tauri/src/command_brief/mod.rs`
- Add: `desktop/src-tauri/src/command_brief/types.rs`
- Add: `desktop/src-tauri/src/command_brief/types_tests.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src/features/command-console/domain/contracts.ts`
- Modify: `desktop/src/features/command-console/domain/contracts.test.mjs`
- Add: `desktop/src/features/command-console/domain/briefContracts.ts`
- Add: `desktop/src/features/command-console/domain/briefContracts.test.mjs`

**Interfaces:**

- Rust serializable types: `AdviserId`, `BriefSection`, `SourceLedgerEntry`, `CitedFinding`, `AdviserContribution`, `SourceFreshness`, `CommandBrief`, `PublishedCommandBrief`, `BriefRunState`, `BriefRunStatus`, `BriefSchedule`, and `BriefLifecycleRecord`.
- TypeScript display contracts mirror the exact Rust JSON wire shape and reject extra keys, unknown advisers/sections/states, unsafe classification inheritance, bad timestamps, duplicate IDs, missing citations, mixed snapshots, and non-pending proposals.
- The nine canonical sections are `today`, `operations`, `navigation`, `daily_routine`, `reports`, `planning_30_60_90`, `decisions`, `conflicts_and_gaps`, and `sources`.

**Steps:**

1. Write failing Rust tests for exact JSON casing, `OFFICIAL` defaults, all closed enums, duplicate source IDs, dangling citations, duplicate adviser identities, missing specialist contributions, mixed snapshot IDs, stale-source referential integrity, confidence bounds, bounded text/arrays, and pending-only actions.
2. Write failing tests proving a `Navigation` contribution cannot mark a finding as an order or decision and that the final brief always carries the advisory limitation.
3. Extend `SourceReference` with a stable ledger ID and source kind while preserving its document/chunk/snapshot/quoted-location metadata. Add `CitedFinding { text, source_ids }` so evidence is referential rather than positional.
4. Extend `CommandBrief` with generated time, run/schedule identity, snapshot ID, the nine sections, degraded sections, missing information, dissent, source ledger, and retrieval timestamps. Wrap it after signing as `PublishedCommandBrief { brief, lifecycle_audit_event_id, publication_state }`; the signed event ID must never be embedded in the ciphertext from which that same ID is derived.
5. Make Rust `TryFrom` validation the authority. Keep TypeScript parsers exact and immutable as persistence/display guards.
6. Add property-style fixture loops for every bounded field at its limit and one past its limit; reject control characters and prototype-pollution keys.
7. Run focused Rust and Node contract tests.
8. Commit only Task 1 changes.

### Task 2: Pin the six personas and bounded prompt/evidence policy

**Files:**

- Add: `desktop/src-tauri/src/command_brief/personas.rs`
- Add: `desktop/src-tauri/src/command_brief/personas_tests.rs`
- Add: `desktop/src-tauri/src/command_brief/provenance.rs`
- Add: `desktop/src-tauri/src/command_brief/provenance_tests.rs`
- Modify: `crates/buzz-agent/src/command_evidence.rs`
- Modify: `crates/buzz-agent/src/command_evidence_tests.rs`

**Interfaces:**

- `PersonaDefinition` contains the fixed adviser ID, purpose, permitted sections, permitted source kinds, permitted tool labels, output schema instruction, and safety boundary.
- `EvidenceEnvelope` wraps each source as inert evidence with ledger ID, source metadata, snapshot, observation time, bounded quote, and explicit `untrusted_evidence=true`.

**Steps:**

1. Write failing tests for the exact six-persona roster, specialist ordering, no tools for Chief of Staff, Navigation advisory wording, and rejection of mutable renderer-supplied prompts/persona names.
2. Write failing prompt-injection tests using retrieved text that requests policy changes, cloud egress, tool expansion, hidden instructions, and navigation orders. Prove it remains quoted evidence and cannot alter the system prompt or output schema.
3. Implement fixed system prompts that require structured JSON only, cite source ledger IDs, state limitations, preserve dissent, and create only pending proposals.
4. Add per-persona source/tool policy: Operations, Navigation, Reporting, and Plans may read admitted RAG and Memory; Daily Routine may additionally receive admitted Apple inputs; Chief of Staff receives only validated contribution JSON and the source ledger.
5. Extend the native evidence gate to return validated source records for orchestration while continuing to reject pseudo-tool calls, mixed snapshots, stale RAG, conflicted Memory, and Apple/file data outside allowlists.
6. Bound each evidence envelope and the total prompt budget. Deterministically truncate by source priority and record every omitted source as a limitation.
7. Run focused `buzz-agent` and Tauri tests.
8. Commit only Task 2 changes.

### Task 3: Build the Rust LM Studio adviser executor

**Files:**

- Add: `desktop/src-tauri/src/command_brief/lmstudio.rs`
- Add: `desktop/src-tauri/src/command_brief/lmstudio_tests.rs`
- Modify: `desktop/src-tauri/src/command_services/policy.rs`
- Modify: `desktop/src-tauri/src/command_services/policy/catalog.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime/lmstudio.rs`

**Interfaces:**

- `AdviserExecutor::run_specialist(request, cancellation)` calls `LmStudioNativeClient::chat` with the exact catalog-admitted Memory/RAG integrations.
- `AdviserExecutor::run_chief_of_staff(request, cancellation)` uses the same local model with an empty integration list and returns consolidation-only structured output.
- `AdviserExecutionResult` carries the validated contribution, model instance, response ID hash, token counts, executed tool evidence identities, start/end timestamps, and redacted diagnostics.

**Steps:**

1. Write failing fake-server tests for a valid native terminal message, executed structured MCP calls, reasoning-text pseudo-calls, malformed JSON, extra keys, wrong adviser, unsupported citations, tool from an unapproved server, plugin tools, response too large, redirect, timeout, cancellation, and non-loopback endpoint.
2. Add a catalog method that produces an immutable `LmStudioRuntimeConfig` and exact wire integrations from already-admitted local services and Keychain credentials. No renderer or persona field may supply an endpoint, header, token, or tool list.
3. Reuse `LmStudioNativeClient`, `LmStudioChatRequest`, and `LmStudioOutput`; do not create a second HTTP policy implementation.
4. Parse the terminal native message into the Rust contribution type and validate every cited source against the run ledger before returning success.
5. For Chief of Staff, construct a second validated runtime with no integrations, reject any returned tool call, and reject any new source ID or factual finding absent from specialist inputs.
6. Hash provider response IDs before persistence and redact tokens, prompts, evidence content, and LM Studio error bodies from diagnostics.
7. Run focused executor tests against a fake literal-loopback server and the existing native client suite.
8. Commit only Task 3 changes.

### Task 4: Collect and freeze local sources with fail-soft section degradation

**Files:**

- Add: `desktop/src-tauri/src/command_brief/sources.rs`
- Add: `desktop/src-tauri/src/command_brief/sources_tests.rs`
- Modify: `desktop/src-tauri/src/command_services/apple_inputs.rs`
- Modify: `desktop/src-tauri/src/command_services/memory.rs`
- Modify: `desktop/src-tauri/src/command_services/rag.rs`

**Interfaces:**

- `SourceCollector::freeze()` returns `FrozenSourceContext { snapshot_id, observed_at, ledger, degraded_sections, limitations }`.
- Collection covers current-day Calendar/Reminders, approved Notes/files, local Memory context, RAG status/catalogue, and retrieval query templates for the five specialists.

**Steps:**

1. Write failing tests for fresh success, permission denial, stale Apple data, deleted/recurring events, unavailable Memory, unresolved Memory conflicts, missing RAG, stale/invalid RAG, snapshot change during collection, duplicate source IDs, mixed snapshot IDs, and source-size truncation.
2. Read and cryptographically verify the active RAG snapshot before any adviser runs. Bind every RAG query/result to that ID and recheck it before consolidation and persistence.
3. Generate fixed bounded retrieval intents from the CO's request and persona definition; renderer text can supply the request but cannot supply MCP method names, filters, collection names, or source metadata.
4. Collect Apple data through the signed helper with date windows and allowlists from protected configuration. Map denial/failure to only the affected sections.
5. Read Memory through the conflict-safe command context. Exclude conflicted fields from the ledger and add a visible conflict limitation.
6. Canonicalise and deduplicate sources, assign stable run-scoped ledger IDs, preserve retrieval/observation timestamps, and label all content untrusted.
7. If the RAG snapshot changes, cancel work and permit the orchestrator one full restart. A second change fails with a visible `snapshot_changed` status.
8. Run focused source-collector and existing command-service tests.
9. Commit only Task 4 changes.

### Task 5: Orchestrate the five specialists and tool-free Chief of Staff

**Files:**

- Add: `desktop/src-tauri/src/command_brief/scheduler.rs`
- Add: `desktop/src-tauri/src/command_brief/scheduler_tests.rs`
- Add: `desktop/src-tauri/src/command_brief/orchestrator.rs`
- Add: `desktop/src-tauri/src/command_brief/orchestrator_tests.rs`
- Modify: `desktop/src-tauri/src/app_state.rs`
- Modify: `desktop/src-tauri/src/app_state_tests.rs`

**Interfaces:**

- `CommandBriefOrchestrator::start(request)` returns a unique run ID and exposes status/cancel operations.
- One app-owned `LocalModelScheduler` accepts capacity `1|2`, rejects duplicate run/adviser IDs, keeps running capacity until abort-aware work settles, and emits lifecycle changes.
- Run states are `queued`, `collecting_sources`, `running_specialists`, `consolidating`, `persisting`, `completed`, `degraded`, `cancelled`, and `failed`.

**Steps:**

1. Write scheduler tests for FIFO capacity one, bounded capacity two, stable result ordering despite concurrent completion, duplicate rejection, queued cancellation, running cancellation, abort-ignoring task capacity retention, and panic/error isolation.
2. Write orchestrator tests proving five specialists run before Chief of Staff, all specialists see the same snapshot, Chief of Staff has no tools, missing adviser output becomes visible degradation, dissent survives verbatim, and unsupported Chief of Staff additions fail validation.
3. Implement one run state machine with bounded in-memory status history and a cancellation token propagated through source collection, LM Studio requests, and persistence.
4. Produce all nine brief sections. Consolidation may rank supported findings, but the final source ledger and complete dissent list are assembled by trusted Rust from validated specialist records.
5. Carry failed adviser identities and source errors into `degraded_sections`, `missing_information`, and `limitations`; never fabricate an empty healthy section.
6. Permit one restart only for a changed RAG snapshot. Other failures are not automatically retried because native chat may have executed tools or stored response state.
7. Store no hidden reasoning. Retain only bounded status metadata and validated final outputs.
8. Run focused orchestrator/scheduler tests with deterministic fake executor and collector implementations.
9. Commit only Task 5 changes.

### Task 6: Persist and publish encrypted owner-only signed brief lifecycle events

**Files:**

- Add: `crates/buzz-core/src/command_brief.rs`
- Add: `crates/buzz-core/src/command_brief_tests.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-core/src/filter.rs`
- Add: `docs/nips/NIP-CB.md`
- Add: `desktop/src-tauri/src/command_brief/store.rs`
- Add: `desktop/src-tauri/src/command_brief/store_tests.rs`
- Add: `desktop/src-tauri/src/command_brief/audit.rs`
- Add: `desktop/src-tauri/src/command_brief/audit_tests.rs`
- Modify: `desktop/src-tauri/src/events.rs`
- Modify: `desktop/src-tauri/src/archive/mod.rs`
- Modify: `desktop/src-tauri/src/archive/pipeline.rs`
- Modify: `desktop/src-tauri/src/archive/store.rs`
- Modify corresponding relay and search tests for gated persistent kinds

**Interfaces:**

- Reserve `KIND_COMMAND_BRIEF = 44210` as a regular stored append-only event.
- Content is NIP-44 v2 ciphertext from the owner to the owner's public key. Tags are exactly one `p` owner tag, one `d` run ID tag, one `status` lifecycle tag, and optional `previous` event ID.
- `CommandBriefEventPayload` contains version, classification, run/schedule IDs, lifecycle state, timestamp, frozen snapshot, bounded final brief or redacted failure metadata, and previous lifecycle event ID. The post-signing event ID lives only in `PublishedCommandBrief` and the local spool row, avoiding a cryptographic self-reference.

**Steps:**

1. Write core round-trip tests for encryption/decryption, wrong-key failure, exact tags, bounded payloads, lifecycle predecessor integrity, classification, and absence of raw prompts/reasoning/credentials.
2. Add `44210` to the authoritative kind registry, `P_GATED_KINDS`, `RESULT_GATED_KINDS`, known kinds, compile-time regular-kind assertions, relay authorisation, COUNT/id-filter gates, and search NULL-vector coverage.
3. Write relay tests proving unauthenticated and wrong-identity `REQ`, `COUNT`, kindless ID queries, search, and archive access cannot reveal event existence or content.
4. Add a protected local SQLite spool with owner/run/event primary keys, encrypted payload, publish state, bounded retry metadata, WAL, atomic schema migration, and backup compatibility.
5. Before UI completion, encrypt and sign the terminal lifecycle event, derive its event ID, commit the event plus ID to the local spool, and return a `PublishedCommandBrief` envelope. Offline publication remains queued and does not invalidate local completion.
6. On reconnect, republish idempotently by event ID. Reject conflicting lifecycle predecessors and never overwrite an earlier event.
7. Decrypt only after the current unlocked identity proves it owns the `p` tag. Return validated view models, not raw ciphertext or arbitrary JSON.
8. Add `NIP-CB.md` with wire format, access controls, lifecycle, privacy, retention, and forward-compatibility rules.
9. Run core, relay, search, archive, and Tauri audit/store tests.
10. Commit only Task 6 changes.

### Task 7: Add the local 0600 schedule, idempotency, and wake catch-up

**Files:**

- Add: `desktop/src-tauri/src/command_brief/schedule.rs`
- Add: `desktop/src-tauri/src/command_brief/schedule_tests.rs`
- Modify: `desktop/src-tauri/src/startup.rs`
- Modify: `desktop/src-tauri/src/app_state.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `scripts/backup-local-workspace.sh`
- Modify: `scripts/restore-local-workspace.sh`

**Interfaces:**

- `BriefSchedule { schedule_id, enabled, local_time, timezone, catch_up_same_day, concurrency }`, default `daily-command-brief`, enabled, `06:00`, current macOS timezone, catch-up enabled, concurrency one.
- Idempotency key is the exact UTF-8 string `<schedule_id>:<YYYY-MM-DD>` in the schedule timezone.

**Steps:**

1. Write deterministic clock tests for before/at/after 0600, daylight-saving gaps/folds, timezone changes, app restart, Mac sleep, duplicate timers, disabled schedule, locked identity, unavailable model, and next-day recovery.
2. Persist schedule settings and claim rows in the protected brief database. Acquire a unique claim before generation; the same local date can never generate a second scheduled brief.
3. On startup/resume, perform at most one same-day catch-up if the scheduled time has passed and no claim exists. Never generate prior-day backlog.
4. When identity, LM Studio, or mandatory local state is unavailable, record a visible deferred status and retry only on a relevant readiness transition with bounded attempts; do not bypass gates.
5. Expose explicit enable/disable, local time, and capacity-one-or-two settings. Reject invalid timezone/time/concurrency and renderer-supplied schedule IDs.
6. Add brief schedule/spool state to encrypted backup/export and restore validation.
7. Document that macOS may delay execution during sleep; the product promises same-day catch-up, not exact asleep execution.
8. Run schedule, startup, backup, and restore tests.
9. Commit only Task 7 changes.

### Task 8: Wire Tauri commands and build the Daily Command Brief UI

**Files:**

- Add: `desktop/src-tauri/src/commands/command_brief.rs`
- Modify: `desktop/src-tauri/src/commands/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Add: `desktop/src/shared/api/tauriCommandBrief.ts`
- Add: `desktop/src/features/command-console/hooks/useDailyCommandBrief.ts`
- Add: `desktop/src/features/command-console/hooks/useDailyCommandBrief.hook.test.mjs`
- Add: `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
- Add: `desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs`
- Add: `desktop/src/features/command-console/ui/AdviserContributionCard.tsx`
- Add: `desktop/src/features/command-console/ui/SourceLedger.tsx`
- Add: `desktop/src/features/command-console/ui/BriefScheduleControls.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs`

**Interfaces:**

- Tauri commands: `get_command_brief_status`, `start_command_brief`, `cancel_command_brief`, `get_latest_command_brief`, `get_command_brief_schedule`, and `set_command_brief_schedule`.
- Tauri emits a bounded `command-brief-status-changed` event containing status metadata only.

**Steps:**

1. Write command tests proving every command requires the active unlocked owner identity, enforces `OFFICIAL`, validates input bounds, never accepts prompts/personas/tools/endpoints, and redacts secrets/errors.
2. Write UI tests for no brief, queued/running progress, manual refresh, cancellation, complete/degraded/failed briefs, offline queued publication, stale sources, missing information, dissent, citations, source timestamps, snapshot ID, nine sections, and schedule settings.
3. Implement thin Tauri command handlers over app-owned orchestrator state. Return immutable validated wire records and emit status events after state transitions.
4. Replace adviser placeholders with the real Daily Command Brief. Keep the prominent `OFFICIAL` banner and advisory/non-accredited warning.
5. Render adviser cards with confidence, findings, citations, limitations, dissent, and pending proposed actions. Do not render approval/execution controls until Phase 5.
6. Make each citation navigate within the brief to its source ledger record showing collection/document/chunk/location, retrieval time, and snapshot, without displaying hidden raw source text.
7. Add manual generation, cancel, schedule time, enabled, and concurrency controls with truthful readiness/degraded states. A failed source must not disable unrelated brief viewing.
8. Preserve keyboard navigation, accessible labels/live regions, reduced-motion behaviour, narrow-width layout, and no colour-only status meaning.
9. Run all Command Console Node tests, typecheck, lint/check, and focused Tauri command tests.
10. Commit only Task 8 changes.

### Task 9: Prove offline briefing, signed history, and Phase 4 acceptance

**Files:**

- Add: `scripts/check-daily-command-brief.sh`
- Add: `scripts/tests/check-daily-command-brief-test.sh`
- Add: `desktop/e2e/daily-command-brief.spec.ts`
- Add: `docs/command-console/phase-4-daily-command-brief.md`
- Modify: `Justfile`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**

- `just check-daily-command-brief` runs fixture acceptance by default and optional live probes only when explicit loopback configuration is supplied.
- The runbook records requirements, schedule semantics, source/readiness diagnostics, offline proof, signed audit behaviour, backup/restore, resource measurements, and known safety limits.

**Steps:**

1. Build a fixture harness with fake literal-loopback LM Studio, Memory, and RAG services plus Apple helper fixtures. Prove structured tool execution, five specialists, tool-free Chief of Staff, citations, dissent, one frozen snapshot, and a validated final brief.
2. Add negative fixtures for reasoning-text pseudo-tools, prompt injection, stale/mixed snapshots, conflicted Memory, denied Apple access, unsupported Chief of Staff claims, attempted cloud endpoint, and attempted non-pending action.
3. Add an offline acceptance path that blocks internet/home-LAN routes, restarts the local stack, generates a brief from the Mac mirror and local Memory, and verifies no outbound cloud, telemetry, webhook, updater, or home-LAN connection.
4. Prove schedule idempotency and wake catch-up with a deterministic clock; prove a manual refresh creates a separate explicit run without altering the daily schedule claim.
5. Prove encrypted event persistence, relay-offline spool, reconnect publication, owner-only read/COUNT/ID/search access, local history reload, and clean-profile backup restore.
6. Measure brief duration, peak RAM, storage growth, thermal observations, and LM Studio/RAG service health on the 64 GB MacBook; record results without inventing thresholds not observed.
7. Add CI fixture checks and `Justfile` targets. Live LM Studio, Apple privacy, network-disconnect, and signed-app checks remain explicit local acceptance gates.
8. Run `just check-daily-command-brief`, `just check-command-knowledge`, `just ci`, full Xcode helper tests, and the desktop smoke test. Run the live offline exercise when the approved local services are available.
9. Request an independent code review, fix all Critical and Important findings, rerun affected tests, and review the complete branch diff against Phase 3.
10. Record the final architecture, verified commands/results, event kind, schedule semantics, offline evidence, and gotchas in Memory MCP with agent `CODEX`.
11. Commit the final Phase 4 runbook/evidence, push `codex/phase-4-daily-command-brief`, and update the existing draft PR. Do not merge it.

## Phase 4 acceptance gates

- A manual or scheduled `OFFICIAL` brief runs only through the MacBook's LM Studio endpoint with catalog-admitted literal-loopback services and no cloud fallback.
- Five specialists run with default concurrency one and maximum two; Chief of Staff runs afterward without tools.
- The complete nine-section brief displays citations, source timestamps, one signed snapshot ID, missing/stale/conflicted inputs, degraded sections, and every dissenting adviser view.
- Navigation output is advisory, source-limited, and cannot become an executable order or decision.
- Every proposed action remains `pending` and no workspace or external system changes state.
- The 0600 schedule is idempotent by local date, survives restart, and performs at most one same-day catch-up after wake.
- Every terminal brief lifecycle event is locally durable, NIP-44 encrypted, owner-signed, `p`-gated, unsearchable, and queued safely while the relay is offline.
- With internet and home LAN disabled, the Mac-local stack can restart, read/write local Memory, retrieve the mirrored RAG snapshot, consume permitted local Apple/file inputs, generate a complete or truthfully degraded brief, and reload it from signed history.
- Backup/restore reproduces the brief store, schedule claims, Memory, RAG, and Buzz state on a clean test profile.
- Focused tests, upstream `just ci`, Xcode helper tests, fixture acceptance, live offline acceptance where services are available, and independent review pass before push.
