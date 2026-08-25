# `pi-acp` pilot adapter

`pi-acp` is an experimental ACP adapter for Buzz managed agents. It embeds
`@earendil-works/pi-coding-agent` through `AgentSession` and keeps Buzz authoritative for relay
routing, identity, deadlines, deduplication, and NIP-AM publication.

Status: live production canary validated on Caliper; not the default runtime and not approved for fleet rollout. See [`../../docs/pi-acp-production-canary-2026-08-25.md`](../../docs/pi-acp-production-canary-2026-08-25.md).

## Implemented

- ACP `initialize`, `session/new`, `session/prompt`, `_session/steering`, and `session/cancel`;
- strict LF-only JSONL framing, preserving U+2028/U+2029 inside JSON strings;
- isolated in-memory Pi session per adapter process;
- extension, skill, template, theme, and context-file discovery disabled;
- explicit built-in tool allowlist (`read` by default);
- per-inbound-event turn, tool, and processed-token budgets with checkpoint steering and abort;
- authoritative Buzz routing metadata supplied by `buzz-acp` under `_meta.buzz`;
- typed `buzz_reply` with non-empty validation, fixed routing, connected stdin, atomic reservation,
  durable receipt replay, and fail-closed crash/network ambiguity handling;
- typed bounded `kanban_tasks` that cannot download the full board;
- cumulative usage mapping, bounded tool output, and process-group cleanup.

`buzz_reply` intentionally reserves before network access. If the process crashes or the network
result is ambiguous, it refuses to retry automatically. This preserves at-most-once publication at
the cost of requiring operator reconciliation for an uncertain delivery.

## Test

```bash
pnpm install --frozen-lockfile
pnpm --filter @buzz/pi-acp test
pnpm --filter @buzz/pi-acp check
cargo test -p buzz-acp prompt_metadata_is_nested_under_buzz_meta --lib
```

## Local canary installation

From a reviewed exact Buzz commit:

```bash
npm install --global ./tools/pi-acp
pi-acp --version
```

The adapter uses Pi's existing provider authentication. Run Pi interactively once if the host has
not yet configured a model/provider. For a read-only question canary, leave `PI_ACP_TOOLS=read`.
Raise budgets only through explicit environment values:

```text
PI_ACP_MAX_TURNS
PI_ACP_MAX_TOOLS
PI_ACP_MAX_PROCESSED_TOKENS
```

The process accepts ACP JSON-RPC/NDJSON on stdin and writes only ACP frames to stdout. Diagnostics
go to stderr. `PI_ACP_PI_COMMAND` and `PI_ACP_PI_ARGS_JSON` retain the non-shipping RPC subprocess
path for protocol fixtures; normal execution uses the embedded SDK bridge.

## Rollback

Stop the canary, restore its prior `agent_command` (normally `codex-acp`), restart it, and run one
bounded question smoke test. Never run `pi-acp` and another ACP adapter simultaneously for the same
identity and relay.
