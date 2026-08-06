# Surface the ACP adapter install hint when model discovery fails

## Problem

When a user opens the Create-agent dialog and picks a runtime whose ACP adapter
binary is not installed (e.g. `claude-agent-acp` absent because the npm package
was never installed), the model field shows only:

> Using built-in model options. Could not load live models for this provider.

That is the catch-all fallback in
`desktop/src/features/agents/ui/personaModelDiscoveryStatus.ts`. It gives the
user nothing actionable, even though
`desktop/src-tauri/src/managed_agents/discovery.rs` already carries per-runtime
`adapter_install_hint`, `adapter_install_commands`, and
`adapter_install_instructions_url` for exactly this situation.

Reported from macOS, where the `buzz-acp` harness is bundled with the app but
the npm adapter was absent — so the existing `missing_command_message` path
(which guards the harness command) never fired.

## Root cause

`discover_agent_models` resolves the *harness* command strictly:

```rust
let resolved_acp = resolve_command(acp_command)
    .ok_or_else(|| missing_command_message(acp_command, "ACP harness command"))?;
```

but resolves the *adapter* command leniently:

```rust
let resolved_agent = resolve_command(agent_command)
    .map(|p| p.display().to_string())
    .unwrap_or_else(|| agent_command.to_string());
```

A missing adapter therefore falls through to the `buzz-acp models` subprocess,
which fails, producing an opaque `buzz-acp models failed (exit N): …` string
that the frontend cannot distinguish from any other discovery failure.
`get_agent_models` (the saved-agent path) has the same shape.

## Design

### 1. A distinguishable backend error

Add a pure helper in a new `managed_agents/adapter_install.rs`:

```rust
pub fn adapter_missing_error(command: &str) -> Option<String>
```

It returns `Some(sentinel)` only when the runtime declares
`adapter_install_commands`; runtimes without an adapter (goose, buzz-agent) and
unknown commands return `None`, preserving today's behavior exactly.

The sentinel mirrors the existing `DANGLING_HARNESS_ID:` convention, with a
JSON payload so the frontend renders hint, command, and link distinctly without
duplicating the runtime catalog in TypeScript:

```
ADAPTER_MISSING:{"runtimeId":"claude","runtimeLabel":"Claude Code","hint":"…","commands":["npm install -g @agentclientprotocol/claude-agent-acp"],"url":"https://github.com/agentclientprotocol/claude-agent-acp"}
```

The guard lives in `run_agent_models_command` — the single spawn point both
`discover_agent_models` and `get_agent_models` funnel into, and the first place
the adapter is actually required. Putting it there rather than at each caller's
command-resolution step matters for correctness as well as duplication: every
provider-API discovery path (OpenRouter, OpenAI-compatible, Anthropic,
Databricks, relay-mesh) runs *before* the spawn, so a setup with a missing
adapter but a working provider key still discovers models exactly as it does
today. Both callers already pass the resolved path when resolution succeeded,
so an unresolvable command at the spawn point is precisely the missing-adapter
case.

It also keeps `agent_models.rs` and `discovery.rs` — both already over the
1000-line desktop file-size ratchet — untouched.

### 2. A frontend branch

`formatModelDiscoveryErrorStatus` gains a first branch that detects the
sentinel, parses the payload, and returns the runtime's own hint text. The hint
already embeds the install command verbatim ("Buzz talks to the Claude Code CLI
through an ACP adapter. Install it with: npm install -g …"), so it is used
as-is — no second copy of the command to keep in sync. When the hint is empty
the message is synthesized from `commands`. Malformed JSON after the sentinel
falls through to the existing catch-all.

### 3. Link rendering

`PersonaModelDiscoveryStatus` gains an optional
`link?: { href: string; label: string }`. `PersonaModelField.tsx` renders it as
an anchor under the message. The two other status consumers
(`agentConfigControls.tsx`, `AgentInstanceEditDialog.tsx`) reduce status to a
plain string through `resolveModelFieldStatusMessage`; they show the message
without the link and are otherwise unchanged.

## Testing

Rust (`agent_models_tests.rs`):
- `adapter_missing_error` yields a sentinel carrying the claude runtime's hint,
  command, and adapter URL.
- The same for codex, proving the payload is read from the catalog rather than
  hardcoded.
- Returns `None` for goose and buzz-agent (no adapter to install).
- Payload parses as JSON and is prefixed by the sentinel.

TypeScript (`personaModelDiscoveryStatus.test.mjs`):
- Sentinel error → warning status whose message is the runtime hint and whose
  link points at the adapter instructions URL.
- Sentinel with an empty hint → message synthesized from the install command.
- Sentinel followed by malformed JSON → catch-all fallback, no crash.
- Existing non-sentinel errors (auth required, API key required, relay-mesh)
  keep their current output.

## Non-goals

- No change to `missing_command_message` or the harness-command path.
- No auto-install trigger from the model field; Doctor already owns that flow.
- No new copy strings — all text comes from the runtime catalog.
