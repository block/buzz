# MCP-Driven Lifecycle Hooks

## Overview

Buzz-agent supports lifecycle hooks — MCP tools that the agent calls at
defined points in its execution loop. Any MCP server can participate by
exposing tools with the `_` prefix. Hooks are invisible to the LLM, advisory
to the agent, and operator-configured.

This convention requires zero MCP protocol changes. Hooks are regular tools
discovered via `tools/list` and invoked via `tools/call`.

## Convention

- Tools whose bare name starts with `_` are lifecycle hooks
- Hooks are filtered from the tool list sent to the LLM
- Hooks are rejected if the LLM attempts to call them directly
- Hooks are called by the agent at defined lifecycle points
- Hook responses are injected as tool-result messages (lower trust than system)
- Hook output is JSON-encoded for prompt-injection safety

## Defined Hooks

### `_Stop`

**When:** The LLM signals `end_turn`, before the agent honors it.

**Input:** `{}`

**Output:** Non-empty text = objection (agent continues). Empty = no objection
(agent stops).

**Use case:** Todo enforcement — object when open tasks remain.

### `_PostCompact`

**When:** After context compaction/handoff, before the next LLM prompt.

**Input:** `{}`

**Output:** Non-empty text = injected into fresh context. Empty = nothing
injected.

**Use case:** Re-inject todo list state after history is summarized and reset.

## Agent Sovereignty

Hooks are advisory, not authoritative. The agent enforces:

| Constraint | Behavior |
|---|---|
| Timeout (2.5s default) | Treated as no objection. Server killed only on second consecutive timeout (tolerates one-off slowness) |
| Rejection budget (3/prompt) | After exhaustion, agent stops regardless; the budget resets on the next prompt |

These constraints ensure a buggy or malicious hook cannot trap the agent.

## Configuration

| Env Var | Default | Description |
|---|---|---|
| `MCP_HOOK_SERVERS` | (unset = no hooks) | Allowlist: `*` for all servers, or comma-separated names |
| `BUZZ_AGENT_HOOK_TIMEOUT_MS` | 2500 | Per-hook call timeout in milliseconds |
| `BUZZ_AGENT_STOP_MAX_REJECTIONS` | 3 | Per-prompt `_Stop` budget (0 = disable) |

Hooks are **off by default**. The operator must explicitly opt in via
`MCP_HOOK_SERVERS`.

### Not a hook: the reply guard

`buzz-agent` has one in-process objection at the `_Stop` gate that is **not** an
MCP hook and exposes no hook tool: the reply guard
(`BUZZ_AGENT_REQUIRE_REPLY=1`), which reminds the model to publish when a turn is
about to end with nothing posted to Buzz. There is no `_ReplyGuard` tool to
implement and no server to allowlist — the env var and the recognition contract
are documented in
[crates/buzz-agent/README.md](../crates/buzz-agent/README.md#reply-guard).

It is mentioned here only because it shares this lifecycle point and this
budget: its reminders count against `BUZZ_AGENT_STOP_MAX_REJECTIONS` like any
hook objection, and a round carrying both a hook objection and a reminder costs
one rejection and delivers both texts. Setting the budget to 0 disables both.
That the gate can carry in-process objections alongside hook output is
deliberate; hooks see no difference.

## Implementing a Hook

Any MCP server can expose hooks. Example: a test-runner server that blocks
`end_turn` while tests are failing:

```json
{
  "name": "_Stop",
  "description": "Returns failing test summary if suite is red.",
  "inputSchema": { "type": "object" }
}
```

The server returns non-empty text to object, empty string to allow stopping.

## Compatibility

Hook naming is aligned with the [Open Plugin Spec](https://open-plugins.com/agent-builders/components/hooks)
event conventions. `_Stop` corresponds to the `Stop` event; `_PostCompact`
corresponds to `PostCompact`.

`MCP_HOOK_SERVERS` is a standard env var name intended for cross-agent adoption.

## Future Work

Additional hook points may be added to support the fuller Open Plugin Spec
event set:

| Open Plugin Event | Potential Hook | Status |
|---|---|---|
| `Stop` | `_Stop` | ✅ Implemented |
| `PostCompact` | `_PostCompact` | ✅ Implemented |
| `PreToolUse` | `_PreToolUse` | Deferred (overlaps with MCP Interceptors SEP-2624) |
| `PostToolUse` | `_PostToolUse` | Deferred (overlaps with MCP Interceptors SEP-2624) |
| `SessionStart` | `_SessionStart` | Candidate for future revision |
| `SessionEnd` | `_SessionEnd` | Candidate for future revision |
| `UserPromptSubmit` | `_UserPromptSubmit` | Candidate for future revision |
| `SubagentStart` | `_SubagentStart` | Candidate for future revision |

Pre/post tool-call hooks are deferred pending coordination with the MCP
Interceptors working group (SEP-2624), which addresses similar concerns at
the protocol layer. The remaining events will be added as use cases emerge.
