# sprout-agent

> Minimal, unbreakable ACP-compliant LLM agent. Stdio in, tool calls out. Non-streaming. No persistence. No cleverness.

[ACP](https://agentclientprotocol.com) is the Agent Client Protocol — JSON-RPC 2.0 over stdio between a client (Zed, JetBrains, sprout-acp, …) and an agent. [MCP](https://modelcontextprotocol.io) is how the agent talks to its tools.

`sprout-agent` is the agent.

## What It Is

```
        +--------+   stdio (JSON-RPC 2.0)   +---------------+
        | client | <----------------------> |  sprout-agent |
        +--------+        ACP frames        +---------------+
                                              │            │
                                              │            │ rmcp (stdio)
                                              │            ▼
                                              │       MCP servers
                                              │       (your tools)
                                              ▼
                                            HTTPS
                                              │
                                              ▼
                                  Anthropic Messages API
                                   or any OpenAI-compat
                                  (vLLM, llama.cpp, OpenRouter,
                                   Block Gateway, Ollama, …)
```

A client sends `session/prompt`. The agent loops: call the LLM → get tool calls → run them via MCP → feed results back → repeat. The loop terminates when the LLM stops asking for tools, the round cap is hit, or the client cancels.

The agent's **output is its tool calls**. Generated text is forwarded to the client as `agent_message_chunk` updates, but the real work happens in the tools. The LLM call is non-streaming — one HTTP POST, one response.

## Quick Start

```bash
# Build
cargo build --release -p sprout-agent

# Run against Anthropic
SPROUT_AGENT_PROVIDER=anthropic \
ANTHROPIC_API_KEY=sk-ant-... \
ANTHROPIC_MODEL=claude-sonnet-4-5 \
  ./target/release/sprout-agent

# Or any OpenAI-compatible endpoint
SPROUT_AGENT_PROVIDER=openai \
OPENAI_COMPAT_API_KEY=sk-... \
OPENAI_COMPAT_MODEL=gpt-5 \
OPENAI_COMPAT_BASE_URL=https://api.openai.com/v1 \
  ./target/release/sprout-agent

# Or Databricks model serving via OAuth 2.0 PKCE
SPROUT_AGENT_PROVIDER=databricks \
DATABRICKS_HOST=https://dbc-...cloud.databricks.com \
DATABRICKS_MODEL=goose-claude-4-6-sonnet \
  ./target/release/sprout-agent
```

That's the whole setup. The agent reads JSON-RPC frames from stdin, writes them to stdout, and logs to stderr.

## ACP Transcript

A complete round-trip. Lines starting with `→` are client→agent (stdin); `←` are agent→client (stdout). Each line is one newline-terminated JSON value. Comments are not part of the wire.

```jsonc
// 1. Handshake.
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}
← {"jsonrpc":"2.0","id":1,"result":{
    "protocolVersion":1,
    "agentCapabilities":{
      "loadSession":false,
      "promptCapabilities":{"image":false,"audio":false,"embeddedContext":false},
      "mcpCapabilities":{"http":false,"sse":false}
    },
    "agentInfo":{"name":"sprout-agent","version":"0.1.0"}
  }}

// 2. Open a session. The client passes the MCP servers to spawn.
→ {"jsonrpc":"2.0","id":2,"method":"session/new","params":{
    "cwd":"/tmp",
    "mcpServers":[{"name":"echo","command":"/usr/local/bin/echo-mcp","args":[],"env":[]}]
  }}
← {"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_a1b2c3d4e5f6a7b8"}}

// 3. Prompt. The agent loops until the LLM stops calling tools.
→ {"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{
    "sessionId":"ses_a1b2c3d4e5f6a7b8",
    "prompt":[{"type":"text","text":"echo hello"}]
  }}

// 4. Agent emits tool_call (status: pending) — visible to the UI.
← {"jsonrpc":"2.0","method":"session/update","params":{
    "sessionId":"ses_a1b2c3d4e5f6a7b8",
    "update":{
      "sessionUpdate":"tool_call",
      "toolCallId":"toolu_01XYZ",
      "title":"echo__say",
      "kind":"other",
      "status":"pending",
      "rawInput":{"text":"hello"}
    }
  }}

// 5. Agent moves the call to in_progress, runs the MCP tool, then completed.
← {"jsonrpc":"2.0","method":"session/update","params":{
    "sessionId":"ses_a1b2c3d4e5f6a7b8",
    "update":{"sessionUpdate":"tool_call_update","toolCallId":"toolu_01XYZ","status":"in_progress"}
  }}
← {"jsonrpc":"2.0","method":"session/update","params":{
    "sessionId":"ses_a1b2c3d4e5f6a7b8",
    "update":{
      "sessionUpdate":"tool_call_update",
      "toolCallId":"toolu_01XYZ",
      "status":"completed",
      "content":[{"type":"content","content":{"type":"text","text":"hello"}}]
    }
  }}

// 8. The model sees the result, decides it's done, and the prompt resolves.
← {"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}
```

That's ACP. Three request methods (`initialize`, `session/new`, `session/prompt`), one inbound notification (`session/cancel`), and three outbound update variants (`agent_message_chunk`, `tool_call`, `tool_call_update`). The full server is hand-rolled in `main.rs`.

## Configuration

Everything is environment variables. No flags, no config files. (We are a subprocess; subprocess config is environment.)

| Variable | Default | Notes |
|---|---|---|
| `SPROUT_AGENT_PROVIDER` | — | `anthropic`, `openai`, or `databricks`. If unset, or if `anthropic`/`openai` is selected but its API key is missing, Databricks is auto-selected when `DATABRICKS_HOST` + `DATABRICKS_MODEL` are set. |
| `ANTHROPIC_API_KEY` | — | Required when provider=anthropic unless Databricks fallback is configured. |
| `ANTHROPIC_MODEL` | — | Required when provider=anthropic. |
| `ANTHROPIC_BASE_URL` | `https://api.anthropic.com` | |
| `ANTHROPIC_API_VERSION` | `2023-06-01` | |
| `OPENAI_COMPAT_API_KEY` | — | Required when provider=openai unless Databricks fallback is configured. |
| `OPENAI_COMPAT_MODEL` | — | Required when provider=openai. |
| `OPENAI_COMPAT_BASE_URL` | `https://api.openai.com/v1` | Point at vLLM, llama.cpp, OpenRouter, Ollama, etc. |
| `OPENAI_COMPAT_API` | `auto` | `auto` \| `chat` \| `responses`. `auto` picks Responses for `*.openai.com`, Chat Completions everywhere else. |
| `DATABRICKS_HOST` | — | Required when provider=databricks or when using Databricks fallback. |
| `DATABRICKS_MODEL` | — | Required when provider=databricks or when using Databricks fallback. |
| `DATABRICKS_TOKEN` | — | Optional static bearer escape hatch. If unset, Databricks uses browser OAuth + refresh cache. |
| `SPROUT_AGENT_SYSTEM_PROMPT` | built-in | Inline system prompt. |
| `SPROUT_AGENT_SYSTEM_PROMPT_FILE` | — | File path. Mutually exclusive with the above. |
| `SPROUT_AGENT_MAX_ROUNDS` | `0` | Tool-loop iteration cap. 0 = unlimited. |
| `SPROUT_AGENT_MAX_OUTPUT_TOKENS` | `32768` | Per LLM call. Headroom for large tool-call inputs (e.g. file writes via heredoc); Sonnet 4 / Opus 4 cap at 64K. |
| `SPROUT_AGENT_MAX_CONTEXT_TOKENS` | `200000` | Provider context window used by the handoff gate. |
| `SPROUT_AGENT_MAX_HANDOFFS` | `10` | Max context handoffs per session before falling back to truncation. |
| `SPROUT_AGENT_LLM_TIMEOUT_SECS` | `120` | |
| `SPROUT_AGENT_TOOL_TIMEOUT_SECS` | `660` | Per-tool call timeout in seconds |
| `SPROUT_AGENT_MAX_PARALLEL_TOOLS` | `8` | Max concurrent tool calls per turn (1 = sequential) |
| `SPROUT_AGENT_MAX_SESSIONS` | unlimited | Max concurrent ACP sessions. Sessions are cheap; default has no cap. |
| `SPROUT_AGENT_MAX_LINE_BYTES` | `4194304` | 4 MiB. Hard cap on inbound JSON-RPC frames. |
| `SPROUT_AGENT_MAX_HISTORY_BYTES` | `1048576` | 1 MiB. Old turns are evicted past this. |


## Providers

`sprout-agent` speaks two HTTP dialects. Pick with `SPROUT_AGENT_PROVIDER`.

| Provider | `SPROUT_AGENT_PROVIDER` | Endpoint (auto) | Tested with |
|---|---|---|---|
| Anthropic | `anthropic` | `POST {base}/v1/messages` | claude-sonnet-4-5, claude-opus-4 |
| OpenAI | `openai` | `POST {base}/responses` | gpt-5, gpt-5-mini, o4-mini, gpt-4o |
| vLLM | `openai` | `POST {base}/chat/completions` | any tool-calling model |
| llama.cpp | `openai` | `POST {base}/chat/completions` | any tool-calling GGUF |
| Ollama | `openai` | `POST {base}/chat/completions` | llama3.1, qwen2.5-coder |
| OpenRouter | `openai` | `POST {base}/chat/completions` | anything they route |
| Block Gateway | `openai` | `POST {base}/chat/completions` | gpt-5, claude |
| Databricks | `databricks` | `POST {host}/serving-endpoints/{model}/invocations` | goose-claude-4-6-sonnet |

If `SPROUT_AGENT_PROVIDER=anthropic` is selected without `ANTHROPIC_API_KEY`, or `SPROUT_AGENT_PROVIDER=openai` is selected without `OPENAI_COMPAT_API_KEY`, the agent automatically falls back to Databricks OAuth when `DATABRICKS_HOST` and `DATABRICKS_MODEL` are set. The same Databricks fallback applies when `SPROUT_AGENT_PROVIDER` is unset. Explicit Anthropic/OpenAI API keys always win.

`provider=openai` speaks two HTTP dialects: the [Responses API](https://platform.openai.com/docs/api-reference/responses) (`/v1/responses`, required for GPT-5 / o-series tool-calling on OpenAI's own service) and the [Chat Completions API](https://platform.openai.com/docs/api-reference/chat) (`/chat/completions`, the broadly-supported OpenAI-compatible wire format).

By default (`OPENAI_COMPAT_API=auto`) the agent picks **Responses** when `OPENAI_COMPAT_BASE_URL` points at an `*.openai.com` host and **Chat Completions** everywhere else. Pin the choice explicitly with `OPENAI_COMPAT_API=chat` or `OPENAI_COMPAT_API=responses` for providers that diverge from the default (e.g. a Responses-compatible self-hosted gateway).

`Provider` is a Rust `enum` with one `match` in `Llm::complete`. There is no trait, no `Box<dyn>`, no async-trait. Adding a third provider is a `match` arm and one `body`/`parse` pair in `llm.rs`.

## MCP Servers

The client passes MCP server specs in `session/new`. The agent spawns each one as a stdio subprocess, calls `tools/list`, and merges everything into a single tool catalog the LLM sees. Tool names are namespaced as `server__tool` (double underscore separator). Bare tool names containing `__` are rejected at registration.

Example: a single echo MCP server.

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "cwd": "/work",
    "mcpServers": [
      {
        "name": "echo",
        "command": "/usr/local/bin/echo-mcp",
        "args": ["--mode", "stdio"],
        "env": [
          { "name": "ECHO_VERBOSE", "value": "1" }
        ]
      }
    ]
  }
}
```

Multiple servers: just add more entries. Tool calls fan out to the right server by namespace prefix.

**Transport: stdio only.** No HTTP, no SSE. We advertise this in `agentCapabilities` (`mcpCapabilities.http: false`, `mcpCapabilities.sse: false`); spec-compliant clients won't ask for what we don't have.

## Security Model

The trust boundary is **the operator who launched the agent**. The harness, MCP server binaries, and API keys are all trusted. Untrusted input — model output, tool results, prompts — is bounded.

| Boundary | Mechanism |
|---|---|
| Stdout discipline | Single-consumer `mpsc` channel feeding stdout. No two tasks can interleave bytes. All logs go to stderr. |
| MCP child env | Whitelist (`PATH`, `HOME`, `TERM`, `LANG`, `LC_ALL`, `TMPDIR`) plus what the client explicitly passes. Your `ANTHROPIC_API_KEY` does not leak into MCP children. |
| MCP child lifetime | Process group via `setpgid(0,0)` in `pre_exec`. On transport break or shutdown: `killpg(SIGKILL)`. Grandchildren die too. |
| Server poisoning | After a timeout or transport break, the offending server is marked dead. Future calls trigger a lazy restart with exponential backoff. Other servers keep working. |
| Frame size | `SPROUT_AGENT_MAX_LINE_BYTES` (default 4 MiB). Oversize → connection killed. |
| LLM response size | 16 MiB hard cap. Both `Content-Length` precheck and streaming-buffer cap. |
| Cancellation | `tokio::select! { biased; _ = cancel.changed() => ... }` at every loop boundary. Cancel always wins the race. |
| Session isolation | Unlimited concurrent sessions by default (configurable via `SPROUT_AGENT_MAX_SESSIONS`). One prompt per session at a time. Each session gets its own MCP servers. |
| `tool_use ↔ tool_result` pairing | Encoded in the type system. Every `ToolCall` and `ToolResult` carries a `provider_id: String` (not `Option`). |

### Bounded Everything

| Limit | Default | Where |
|---|---|---|
| Inbound JSON-RPC frame | 4 MiB | `SPROUT_AGENT_MAX_LINE_BYTES` |
| Single prompt | 1 MiB | `MAX_PROMPT_BYTES` |
| History window | 1 MiB | `SPROUT_AGENT_MAX_HISTORY_BYTES` |
| LLM response body | 16 MiB | `MAX_LLM_RESPONSE_BYTES` |
| LLM error body | 4 KiB | `MAX_LLM_ERROR_BODY_BYTES` |
| Tool result body | 256 KiB | `MAX_TOOL_RESULT_BYTES` |
| MCP servers / session | 16 | `MAX_MCP_SERVERS` |
| Tools / session | 128 | `MAX_TOOLS_PER_SESSION` |
| Tool description bytes | 1 KiB | `MAX_DESCRIPTION_BYTES` |
| Tool schema bytes | 4 KiB | `MAX_SCHEMA_BYTES` (oversize → replaced with `{}`) |
| Tool calls per turn | 64 | `MAX_TOOL_CALLS_PER_TURN` |
| Loop rounds | 0 (unlimited) | `SPROUT_AGENT_MAX_ROUNDS` |
| LLM call timeout | 120 s | `SPROUT_AGENT_LLM_TIMEOUT_SECS` |
| Tool call timeout | 660 s | `SPROUT_AGENT_TOOL_TIMEOUT_SECS` |

## What This Is NOT

A short list, because the answer is mostly "no":

- **Not a framework.** No plugins, no recipes, no slash commands, no modes. MCP servers can participate in agent lifecycle via [hook tools](../../docs/MCP_DRIVEN_HOOKS.md) (`_Stop`, `_PostCompact`), but these are advisory, fail-open, and budget-bounded — not a plugin system.
- **Not streaming.** One non-streaming HTTP POST per round. The LLM's generated text is forwarded to the client as `agent_message_chunk`, but there is no token-level streaming.
- **Not persistent.** Everything is in-memory, per-process. No SQLite. When context fills, the agent summarizes its own history and continues (context handoff). No external persistence.
- **Not an SDK.** This is a binary. The protocol seam is stdin/stdout. Use it from any language.
- **Not a UI.** No TUI, no web, no notifications. The client renders.
- **Not authenticated.** API keys come from env. Use systemd, Docker secrets, or a wrapper.
- **Not networked MCP.** Stdio transport only. No HTTP/SSE MCP transport.
- **Not load-able.** No `session/load`. We advertise `loadSession: false`.
- **Not a router.** No agent-to-agent, no fan-out, no orchestration. One model. One loop.

**Concurrency model:**

```
                  ┌──── reader task ──────────┐
                  │  (stdin → JSON-RPC → ...) │
                  │                           │
   stdin ─────────┤   dispatch                │
                  │     │                     │
                  │     ├── initialize        │  (sync reply)
                  │     ├── session/new       │  (sync reply)
                  │     ├── session/prompt ───┼─── spawn ──> prompt task
                  │     │                     │              │
                  │     ├── session/cancel ───┼─> watch::send│ (biased select wins)
                  │     │                     │              │
                  └───────────────────────────┘              │
                                                             │
                  ┌── writer task ────────────────┐          │
   stdout ────────┤  mpsc<WireMsg> consumer       │<─────────┘
                  │  (the only stdout writer)     │
                  └───────────────────────────────┘
```

One reader, one writer, up to 8 concurrent prompt tasks (one per session).

## Building

```bash
cargo build --release -p sprout-agent
```

## Testing

```bash
cargo test -p sprout-agent
```

Test strategy is **real subprocess, no mocks**:

- **Fake LLM** — `tests/fake_llm.rs` and the helpers in `tests/regressions.rs` spin up a real `tokio::net::TcpListener` on port 0, parse `Content-Length`, and return scripted JSON. No HTTP mocking library.
- **Fake MCP server** — `tests/bin/fake_mcp.rs` is a separate binary controlled by env vars: `FAKE_MCP_HANG_INIT`, `FAKE_MCP_TOOL_DELAY`, `FAKE_MCP_SPAWN_GRANDCHILD`, etc. Each fault path is a real process being abused.
- **Regression tests are the changelog.** Each `#[test]` in `regressions.rs` is named for the bug it locks down: `assistant_text_preserved_across_prompts`, `cancel_leaves_history_valid_for_next_prompt`, `mcp_init_timeout_kills_child`, `oversize_line_kills_connection`. Read them in order to learn the protocol's failure modes.
