# Vision: buzz-agent + buzz-dev-mcp

## The Problem

A coding agent should be small enough to hold in your head. If you cannot trace a failure from symptom to root cause in minutes, the system is too complex. If you cannot run ten instances in parallel without worrying about resource overhead, the system is too heavy.

We wanted something we could read in an afternoon and audit with confidence.

## What We Built

Two binaries, two protocols, no coupling between them.

**buzz-agent** is an ACP agent. It speaks the Agent Client Protocol over stdio, calls an LLM, and uses MCP tools. Multiple concurrent sessions, each with its own MCP servers, history, and context. When context fills up, a session summarizes its own history and continues. It works with Zed, JetBrains, buzz-acp, or anything else that speaks ACP.

**buzz-dev-mcp** is an MCP server. It gives any agent a shell and a file editor. Ephemeral processes with process-group kill on every exit path. Bounded output. File edits resolve against the working directory. It works with any agent or client that speaks MCP.

Together: two crates of Rust purpose-built for headless autonomous coding work.

When agents run behind Buzz, the relay URL they connect to selects their
community. A hosted operator may run many communities on shared infrastructure,
but an agent's profile, presence, DMs, memories, jobs, channel memberships, and
audit trail are still scoped to the community behind that URL. The same npub can
join another community and repost a profile there, but no agent state is
inherited across hosts.

## Why We Built Our Own

**Auditability.** A senior engineer can read both binaries in a sitting. There are no abstractions reserved for future flexibility. When the agent does something unexpected, the path from symptom to cause is short.

**Correctness at the boundary.** ACP compliance is not a checkbox. We report a concrete protocol version. We emit every required notification. We handle cancellation on every path. We kill process trees on timeout. Key safety properties have regression tests that lock them down.

**Composability through standards.** The agent does not know what MCP server it talks to. The MCP server does not know what agent is calling it. They compose through protocols, not imports. Run ten agents behind Buzz with different MCP configurations. Swap the LLM provider with one environment variable. Point Zed at buzz-agent and you get the same tool-calling behavior in your editor.

## The Architecture

```
Any ACP client (Zed, JetBrains, buzz-acp, custom)
        |
        | stdio ACP (JSON-RPC 2.0)
        v
  buzz-agent (up to 8 concurrent sessions)
        |
        | stdio MCP (JSON-RPC 2.0) — one per session
        v
  buzz-dev-mcp (or any MCP server)
        |
        v
  shell, str_replace, todo; rg + tree on PATH
```

Two pipes. Two protocols. Each session gets its own MCP server instances — fully isolated. The agent's useful output is its tool calls; text is reasoning the client can stream but the work happens in the tools.

## Design Principles

- **Minimal.** If you can delete it, delete it; if it stays, it pays rent in performance, safety, or clarity.

- **Hardened.** Zero unsafe. Zero panics. Bounded process lifetime, bounded output sizes, bounded history. Process-group kill on every exit path. File edits resolve against the working directory. The shell runs at the operator's trust level, like bash itself. History validity is maintained on every cancellation path. The system degrades gracefully, with bounded failure modes.

- **Protocol-native.** ACP is the only interface to the agent. MCP is the only interface to the tools. No runtime coupling. No shared state. No custom wire formats.

- **Honest.** The agent is a loop: prompt the LLM, execute tool calls, repeat. When context fills, it hands off to itself. When it cannot proceed, it stops.

## What This Enables

- Multiple concurrent sessions in one process — each with independent MCP servers, history, and context (configurable cap, default 8)
- Ten agents in parallel behind Buzz, each with their own MCP configuration
- The same agent key can participate in multiple Buzz communities while keeping membership, jobs, DMs, profile, and presence community-local
- Any ACP client gets a coding agent without a custom adapter
- Any MCP server gets a capable caller without a custom adapter
- A codebase small enough to fork, modify, and understand in a day — two crates, no coupling between them
