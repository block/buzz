# Agent usage reporting gaps by ACP harness

Status: draft (2026-10-20). Scope: what `buzz-acp` can publish in the NIP-AM
kind 44200 turn metric (`docs/nips/NIP-AM.md`) for each harness — tokens/cost,
tool calls by name (`tools`), and skill invocations by name (`skills`).

Evidence is adapter source read on 2026-10-20 plus one live `goose acp` run.
Line references drift; re-verify before relying on them.

## Summary

| Harness | Per-turn tokens | Cost | Tool names | Skills | Context % |
|---|---|---|---|---|---|
| buzz-agent | ✅ goose extension, cumulative → delta | ✅ | ✅ title = tool name, MCP `server__tool` | ✅ `load_skill` + `rawInput.name` | ❌ no window size |
| goose | ✅ goose extension (when client advertises it) | ✅ | ✅ via `_meta.goose.toolCall.toolName` (`ext__tool`) | ✅ `skills__load_skill` + `rawInput.name` | ✅ `used` / `contextLimit` |
| claude-agent-acp | ✅ `session/prompt` result `usage` | ✅ `usage_update.cost` | ✅ `_meta.claudeCode.toolName`, MCP `mcp__server__tool` | ✅ `Skill` + `rawInput.skill` / `_meta.claudeCode.skill` | ✅ `usage_update {used,size}` |
| codex-acp | ❌ none; metric is usage-null | ❌ | ⚠️ MCP `server/tool` ✅; builtins only as ACP `kind` buckets (usage-null record) | ❌ no skill tool call | ✅ `usage_update {used,size}` (usage-null record) |
| hermes | ✅ prompt result `usage`, session-cumulative → delta | ❌ | ⚠️ parsed from title prefix (`terminal: …` → `terminal`) | ❌ `skill_view` sends no `rawInput` | ✅ `usage_update {used,size}` |
| pi (`buzz-pi-acp` @ `86b201e`) | ❌ none; metric is usage-null | ❌ | ✅ bare titles; bash → `execute` (usage-null record) | ❌ skills are plain `read`s | ❌ none at pinned commit |

✅ works today on this branch · ⚠️ partial · ❌ not available from the wire

## Shared gaps (all harnesses)

1. **Usage-null records.** A turn whose harness reports no tokens (codex, pi)
   publishes a 44200 with no `turn`/`cumulative`/`turnSeq`, carrying only
   `tools`/`skills`/`context` when any exist; a turn with none publishes
   nothing. Such a record cannot show token usage, only activity.
2. **Closed relays reject 44200 from direct-member agents** (403):
   #4223, #4937, #7077, #8246; fix in PR #5581. Affects every harness.
3. **Unmetered turns.** Failed, timed-out, and cancelled turns may not
   publish (see #6169).
4. **Consumers.** Only desktop parses these metrics (mobile has no 44200 decoder), and it does
   not show `tools`/`skills`. Desktop skips usage-null rows in token
   accounting.
   #5652 (usage & budgets view) is the natural consumer.

## Per harness

### buzz-agent
- Wire: `tool_call` `title = call.name`, `kind:"other"`, `rawInput = args`
  (`crates/buzz-agent/src/agent.rs:1126`). MCP tools named `server__tool`
  (`crates/buzz-agent/src/mcp.rs`). `load_skill` is a builtin
  (`crates/buzz-agent/src/builtin.rs`).
- Usage: `_goose/unstable/session/update` `usage_update`, cumulative.
- Gaps: none known.

### goose (`aaif-goose/goose`)
- Wire: title is display text (`"developer: shell · cargo test"`, or an LLM
  label), `kind` always `other`; real name in `_meta.goose.toolCall.toolName`
  (`crates/goose/src/acp/server/tool_calls/conversion.rs`). Skills via the
  `skills` extension → `skills__load_skill {name}`.
- Usage: goose extension `usage_update` (cumulative i/o/cost) when the client
  advertises support; also standard `usage_update` and session-cumulative
  prompt-result `usage`.
- Gaps: live run (v1.45.0, ollama provider) produced no tool calls, so the
  tool-call shape is source-verified only.

### claude-agent-acp (`agentclientprotocol/claude-agent-acp`)
- Wire: `_meta.claudeCode.toolName` on every `tool_call` / update (`Bash`,
  `Read`, `mcp__docs__search`, `Skill`). Skill name arrives on a later
  `tool_call_update` (`rawInput.skill`, `_meta.claudeCode.skill`); counted
  once via `toolCallId` dedupe.
- Usage: prompt-result `usage` (input/output/cache read/cache write/total) +
  `usage_update.cost`.
- Gaps: none known. Source-verified from the adapter's wire snapshots; not run
  live here.

### codex-acp (`zed-industries/codex-acp`)
- Wire: exec title is display text (`"Read foo.rs"`, `"List /cwd"`, raw
  command), `kind` execute/read/search, `rawInput` = exec event. MCP:
  `title "Tool: server/tool"`, `rawInput {server, tool, arguments}` → counted
  as `server/tool`. No skill tool.
- Usage: `usage_update {used, size}` only (context occupancy). Prompt result
  has no `usage`.
- Gaps: no per-turn tokens or cost (usage-null record carries tools and
  context); builtin tools only as `kind` buckets;
  skills uncountable. Needs upstream: prompt-result `usage` and a stable tool
  name in `_meta`.

### hermes (`NousResearch/hermes-agent`, `acp_adapter/`)
- Wire: `title = "{tool}: {preview}"` or bare `{tool}`; `kind` from a map;
  no `_meta`; `rawInput` omitted for polished tools (`terminal`, `read_file`,
  `skill_view`, …).
- Usage: prompt-result `usage` (prompt/completion/total/reasoning/cache-read)
  and `usage_update {size, used}`. The `usage` values are SESSION-CUMULATIVE
  (`agent.session_*`) and `inputTokens` already includes cache reads; buzz-acp
  derives per-turn deltas (see `StandardUsageTracker::take_hermes`). No cost.
- Gaps: a cache-read of 0 is indistinguishable from unreported, so it is
  omitted. Skills not countable (no `rawInput.name` on `skill_view`). MCP naming
  unverified (believed `mcp_<server>_<tool>`).

### pi (`salman1993/buzz-pi-acp` @ `86b201e`)
- Wire: non-bash tools `title = toolName` (`read`, `edit`, `write`); bash
  title is the command text with no `rawInput` → counted as `execute`.
- Usage: none at the pinned commit (upstream `svkozak/pi-acp` emits
  `usage_update`; fork `main` not checked).
- Gaps: no tokens (usage-null record carries tools only); skills are plain `read`s of
  `SKILL.md`.

## Next steps (ranked)

1. Land PR #5581 so closed relays accept 44200 from direct members.
2. Upstream asks: codex-acp and hermes stable tool name in `_meta` and
   prompt-result `usage` for codex; `rawInput` on hermes `skill_view`.
3. Live-verify goose and claude-agent-acp against a relay and decrypt the
   owner-side 44200 events.
