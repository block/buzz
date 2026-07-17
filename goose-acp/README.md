# goose-acp — bundled slim goose agent

A minimal binary exposing [goose](https://github.com/aaif-goose/goose) as an
ACP agent server on stdio, so the Buzz distribution can bundle a capable
goose agent without users installing goose themselves.

## What's in / what's out

Built from the `goose` library crate with `default-features = false`,
`features = ["rustls-tls"]`:

| Included | Excluded |
|---|---|
| Full agent loop, sessions, compaction | code-mode (pctx → V8 JS/TS runtime) |
| All API providers (Anthropic, OpenAI, …) | local-inference (llama.cpp/candle) |
| MCP extensions, developer tools | system keyring, AWS providers |
| ACP stdio server | telemetry/otel, goose CLI/TUI |

The wrapper explicitly enables Goose's in-core `developer` extension so a
fresh Buzz install has shell and file tools without relying on an existing
Goose config. Other extensions can still arrive through ACP or Goose config.

The desktop payload removes the old `buzz-agent` sidecar when it adds this one,
so the distribution impact is the difference between those two compressed
binaries rather than the full size of Goose.

Measured on macOS arm64 with Goose v1.43.0:

| Payload | Raw | gzip -9 | bzip2 -9 |
|---|---:|---:|---:|
| `goose-acp` | 34,128,848 B | 10,543,252 B | 9,552,021 B |
| removed `buzz-agent` | 10,135,440 B | 4,074,456 B | 3,734,236 B |
| net desktop increase | 23,993,408 B | 6,468,796 B | 5,817,785 B |

## Why a separate workspace

This crate is excluded from the Buzz workspace on purpose:

- goose's ~700-crate dependency graph stays out of the workspace Cargo.lock
- Goose's ACP/MCP dependencies are resolved in this crate's own lockfile, so
  their versions do not perturb Buzz's workspace lockfile

## Building

```bash
cargo build --release --manifest-path goose-acp/Cargo.toml
# → goose-acp/target/release/goose-acp
```

`scripts/bundle-sidecars.sh` stages it as a Tauri sidecar
(`binaries/goose-acp-<triple>`). Release CI and the full local app recipes build
it explicitly because it is the desktop's default harness.

## Using it

The binary **is** the ACP server — no `acp` subcommand (buzz-acp knows this;
see `default_agent_args` in `crates/buzz-acp/src/config.rs`):

```bash
BUZZ_ACP_AGENT_COMMAND=/path/to/goose-acp buzz-acp ...
```

Provider/model configuration is standard goose: `GOOSE_PROVIDER`,
`GOOSE_MODEL`, provider API-key env vars, or `~/.config/goose/config.yaml`.
When Desktop launches the sidecar, it also translates Buzz's existing
`OPENAI_COMPAT_API_KEY` / `OPENAI_COMPAT_BASE_URL` settings to Goose's native
names and maps the `relay-mesh` preset onto Goose's OpenAI-compatible provider.

## Why a sidecar instead of the Rust `Agent` API?

Goose's Rust API works well when an application owns a small chat loop (for an
example, see the sibling `mesh-console` project). Buzz already has the inverse
architecture: `buzz-acp` is an ACP client that owns relay subscriptions,
session lifecycle, cancellation, steering, config updates, and UI events.

Calling `Agent::reply` directly would require rebuilding Goose's provider,
session, extension, permission, and event-to-ACP layers inside `buzz-acp`, while
also pulling Goose's dependency graph into that binary. Calling Goose's public
ACP server keeps those semantics upstream, preserves process isolation, and is
only the wrapper in `src/main.rs`. The Rust API remains a reasonable future
option if Buzz replaces ACP as its harness boundary, but it is not a size win
on its own: the same Goose agent code still has to be linked somewhere.

## Runtime proof

`tests/stdio_turn.rs` launches the built sidecar, speaks the same ACP stdio
protocol as `buzz-acp`, and completes `initialize` → `session/new` →
`session/prompt` against a deterministic local OpenAI-compatible stream. It
asserts Goose 1.43.0 identity, successful developer-tool initialization, the
authenticated provider request, streamed assistant text, and an `end_turn`
response. CI runs this test from the isolated lockfile so a sidecar that merely
compiles but cannot serve a full turn fails.
