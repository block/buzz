# Design: Live per-channel MCP control for Buzz agents

**Status:** Proposal for contribution to `block/buzz`
**Date:** 2026-07-31
**Authors:** Moni (product), with Claude Code (design + verification)

---

## 1. Problem

Buzz agents receive their MCP (Model Context Protocol) tool set **once**, at `session/new`, and it can never change for the life of the session. Two consequences, both verified against a live Buzz install:

1. **No mid-conversation grants.** Mid-task, a user cannot give an agent a tool it lacks ("I need Razorpay now") without losing the agent's entire in-context working memory — the workaround today is rotating the session, which resets the agent to a small slice of channel history.
2. **Severe token waste.** Because Buzz passes an empty `mcpServers` list by default, agents fall back to their own global config (e.g. `~/.claude.json` with 13 MCP servers). Measured on a live session: **~64k tokens of unused tool schemas injected into every turn** — 255k cumulative tokens consumed by an 8-message channel.

## 2. Goals / non-goals

**Goals**
- Per-agent MCP configuration UI (Codex-style panel): choose which MCP servers each agent starts with.
- **Mid-conversation MCP toggle**: grant or revoke a tool in a live channel and have the agent continue the *same* conversation with the new tool set.
- Full conversational continuity across a toggle — the whole transcript, not a truncated window.
- Eliminate the unused-global-MCP token overhead on channels that have an explicit MCP set.
- Everything additive, in the existing architectural patterns of `buzz-acp`; no `unsafe`; DCO sign-off.

**Non-goals**
- Mid-*turn* tool injection (tools land at turn boundaries; see §5).
- ACP protocol changes (none required — the mechanism uses shipped adapter behavior).
- Cursor support (Buzz has no Cursor ACP bridge yet; out of scope).
- A general meta-MCP proxy/gateway (deferred to v2; see §9).

## 3. Key mechanism (verified in source)

ACP fixes `mcpServers` at session creation — **but `session/resume` also carries an `mcpServers` field, and both flagship adapters implement "resume with a changed MCP set" as a supported reconfiguration path.**

- **Claude adapter** (`@agentclientprotocol/claude-agent-acp`): `getOrCreateSession()` fingerprints `(cwd, mcpServers)`; on mismatch it tears down the SDK subprocess and recreates it with `resume: <same sessionId>` and the new servers, restoring the **full conversation transcript** via Claude Code's native resume. The in-code comment names this exact case ("…or MCP servers reconfigured"). — `acp-agent.js` lines 132–139, 3981–4010, 4266–4302.
- **Codex adapter**: `resumeSession` → `threadResume({config: createSessionConfig(cwd, dirs, request.mcpServers), threadId: sessionId})`. — `dist/index.js` lines 26252, 28643–28657.

The **session ID never changes**, so Buzz's `channel → session` bookkeeping is untouched, and continuity is the entire transcript (disk-backed), not Buzz's 12-message `context_limit`.

**Toggle semantics:** use `session/resume`, **not** `session/load` — `load` replays full history as `session/update` notifications before responding, risking the 60s request timeout on long conversations; `resume` skips replay with identical reconfiguration semantics.

## 4. Why not the alternatives (divergent pass)

An unconstrained solution-space search (8 routes) independently converged on this mechanism. The notable rejected alternatives:

- **Meta-MCP router/gateway (one fixed MCP that hot-mounts downstream servers via `notifications/tools/list_changed`)** — the zero-churn ideal, but it depends on each agent's MCP client honoring `list_changed` (verified present in the Claude CLI binary; probable for Codex; unknown for Goose). Higher complexity, and the degraded fallback (a coarse `call_tool(server, tool, args)`) loses per-tool schemas and permission granularity. **Deferred to v2**, composed cleanly on top of this design.
- **`session/fork`** — same mechanics but mints a new sessionId, forcing a rewrite of the channel→session map and littering the store with abandoned forks. Codex doesn't register `session/fork`. Not the common denominator.
- **Fresh session + harness-injected history (status quo)** — loses in-context tool results, file reads, and reasoning state. Remains the **fallback** for agents without resume support.

## 5. Design

### 5.1 Data flow

```
Desktop per-channel MCP panel (spawn config + live toggles)
   │  owner-signed, encrypted, freshness-checked control frame (existing path)
   │  { type: "update_mcp_servers", channelId, mcpServers: [...] }
   ▼
buzz-acp control dispatch  (lib.rs ~880, beside switch_model)
   ▼
handle_update_mcp_servers_control()
   ├─ record desired set:  desired_mcp: HashMap<Uuid /*channel*/, Vec<McpServer>>
   ├─ agent idle + supports resume → apply NOW:
   │      acp.session_resume(session_id, cwd, new_servers)
   │      (same session id; bookkeeping untouched)
   ├─ agent busy → mark channel dirty; apply at next turn boundary (no cancel)
   └─ agent lacks resume → invalidate (existing fresh-session fallback)
   ▼
observer.emit("control_result", { status:
   "applied_live" | "pending_turn_end" | "rotated_no_resume_support" | "unchanged" })
   → desktop renders toggle state honestly
```

Turn-boundary application is safe by construction: the agent is moved out of the pool during a turn, so the dirty flag is only consulted when the agent is idle. Unlike `switch_model`, an MCP change never cancels an in-flight turn — killing work to add a tool is a worse trade.

### 5.2 Changes in `crates/buzz-acp` (Rust) — all additive

1. **`acp.rs`** — add `session_resume(session_id, cwd, mcp_servers) -> Result<Value, AcpError>` sending `session/resume` (params per `zResumeSessionRequest`); in `initialize()`, record `agentCapabilities.sessionCapabilities.resume` (and `loadSession`) into a `resume_supported` flag, mirroring the existing `steering_supported` pattern.

2. **`config.rs`** — multi-server config: add `--mcp-servers-json` / `BUZZ_ACP_MCP_SERVERS` (JSON array) alongside the legacy single `mcp_command`. Extend `McpServer` to an enum covering stdio + http/sse (the ACP schema union; the Claude adapter advertises `mcpCapabilities {http, sse}`) — remote MCP servers (e.g. Razorpay) require it.

3. **`pool.rs`** — move the *effective* MCP set out of the immutable `PromptContext` into pool state: `mcp_overlay: Vec<McpServer>` + `mcp_dirty: HashSet<Uuid>` on `SessionState`. At the prompt-time session lookup, if a session exists and its channel is dirty:
   ```
   match agent.acp.session_resume(&sid, &ctx.cwd, effective_servers()).await {
       Ok(_)  => { clear dirty; proceed with SAME sid }
       Err(MethodNotFound) | !resume_supported
             => invalidate_channel(cid)          // fallback
                + control_result "rotated_no_resume_support"
       Err(e) => existing session-error path
   }
   ```
   `create_session_and_apply_model()` uses the channel's desired set (falling back to `ctx.mcp_servers`) instead of unconditionally `ctx.mcp_servers.clone()`.

4. **`lib.rs`** — new `update_mcp_servers` control-frame arm + `handle_update_mcp_servers_control()`, modeled on `handle_switch_model_control()` but never cancelling in-flight turns. The `desired_mcp` map is runtime-only (re-sent by the desktop on reconnect), consistent with `desired_model` semantics.

5. **`session/new` hygiene (the 64k-token fix, same PR)** — when a channel has an explicitly managed MCP set, include `_meta.claudeCode.options = {"strictMcpConfig": true, "settingSources": ["project"]}` in `session_new_full` params. Other agents ignore unknown `_meta` per JSON-RPC. Semantics: *unmanaged channel = legacy behavior (agent's own global config); managed channel = exactly what the panel shows.* **This must remain opt-in per managed channel** or users who rely on their global MCPs inside Buzz would silently lose them.

### 5.3 Desktop changes

- Per-agent MCP panel (spawn config): TOML/JSON list of servers, toggle each on/off.
- Per-channel live toggle list: emits the `update_mcp_servers` control frame; renders `control_result` status (live / pending until turn end / rotated).
- `managed-agents.json` gains `mcp_servers`; `runtime.rs` serializes it into `BUZZ_ACP_MCP_SERVERS` at spawn so grants persist across harness restarts.
- `agentControl.ts` gains `updateManagedAgentMcpServers(pubkey, channelId, servers)`, mirroring `switchManagedAgentModel`.

### 5.4 Post-toggle verification (required)

The reconfigure-on-resume behavior is an **implementation detail, not an ACP contract** — nothing in the schema promises `session/resume` applies a new tool set, and an adapter update could stop honoring it. Therefore the harness must **verify the grant landed** (observe the next turn's advertised tool list, or probe the adapter's MCP startup status) rather than trust the resume's success response. On verification failure, fall back to invalidate-and-rotate.

### 5.5 Per-agent support matrix

| Agent | Mid-chat toggle | Notes |
|---|---|---|
| Claude Code (claude-agent-acp) | ✅ Full (resume-swap) | Fingerprint teardown + native resume; verified |
| Codex (codex-acp) | ✅ Full (threadResume) | Always re-resumes — no change-detection, so suppress no-op grants |
| Goose | ⚠️ Capability-gated | Verify `session/load`/resume support; else fallback + honest UI message |
| buzz-agent (native) | ❌ `loadSession: false`, in-memory | Best served by a native `McpRegistry` hot-add (Buzz's own code) — a follow-up, and actually zero-churn |

## 6. Continuity accounting (honest ledger)

**Preserved across a resume-swap toggle:** the complete conversation (every message, tool call, tool result, file content — rebuilt verbatim from the session transcript); the session's model and mode; todo state; permission settings; the sessionId itself.

**Lost:** subprocess ephemera — background bash jobs die at teardown; persistent-shell cwd/env resets; any stateful previously-attached MCP server restarts (Buzz's dev-mcp is stateless per call, so in practice: nothing). Cost: one subprocess respawn (~2–5s, hidden in the idle path) and a possible prompt-cache rewrite on the next turn — a token/latency *cost*, not a memory loss.

**Both directions the user asked for are satisfied:** grant a tool mid-chat *and* keep the whole conversation. Proof chain: `session/resume` → fingerprint mismatch → `createSession({resume: sessionId, mcpServers: NEW})` → `query({resume, mcpServers})` → disk transcript reload.

## 7. Security

`update_mcp_servers` names a command to execute — it is remote-code-execution **by design**, so it must stay inside the existing owner-signed, encrypted, ±5-minute freshness envelope (the same path as `switch_model`). The desktop UI must make the grant explicit about *what* binary/URL will run — a friendly name pointing at an arbitrary command is a supply-chain phish.

## 8. Testing

Following the crate's existing test shapes:
- `session_resume` request-shape tests (beside the `session_new_full_*` tests).
- Fingerprint/no-op tests: same set sorted differently → **no** resume (the adapter fingerprint is order-sensitive for `args`/`env`; avoid spurious churn).
- Control-frame dispatch tests (beside the `switch_model` ones).
- Pool test: a respawned slot re-applies the channel's desired set on next session creation.
- **Integration test (the continuity guarantee):** run 20 turns with tool calls, grant a server, resume, assert recall of early-turn facts. This is the test that proves the headline claim.

## 9. Abandon / fallback triggers

Defined up front so the design degrades gracefully rather than silently:

1. **Resume mangles heavy sessions** (the adapter references known resume context-accounting issues) → reproducible history loss = pivot to Route C (gateway) as primary.
2. **Grant clustering on very long sessions** (full-context rebuild gets expensive at 150k+ tokens) → flip default to gateway; keep resume-swap as fallback.
3. **Upstream adapters remove reconfigure-on-resume** → Route C becomes the principled primary, since it depends only on the MCP spec's `list_changed` (a real contract).

## 10. v2 (out of scope, designed to compose)

A Buzz gateway meta-MCP (extending `buzz-dev-mcp`, which is already in every session with relay credentials) that hot-mounts downstream servers and emits `notifications/tools/list_changed`. Zero session churn, mid-turn toggling — for agents whose MCP client verifiably honors `list_changed`. The gateway would simply be one of the servers in the resume-swapped list, so the two designs compose.

---

## Appendix: evidence citations

| Claim | Source |
|---|---|
| `session/resume` request includes `mcpServers` | ACP SDK `zResumeSessionRequest`, codex-acp `dist/index.js:19573` |
| Claude adapter reconfigures MCP on resume (fingerprint teardown) | `acp-agent.js:132-139, 3981-4010` |
| Resume restores full transcript (same sessionId) | `acp-agent.js:4266-4302, 5217-5259` |
| Codex resume applies new MCP config | `dist/index.js:26252, 28643-28657` |
| `strictMcpConfig` ignores global MCP config | claude-agent-sdk `sdk.d.ts:1959`; `acp-agent.js:4103, 4154-4175` |
| Buzz sends `mcpServers` at `session/new`; default near-empty | `crates/buzz-acp/src/acp.rs:621-697`, `lib.rs:4179` |
| Control-frame path (owner-signed, encrypted, fresh) | `crates/buzz-acp/src/lib.rs:834-1005` |
| Claude CLI honors `tools/list_changed` | binary strings + `tengu_mcp_list_changed` telemetry event |
