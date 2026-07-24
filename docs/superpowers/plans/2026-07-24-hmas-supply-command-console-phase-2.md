# HMAS Supply Command Console Phase 2 Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task with fresh implementer and reviewer agents.

**Goal:** Add a distinct ACP-compatible LM Studio-native adviser runtime that keeps `OFFICIAL` inference local, uses structured native MCP integrations, exposes truthful health, and produces validated adviser contributions without treating reasoning text as tool calls.

**Architecture:** Keep `buzz-acp` as the signed Buzz-to-ACP harness and factor the existing `buzz-agent` ACP session machinery only where necessary to support a second packaged runtime identity, `buzz-lmstudio-agent`. The new runtime calls LM Studio's native `/api/v1/chat` and `/api/v1/models` endpoints, maintains native `response_id` state per ACP session, and treats only native `tool_call` output items as executed tool evidence. A Rust egress policy is evaluated before the HTTP client is constructed and again before every request; `OFFICIAL` permits only literal loopback LM Studio and loopback ephemeral MCP integrations, has no cloud fallback, disables environment proxies, and refuses redirects. The desktop runtime catalog remains the single source of configuration capability facts.

**Tech Stack:** Rust, ACP JSON-RPC, reqwest, serde, LM Studio native REST API v1, MCP integrations, Tauri 2, React/TypeScript, Node tests, Hermit.

---

## Verified baseline and constraints

- LM Studio `0.4.13+1` is installed on the target Apple Silicon Mac and its server currently answers on port 1234.
- The current LM Studio process listens on all interfaces and accepted an unauthenticated request. The application will call only the literal loopback address, but readiness must report this host-exposure condition as unsafe until the operator enables authentication and restricts host/network access.
- `GET /api/v1/models` currently reports `qwen/qwen3.6-27b` loaded with a 262,144-token context window.
- A native `POST /api/v1/chat` request returns typed `output` items, token statistics, and a stateful `response_id`.
- The native endpoint rejects arbitrary `tools` and `tool_choice`. It accepts MCP `integrations` and returns already-executed `tool_call` items.
- OpenAI-compatible tool calls can currently be induced only with model-specific reasoning settings. That path remains out of scope because its semantics and parser are not the approved native-MCP route.
- `/api/v1/chat` does not accept a caller-supplied assistant-message history. Multi-turn ACP sessions must use `previous_response_id`; a missing or invalid response ID must fail explicitly rather than silently reconstructing an incomplete conversation.
- LM Studio plugin IDs conceal their underlying server transport. For `OFFICIAL`, Phase 2 therefore permits only explicit `ephemeral_mcp` integrations whose URLs pass the Rust loopback policy. Preconfigured plugin integrations remain unavailable until their targets can be independently attested.
- Phase 2 proves the application-side egress boundary. Host-level containment of the separate LM Studio process, including update/telemetry suppression and redirect behavior inside LM Studio's own MCP client, remains a Phase 6 assurance exercise.
- The existing `buzz-acp` harness still exchanges signed prompts, results, and observer events through the configured Buzz relay. Phase 2 proves local-only model and MCP HTTP routing, not that all command content remains on the Mac. Real `OFFICIAL` adoption therefore requires the MacBook-local relay to be the configured authoritative relay; preventing an operator from pointing the harness at a remote relay is an assurance/adoption gate, not a property implied by the LM Studio provider.

## Scope and acceptance boundary

Phase 2 includes:

- A distinct packaged and discoverable `buzz-lmstudio-agent` ACP runtime.
- Native LM Studio model discovery, health checks, chat transport, stateful session continuity, and bounded response parsing.
- Exact MCP integration configuration with per-server tool allowlists.
- A fail-closed `OFFICIAL` egress policy below prompts and personas.
- Structured adviser-output validation using the Phase 1 contracts.
- A single-model scheduler abstraction defaulting to one adviser run, with a hard maximum of two.
- Unit, integration, desktop, and live local-runtime evidence.

Phase 2 excludes:

- RAG snapshot export/import, a local RAG service, Memory MCP replication, Apple data access, adviser orchestration, scheduled briefs, and workspace mutations.
- Automatic edits to LM Studio's global `mcp.json`.
- Cloud routing for `PUBLIC`; Phase 2 may model the route but does not enable the Phase 5 cloud action path.
- Any ship-control, navigation-control, communications, combat, logistics, or personnel-system integration.

### Task 1: Define the native LM Studio wire contract and bounded parser

**Files:**

- Add: `crates/buzz-agent/src/lmstudio.rs`
- Modify: `crates/buzz-agent/src/types.rs`
- Modify: `crates/buzz-agent/src/lib.rs`
- Add or modify focused unit tests in `crates/buzz-agent/src/lmstudio.rs`

**Steps:**

1. Write failing tests for native chat requests with a new conversation, a continued conversation, exact `ephemeral_mcp` integrations, reasoning settings, output-token/context limits, and no unsupported custom-tool fields.
2. Write failing parser tests for `message`, executed `tool_call`, `reasoning`, and `invalid_tool_call` output items; token statistics and `response_id`; duplicate or malformed fields; over-large bodies; and unknown item types.
3. Require `invalid_tool_call`, missing terminal message, invalid argument/output shapes, and malformed response IDs to fail closed with bounded diagnostics.
4. Prove that tool-looking text inside `message` or `reasoning` never becomes a structured tool call.
5. Implement serialisable request types and a bounded response parser without adding `unwrap()` or `expect()` to production paths.
6. Run focused Rust tests and formatting.
7. Commit only Task 1 changes.

### Task 2: Add the typed `OFFICIAL` runtime and egress policy

**Files:**

- Add: `crates/buzz-agent/src/egress.rs`
- Modify: `crates/buzz-agent/src/config.rs`
- Modify: `crates/buzz-agent/src/llm.rs`
- Modify: `crates/buzz-agent/src/types.rs`
- Add: `crates/buzz-agent/src/bin/buzz-lmstudio-agent.rs`
- Modify: `crates/buzz-agent/Cargo.toml`

**Steps:**

1. Write failing tests for exact `PUBLIC | OFFICIAL` parsing with omitted classification defaulting to `OFFICIAL`.
2. Write failing tests proving that `OFFICIAL` accepts only `http://127.0.0.1:<port>` and `http://[::1]:<port>` LM Studio endpoints, rejects DNS names including `localhost`, userinfo, fragments, non-HTTP schemes, wildcard/unspecified/private/LAN/public addresses, and any fallback provider.
3. Write failing tests for exact MCP integration parsing: only loopback `ephemeral_mcp`, non-empty unique server labels, explicit non-empty `allowed_tools`, no duplicate tools, no plugin IDs, no arbitrary headers in `OFFICIAL`, and strict size/count bounds.
4. Add a request-authorisation function used by model discovery, chat, continuation, and summary paths; tests must prove no HTTP request is emitted after a denial.
5. Construct the native HTTP client with environment proxies disabled, redirects disabled, bounded connect/read timeouts, and bounded response bodies.
6. Add `Provider::LmStudioNative` and a distinct `buzz-lmstudio-agent` entry point which refuses non-native providers and defaults to `OFFICIAL`.
7. Keep existing `buzz-agent` providers behaviorally unchanged.
8. Run focused Rust tests, regression tests for existing providers, Clippy, and formatting.
9. Commit only Task 2 changes.

### Task 3: Bridge ACP sessions to native state and executed MCP evidence

**Files:**

- Modify: `crates/buzz-agent/src/lib.rs`
- Modify: `crates/buzz-agent/src/agent.rs`
- Modify: `crates/buzz-agent/src/llm.rs`
- Modify: `crates/buzz-agent/src/types.rs`
- Modify: `crates/buzz-agent/src/wire.rs` only if an additive observer update is required
- Modify: `crates/buzz-agent/tests/fake_llm.rs`
- Add: `crates/buzz-agent/tests/lmstudio_native.rs`
- Modify: `crates/buzz-agent/tests/golden_transcripts.rs` as required

**Steps:**

1. Add a failing fake-server integration test for ACP `initialize` → `session/new` → first `session/prompt` → second `session/prompt`, asserting the second native request uses the first response's `response_id`.
2. Add failing tests for cancellation, LM Studio restart or expired response state, authentication failure, timeout, model-not-loaded behavior, malformed output, and session isolation.
3. Reject ACP-supplied stdio MCP servers for the native runtime. Native integrations come only from the validated runtime policy; `buzz-dev-mcp` must never be attached to an `OFFICIAL` session.
4. Map native executed `tool_call` items to read-only ACP observer/audit updates, including server label, tool name, arguments, returned output, and ordering, without asking `McpRegistry` to execute them again.
5. Preserve native reasoning as reasoning output only. A tool-looking reasoning string with no native `tool_call` item must produce zero tool execution updates.
6. Keep each ACP session's `response_id` private to that session and clear it on session teardown; refuse silent cross-session reuse.
7. Ensure summaries and handoffs do not fall back to an OpenAI-compatible route. If native state cannot safely support an existing helper, return an explicit capability error and document it.
8. Run the fake-server suite, existing golden transcript/regression tests, Clippy, and formatting.
9. Commit only Task 3 changes.

### Task 4: Register the runtime in the macOS desktop and expose truthful health

**Files:**

- Modify: `desktop/src-tauri/src/managed_agents/discovery.rs`
- Modify: `desktop/src-tauri/src/managed_agents/discovery/runtime_metadata.rs`
- Modify: `desktop/src-tauri/src/managed_agents/readiness.rs`
- Modify: `desktop/src-tauri/src/managed_agents/runtime.rs`
- Modify: `desktop/src-tauri/src/commands/agent_models.rs`
- Modify: `desktop/src-tauri/src/managed_agents/env_vars.rs`
- Modify: `desktop/src-tauri/src/managed_agents/agent_env.rs` only if packaging requires it
- Modify: `desktop/src-tauri/tauri.conf.json`
- Modify: `Justfile`
- Modify: `desktop/src/features/agents/AGENTS.md`
- Modify: `desktop/src/features/agents/ui/agentConfigOptions.tsx`
- Modify: `desktop/src/features/agents/lib/agentConfigCore.ts`
- Modify corresponding Rust and TypeScript tests

**Steps:**

1. Write failing Rust catalog/readiness tests for the distinct runtime identity, command, provider lock, model discovery capability, absence of a stdio MCP command, required native model/base configuration, and optional Keychain-backed LM Studio token.
2. Add `buzz-lmstudio-agent` to the Tauri external binaries, every platform-specific Just build/copy/setup loop, and the Rust runtime catalog. Keep capability facts in `KnownAcpRuntime`; do not add renderer-level runtime-ID checks.
3. Make the runtime provider-locked to `lmstudio-native`, model-switchable through ACP, and explicit that MCP integrations are native policy configuration rather than a desktop stdio MCP command.
4. Add read-only `/api/v1/models` discovery through the native egress policy and return downloaded/loaded state without claiming an unloaded model is ready.
5. Extend readiness to distinguish application installed, API unreachable, authentication required, no loaded model, configured model unavailable, and ready.
6. Report a wildcard-bound or unauthenticated server as a security warning; do not silently change the user's LM Studio global settings.
7. Project the catalog metadata through `agentConfigCore` and render the existing canonical configuration controls.
8. Update the nested `AGENTS.md` because runtime configuration behavior changes.
9. Run focused Tauri Rust tests, focused TypeScript tests, `just desktop-check`, and formatting.
10. Commit only Task 4 changes.

### Task 5: Add structured adviser validation and bounded single-model scheduling

**Files:**

- Add: `desktop/src/features/command-console/domain/adviserRuntime.ts`
- Add: `desktop/src/features/command-console/domain/adviserRuntime.test.mjs`
- Add: `desktop/src/features/command-console/domain/localRunScheduler.ts`
- Add: `desktop/src/features/command-console/domain/localRunScheduler.test.mjs`
- Modify: `desktop/src/features/command-console/domain/contracts.ts` only for proven parser gaps
- Modify: `desktop/src/features/command-console/domain/contracts.test.mjs` as required

**Steps:**

1. Write failing tests for extracting exactly one `AdviserContribution` from a native terminal message, including adviser identity, findings, evidence, confidence, limitations, dissent, and proposed actions.
2. Reject Markdown-fenced pseudo-JSON, extra prose, missing citations for factual findings, unsupported fields, invalid classifications, oversized data, and adviser-identity substitution.
3. Preserve dissent and limitations verbatim as data; the parser must not consolidate or discard them.
4. Write failing scheduler tests for FIFO fairness, sequential default operation, cancellation, error isolation, no starvation, and a configuration hard limit of two concurrent runs.
5. Implement a dependency-light scheduler for later Phase 4 orchestration; Phase 2 must not invent simulated adviser results or start workspace actions.
6. Add a `ModelRoute` builder for the native runtime whose `egressDecision` mirrors the Rust policy result for display, while documenting Rust as the enforcement authority.
7. Run focused Node tests, TypeScript checks, desktop checks, and formatting.
8. Commit only Task 5 changes.

### Task 6: Prove the native runtime end to end and document Phase 2

**Files:**

- Add: `scripts/check-lmstudio-native.sh`
- Add: `scripts/tests/check-lmstudio-native-test.sh`
- Modify: `Justfile`
- Add: `docs/command-console/phase-2-local-agent-runtime.md`
- Modify: `docs/command-console/phase-1-foundation.md` only for the forward link
- Modify: `desktop/tests/e2e/command-console.spec.ts` only if truthful health is surfaced in the console during this phase

**Steps:**

1. Write failing script tests for unreachable service, authentication required, no loaded model, configured-model mismatch, malformed response, and redacted diagnostics.
2. Add a non-mutating Just recipe that checks the native model catalog and performs an optional bounded chat smoke test without loading, unloading, downloading, or changing LM Studio configuration.
3. Run a live native chat against the loaded local model and capture response item types, model identity, and statistics without recording prompt content or secrets.
4. If a pre-existing read-only Memory or RAG HTTP MCP endpoint can be represented as a loopback ephemeral integration without configuration changes, prove one structured native tool call. Otherwise record the proof as blocked pending the Phase 3 local service and keep the runtime tests deterministic with a fake loopback MCP server.
5. Prove that reasoning-text pseudo-tool markup causes no tool execution and that only native `tool_call` output produces observer evidence.
6. Exercise the egress denial suite with a local capture server and proxy environment variables, proving denied `OFFICIAL` routes generate no request and valid requests do not use the proxy.
7. Document exact configuration, Keychain/token handling, MCP allowlists, offline behavior, security boundary, diagnostics, and Phase 3 dependencies.
8. Run `cargo test -p buzz-agent`, focused desktop/Tauri tests, `just desktop-check`, `just test-unit`, and `just ci`.
9. Request whole-phase code review, address all Critical and Important findings, and rerun affected tests.
10. Record verified Phase 2 decisions, live endpoints, version facts, and gotchas in Memory MCP with `agent="CODEX"`.
11. Commit remaining documentation/evidence, push the branch, and update the draft PR.

## Final verification

Run from the activated Hermit environment:

```bash
. ./bin/activate-hermit
cargo test -p buzz-agent
just check-lmstudio-native
just desktop-check
just test-unit
just ci
```

Also verify:

- `buzz-lmstudio-agent` is packaged and separately discoverable from `buzz-agent`.
- An omitted classification becomes `OFFICIAL`.
- `OFFICIAL` cannot select an OpenAI-compatible, LiteLLM, OpenAI, LAN, or public endpoint.
- The configured Buzz relay is the MacBook-local authoritative relay; Phase 2 must not describe local model routing as proof that an arbitrarily configured relay is local.
- Proxy environment variables and HTTP redirects cannot bypass the native runtime policy.
- The native runtime sends no `tools` or `tool_choice` fields.
- The second ACP prompt uses the prior native `response_id`.
- Only native `tool_call` items generate executed-tool observer evidence.
- Every configured MCP integration has an exact per-server tool allowlist.
- Runtime readiness never labels an unavailable or unloaded model as ready.
- Runtime readiness visibly warns when LM Studio is wildcard-bound or unauthenticated.
- Adviser output is rejected unless it exactly matches the Phase 1 structured contract.
- The scheduler defaults to one active local run and cannot exceed two.
