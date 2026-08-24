# `pi-acp` RPC spike

This is a **non-shipping protocol spike** for the architecture in
[`docs/pi-harness-integration.md`](../../docs/pi-harness-integration.md). It proves that the ACP
lifecycle used by `buzz-acp` can be translated to `pi --mode rpc` without changing Buzz relay or
identity ownership.

Implemented:

- ACP `initialize`, `session/new`, `session/prompt`, `_session/steering`, and `session/cancel`;
- strict LF-only JSONL framing (U+2028/U+2029 are preserved inside JSON strings);
- Pi text/thinking, tool lifecycle, settled state, cancellation, and cumulative usage mapping;
- one isolated task session per adapter process;
- resource discovery disabled and read-only Pi tools by default;
- bounded tool output and process-group cleanup.

Deliberately absent:

- typed Buzz write/Kanban tools;
- production permission and credential broker;
- hard budget extension hooks;
- multiple simultaneous sessions;
- desktop discovery or release packaging.

Those require the Pi SDK sidecar described in Phase 2. Do not point a production managed agent at
this executable.

## Test

```bash
cd tools/pi-acp-rpc-spike
node --test test/*.test.mjs
```

## Manual protocol run

```bash
PI_ACP_TOOLS=read node src/pi-acp.mjs
```

The process accepts ACP JSON-RPC/NDJSON on stdin and writes only ACP frames to stdout. Diagnostics
and Pi stderr go to stderr. `PI_ACP_PI_COMMAND` and `PI_ACP_PI_ARGS_JSON` exist for tests; the default
Pi command is:

```bash
pi --mode rpc --no-session --no-extensions --no-skills --no-prompt-templates --tools read
```
