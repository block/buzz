# buzz-acp

ACP harness that connects AI agents to Buzz. The harness listens for @mentions on the relay, prompts your agent, and the agent replies using the Buzz CLI.

```
Buzz Relay ──WS──→ buzz-acp ──stdio──→ Your Agent
                                               │
                                          Buzz CLI
                                       (send_message, etc.)
```

Supports any agent that speaks [ACP](https://agentclientprotocol.com/) over stdio: **goose**, **codex** (via [codex-acp](https://github.com/agentclientprotocol/codex-acp)), and **claude code** (via [claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp)).

## Prerequisites

- A running Buzz relay (`just relay` starts Docker services automatically, or use a hosted instance)
- A Nostr keypair for the agent (see [Generating Keys](#generating-keys))

Build:

```bash
cargo build --release -p buzz-acp
export PATH="$PWD/target/release:$PATH"
```

## Generating Keys

Each agent needs a Nostr keypair — this is the agent's identity in Buzz. Use `buzz-admin` to generate one:

```bash
cargo run -p buzz-admin -- generate-key
```

This prints a public and secret key pair as hex. **Save the secret key immediately — it is not stored and cannot be recovered.** Set `BUZZ_PRIVATE_KEY` to the secret key to act as this identity.

Then register the agent's public key as a relay member so it can read and publish:

```bash
BUZZ_RELAY_PRIVATE_KEY=<relay signing key> \
  cargo run -p buzz-admin -- add-member --pubkey <agent public key>
```

`add-member` publishes a kind:13534 membership event, so the relay needs a stable signing key: set `BUZZ_RELAY_PRIVATE_KEY` in the relay's environment (uncomment it in `.env`) and restart the relay before running this.

> **Running multiple agents?** Mint a separate keypair for each. Every agent needs its own identity.

## Channels

The harness discovers channels by querying the relay with the agent's authenticated identity.

By default, the harness discovers only channels the agent is a **member** of (`GET /api/channels?member=true`). When the agent is added to a new channel, the membership notification subscription auto-subscribes to it.

**Private channels** require explicit membership. The relay doesn't yet have a REST/event API for managing channel members — this is a known gap. For now, use `create_channel` via the Buzz CLI to create new channels (the creator is automatically a member).

## Quick Start (goose)

```bash
export BUZZ_PRIVATE_KEY="nsec1..."   # your agent's key (see "Generating Keys")
export BUZZ_RELAY_URL="ws://localhost:3000"
export GOOSE_MODE=auto

buzz-acp
```

That's it. The harness spawns `goose acp`, connects to the relay, discovers channels, and starts listening. When someone @mentions the agent, goose receives the message and can reply using the Buzz CLI that the harness configures automatically.

## Running with Codex

[codex-acp](https://github.com/agentclientprotocol/codex-acp) wraps OpenAI Codex in an ACP interface.

```bash
# Install the adapter (npm package — no Rust build required)
npm install -g @agentclientprotocol/codex-acp

# Run
export OPENAI_API_KEY="sk-..."   # required — use an OpenAI API key, not a ChatGPT subscription

buzz-acp
```

> **API key note:** `codex-acp` always attempts a ChatGPT WebSocket login first, which logs a `426 Upgrade Required` error. This is expected and non-fatal — it falls back to `OPENAI_API_KEY` automatically. Set `OPENAI_API_KEY` to ensure it has a working fallback.

## Running with Claude Code

[claude-agent-acp](https://github.com/agentclientprotocol/claude-agent-acp) wraps the Claude Agent SDK in an ACP interface.

```bash
# Install the current adapter package
npm install -g @agentclientprotocol/claude-agent-acp

# Run
export ANTHROPIC_API_KEY="sk-ant-..."
export BUZZ_ACP_AGENT_COMMAND="claude-agent-acp"

buzz-acp
```

Older installs that still expose `claude-code-acp` are also supported. `buzz-acp`
treats both Claude ACP command names as the same zero-arg runtime.

## Configuration

All configuration is via environment variables (or CLI flags — every env var has a matching flag).

### Core

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `BUZZ_PRIVATE_KEY` | **yes** | — | Agent's Nostr private key (`nsec1...`). Used for relay auth and agent identity. |
| `BUZZ_RELAY_URL` | no | `ws://localhost:3000` | Relay WebSocket URL. |
| `BUZZ_ACP_AGENT_COMMAND` | no | `goose` | Agent binary to spawn. |
| `BUZZ_ACP_AGENT_ARGS` | no | `acp` | Agent arguments (comma-separated). |
| `BUZZ_ACP_MCP_COMMAND` | no | `""` (empty) | Path to an optional MCP server binary to provide to the agent subprocess. |
| `BUZZ_ACP_IDLE_TIMEOUT` | no | `620` | Idle timeout: max seconds of silence before cancelling a turn. Resets on any agent stdout activity. |
| `BUZZ_ACP_MAX_TURN_DURATION` | no | `7200` | Absolute wall-clock cap per turn (safety valve). |
| `BUZZ_API_TOKEN` | no | — | API token (required if relay enforces token auth). |

**Note:** `BUZZ_ACP_AGENT_ARGS` splits on commas. For args with values, use: `-c,key="value"`.

**Legacy env vars:** `BUZZ_ACP_PRIVATE_KEY`, `BUZZ_ACP_API_TOKEN`, and `BUZZ_ACP_TURN_TIMEOUT` (replaced by `BUZZ_ACP_IDLE_TIMEOUT`) are still accepted as fallbacks.

### Parallel Agents & Heartbeat

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--agents` | `BUZZ_ACP_AGENTS` | `1` | Number of agent subprocesses (1–32). |
| `--lazy-pool` | `BUZZ_ACP_LAZY_POOL` | `false` | Connect, subscribe, and queue accepted work before starting ACP/LLM subprocesses. The first accepted event wakes one pool initialization task; failures retry with bounded exponential backoff while work remains. |
| `--heartbeat-interval` | `BUZZ_ACP_HEARTBEAT_INTERVAL` | `0` | Seconds between heartbeat prompts. `0` = disabled. Must be `0` or 10–86400 when enabled; a durably designated agent requires the exact positive cadence pinned in both its Desktop designation and policy. |
| `--heartbeat-prompt` | `BUZZ_ACP_HEARTBEAT_PROMPT` | (built-in) | Custom heartbeat prompt text. Conflicts with `--heartbeat-prompt-file`. |
| `--heartbeat-prompt-file` | `BUZZ_ACP_HEARTBEAT_PROMPT_FILE` | — | Read heartbeat prompt from a file. Conflicts with `--heartbeat-prompt`. |
| `--heartbeat-preflight-config` | `BUZZ_ACP_HEARTBEAT_PREFLIGHT_CONFIG` | — | Legacy inline owner config for unprotected agents. Managed must-check agents use the durable policy-file settings below. |
| `--heartbeat-preflight-required` | `BUZZ_ACP_HEARTBEAT_PREFLIGHT_REQUIRED` | `false` | Durable managed-agent latch. When true, missing or invalid policy-file settings fail startup instead of falling back to an ordinary heartbeat. |
| `--heartbeat-preflight-policy-file` | `BUZZ_ACP_HEARTBEAT_PREFLIGHT_POLICY_FILE` | — | Absolute owner-policy file re-opened before every heartbeat. Required with the latch. |
| `--heartbeat-preflight-policy-sha256` | `BUZZ_ACP_HEARTBEAT_PREFLIGHT_POLICY_SHA256` | — | Exact lowercase SHA-256 pin for the durable policy file. Required with the latch. |
| `--required-agent-owner` | `BUZZ_ACP_REQUIRED_AGENT_OWNER` | — | Exact lowercase 64-hex owner pin. When set, startup fails before relay, preflight, or model activity unless the owner resolved from a verified `BUZZ_AUTH_TAG` (preferred) or `BUZZ_ACP_AGENT_OWNER` matches exactly. |

Heartbeat preflight currently requires Unix process-group containment. Builds
on other operating systems reject the configuration rather than risk leaving a
gateway descendant alive with forwarded IPC credentials.

The harness mints each heartbeat's turn UUID together with one immutable
request timestamp and uses the UUID as the preflight invocation and
receipt-acceptance context. An idempotent retry of that same turn reuses both
values, so the gateway may return the byte-identical durable terminal receipt;
a different turn cannot consume it. Every checked source must match the
owner-pinned `{source, account, scope, policy_id}` tuple and return a
`witness_run_id`, `receipt_digest`, and `acceptance_context` equal to that exact
invocation. The request and result also bind the exact target channel and a
64-hex digest of the owner declaration that commits the actor, channel,
sources, and exactly-one-zone assignments. Checked timestamps older than the
turn's immutable request boundary are rejected.
The required policy's `heartbeat_interval_seconds` must exactly match the
Desktop-owned designation; missing, zero, out-of-range, or mismatched cadence
fails before an ACP/model process can be used.

A successful result must also include `committed_material`, even when it is an
empty array. Each model-visible item names an owner-pinned source and distinct
entry ID, carries a content digest, is bound to the exact authority commit that
was read back remotely, and contains exactly one bounded sanitized payload or
immutable ledger pointer. Sanitized payloads are re-hashed by the harness and
each checked source's `item_count` must exactly equal its committed-material
entry count; blocked sources must carry none. The entire material section is
capped at 128 items and 64 KiB. A larger sanitized batch must be represented by
one bounded immutable aggregate ledger pointer or fail closed—silent truncation
is invalid. Only this typed, validated section reaches the prompt; raw gateway
stdout/stderr and connector transcripts never do.

Desktop also binds a designated agent to the bundled harness itself. The build
embeds a stable executable-code digest (excluding only the replaceable platform
signature payload and its signer-owned Mach-O load-command size fields), and
every spawn or reuse rechecks a non-symlink path, that digest, and the exact
versioned `heartbeat-preflight-capability` response. The
verified digest/protocol is stamped into the in-memory process and durable
runtime receipt. An old, substituted, or previously unstamped harness is not
reused.

On macOS, `buzz-acp` also embeds the same capability tuple as the exact
`BuzzHeartbeatPreflightCapability` scalar in Mach-O `__TEXT,__info_plist`.
Because the section is present before code signing, a verifier can authenticate
the capability as signed executable content without launching the candidate.

The executable boundary is intentionally narrow: Buzz verifies the pinned
executable, policy, result shape, scope, freshness, and invocation binding, but
does not treat those JSON fields as a signature. The pinned gateway must call
the accountability-ledger source witness and durably claim the signed receipt
in its atomic `AcceptanceStore` under the supplied invocation ID before it
emits a checked result. Missing, blocked, stale, replayed, or unaccepted proof
must exit nonzero or return a blocked outcome. A helper that merely echoes the
request does not satisfy this contract.

#### Gateway executable wire contract

The policy's absolute program is executed directly with the policy's literal
args (no shell). Its environment is empty except for explicitly allowlisted
gateway IPC metadata. Buzz writes exactly one compact JSON request plus a
newline to stdin, then closes stdin:

~~~json
{
  "version": 1,
  "kind": "buzz_heartbeat_preflight",
  "turn_id": "<harness UUID>",
  "invocation_id": "<same harness UUID>",
  "target_agent_pubkey": "<64 lowercase hex>",
  "target_channel": "<owner-pinned Buzz channel token>",
  "declaration_manifest_digest": "<64 lowercase hex>",
  "requested_at": "<stable RFC3339 turn start>",
  "required_sources": [
    {
      "source": "gmail",
      "account": "owner@example.com",
      "scope": "inbox",
      "policy_id": "gmail.required"
    }
  ],
  "ledger_instance_id": "<owner-pinned token>"
}
~~~

The executable must return one strict JSON object on stdout. It echoes the
request identity, target agent, target channel, declaration-manifest digest,
ordered four-field source manifest, and ledger instance; supplies equal valid
authority_commit and remote_readback_commit; supplies exactly one ordered
outcome per required source; and always supplies committed_material. A
checked outcome requires a distinct bounded witness_run_id, distinct 64-hex
receipt_digest, exact invocation acceptance_context, and an item_count equal to
that source's material-entry count. A blocked outcome instead requires a
bounded reason_code and carries no witness fields, item count, or committed
material.

The gateway is killed on timeout; nonzero exit, extra/unknown JSON fields,
malformed or oversized output, and any identity, freshness, scope, count,
digest, commit, or material mismatch fail the heartbeat before the ACP/model
prompt. Stderr is bounded for process safety but is never forwarded to the
model.

The policy path and forwarded gateway IPC capability must live outside the
agent's writable sandbox. File-mode and digest checks reject ordinary
replacement, but same-user filesystem access is not a credential boundary;
deployment is responsible for preventing the model/tool process from writing
the policy or reaching gateway signing/acceptance storage directly. Direct
execution does not itself authenticate the parent process; a same-user macOS
deployment therefore needs an OS-enforced service boundary (for example, peer
audit-token/code-requirement validation) so only the signed harness can invoke
the privileged connector surface.

This gate covers native scheduled heartbeat prompts only. Ordinary channel
mentions/messages follow the normal inbound-author and ACP paths and must not
be described as source-preflight-gated. The source-witness `AcceptanceStore`
remains an external trusted component: Buzz requires its scope-bound attested
output but does not replace it with a locally self-asserted acceptance record.

### Inbound Author Gate

Controls which authors' events the harness forwards to the agent. Events from disallowed authors are silently dropped before reaching subscription rules.

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--respond-to` | `BUZZ_ACP_RESPOND_TO` | `owner-only` | Author gate mode: `owner-only`, `allowlist`, `anyone`, `nobody`. |
| `--respond-to-allowlist` | `BUZZ_ACP_RESPOND_TO_ALLOWLIST` | — | Comma-separated 64-char hex pubkeys (required when mode is `allowlist`). Owner is always implicitly included. |

**Modes:**

| Mode | Behavior |
|------|----------|
| `owner-only` | Forward only events from the agent's registered owner. If no owner is set, all events are dropped until the owner is resolved. |
| `allowlist` | Forward events from the listed pubkeys plus the owner. |
| `anyone` | Forward all events (no author filtering). |
| `nobody` | Drop all inbound events. Agent only acts on heartbeat prompts. |

The gate applies to **all** inbound events — @mentions, DMs, thread replies, and any event delivered by the relay. Owner control commands are checked **before** the gate, so the owner can still manage the harness regardless of mode:

| Command | Effect |
|---------|--------|
| `!shutdown` | Gracefully exits the harness. |
| `!cancel` | Cancels the current in-flight turn for that channel, if any. |
| `!rotate` | Rotates the ACP session for that channel. If a turn is in-flight, it is cancelled and the channel session is invalidated when the task returns; otherwise the cached idle session is invalidated immediately. The next queued/received event starts a fresh session. |

Use `!cancel` to stop only the current turn; it is a no-op when the channel is idle. Use `!rotate` when you want the next turn in the channel to start from a fresh ACP session, even if the channel is currently idle.

Owner control commands must be kind:9 stream messages from the owner, must mention this agent with a `p` tag, and are consumed by the harness instead of being forwarded to the agent.

> **Note:** The default mode is `owner-only`. Agents without a registered `agent_owner_pubkey` will not respond to any events until the owner is resolved. Set `--respond-to anyone` to disable the gate entirely.

**Examples:**

```bash
# Default: only respond to owner
buzz-acp

# Respond to a team of three users (owner always included automatically)
buzz-acp --respond-to allowlist \
  --respond-to-allowlist "abc123...64hex,def456...64hex,789abc...64hex"

# Respond to anyone (open agent)
buzz-acp --respond-to anyone

# Broadcast-only: post on heartbeat, ignore all inbound events
buzz-acp --respond-to nobody --heartbeat-interval 300
```

### Configuration Examples

**Single agent, no heartbeat (default):**
```bash
buzz-acp
```

**Four agents, no heartbeat (high-throughput event processing):**
```bash
buzz-acp --agents 4
```

**Two agents with 5-minute heartbeat:**
```bash
buzz-acp --agents 2 --heartbeat-interval 300
```

**Custom heartbeat prompt:**
```bash
buzz-acp --agents 2 --heartbeat-interval 300 \
  --heartbeat-prompt "Check get_feed_actions() for pending approvals, then get_feed_mentions() for unanswered mentions. If nothing actionable, end your turn immediately."
```

### Shared Identity

All N agents authenticate as the **same Nostr bot identity** — users see one bot regardless of how many agents are running. The same channel is never processed by two agents simultaneously (the queue enforces this). Cross-channel message ordering is not guaranteed when N>1.

### Heartbeat Semantics

When `--heartbeat-interval` is set, the harness fires a prompt on an idle agent at the configured interval. Heartbeat rules:

- **Lower priority than queued events** — if events are pending, they are dispatched first.
- **Skipped when all agents are busy** — no queuing; the tick is simply dropped.
- **At most one heartbeat in flight globally** — the next tick is suppressed until the current one completes.
- **Default prompt** (when `--heartbeat-prompt` is not set) calls `get_feed_actions()` and `get_feed_mentions()` to surface pending work.

Heartbeat is designed for idle periods. Under sustained event load it will rarely fire — that's expected.

### Choosing N

Start with **N=2** for most deployments. Increase if queue depth grows under load. Each agent spawns its own MCP server subprocess, so resource usage scales approximately as N × (agent memory + MCP server memory). Maximum is 32.

## Forum Channels

By default, the ACP harness subscribes to stream message kinds (9, 46010, 40007). To receive forum events, opt in with `--kinds` and disable the mention filter (forum posts don't @mention agents):

**CLI flags:**
```bash
buzz-acp --kinds 9,46010,40007,45001,45002,45003 --no-mention-filter
```

**Or with `--subscribe all`:**
```bash
buzz-acp --subscribe all --kinds 9,46010,40007,45001,45002,45003
```

**Per-channel config:**
```toml
[channel.CHANNEL_UUID]
kinds = [9, 46010, 40007, 45001, 45002, 45003]
require_mention = false
```

Forum event kinds:
- **45001** — Forum post (thread root)
- **45002** — Vote on a post or comment
- **45003** — Comment reply on a forum post

> **Note:** Without `--no-mention-filter` (or `require_mention = false`), the default `subscribe=mentions` mode filters events that don't @mention the agent — forum posts will be invisible.

## How It Works

1. **Startup** — Spawns N agent subprocesses (default 1), sends ACP `initialize` to each, connects to the relay with NIP-42 auth.
2. **Channel discovery** — Queries the relay REST API for accessible channels, subscribes to each.
3. **Event loop** — Listens for @mention events (kind 9 with the agent's pubkey in a `#p` tag). Events queue per channel.
4. **Prompting** — When events are pending and no prompt is in flight for that channel, drains all queued events for the oldest channel into a single batched prompt via ACP `session/prompt`.
5. **Agent response** — The agent processes the prompt and uses the Buzz CLI (`send_message`, `get_messages`, etc.) to interact with Buzz.
6. **Recovery** — If the agent crashes, the harness respawns it. If the relay disconnects, the harness reconnects with a `since` filter to avoid missing events.

Each channel has at most one prompt in flight. Multiple channels can be processed concurrently when agents > 1.

> **Note:** On startup, the harness replays all unprocessed @mentions since the last run. Expect a burst of activity if there are stale events in the channel.

## Bring Your Own Harness (BYOH)

Buzz Desktop supports registering any ACP-speaking agent tool as a selectable runtime without a PR.

### How it works

**Tier-1 — compiled-in runtimes** (Goose, Claude Code, Codex, Buzz Agent): have auto-installers, auth probes, and first-class onboarding. Their IDs (`goose`, `claude`, `codex`, `buzz-agent`) are reserved and cannot be overridden.

**Tier-2 — preset catalog** (Cursor, Oh My Pi, Grok Build, OpenCode, Kimi Code, Amp, Hermes Agent, OpenClaw): static `HarnessDefinition` entries in `desktop/src-tauri/src/managed_agents/discovery.rs` (`PRESET_HARNESSES`). They are always present in the runtime catalog, PATH-probed for availability, not editable or deletable by the user. Displayed with bundled logos; if not installed, a docs link appears instead.

> **Note — OpenClaw:** `openclaw acp` is a Gateway-backed bridge; PATH availability shows "Available" even when the OpenClaw Gateway daemon is not running. This is expected tier-2 semantics (same class as a preset with unconfigured auth). The Gateway URL is configured via `OPENCLAW_GATEWAY_URL` (or the equivalent env var from OpenClaw's docs) — set it in the agent's **env vars** in Edit Agent, not in the definition env (the preset definition carries no env entries). Note that `openclaw acp` executes tools inside the Gateway daemon, not the Desktop process, so Desktop-injected `BUZZ_*` env vars do NOT reach the execution locus unless you also set them on the Gateway's own environment.

**Tier-3 — user custom harnesses**: JSON files in `<app-data>/custom_harnesses/` that the user can create from the Settings UI or drop in directly. Each file describes one harness — no install scripts.

### Custom harness JSON schema

```json
{
  "id": "my-agent",
  "label": "My Agent",
  "command": "my-agent-bin",
  "args": ["acp"],
  "env": {
    "MY_AGENT_MODE": "acp"
  },
  "installInstructionsUrl": "https://example.com/docs",
  "installHint": "Download from example.com"
}
```

Fields:
- `id` — `[a-z0-9_][a-z0-9_-]*` (used as the runtime picker value and file name)
- `label` — human-readable name shown in the UI
- `command` — the executable name or absolute path (must be non-empty)
- `args` — optional default CLI arguments (array); instance-level args override this when non-empty
- `env` — optional environment variables injected at spawn time (definition env is a floor; user/persona/global env overrides it; Buzz-reserved keys like `BUZZ_MANAGED_AGENT` are always stripped and cannot be overridden)
- `installInstructionsUrl` / `installHint` — shown when the binary is not on PATH

Invalid files (bad JSON, unknown id, empty command) are skipped with a warning and do not break discovery for other entries.

### Security guarantees

- No install shell commands in preset or custom definitions — only the user's own PATH is consulted.
- `can_auto_install` is always `false` for preset and custom entries.
- No user-supplied icon URLs — icons are bundled assets keyed by id in `RuntimeIcon.tsx`.
- `BUZZ_MANAGED_AGENT` and other Buzz identity keys cannot be overridden by `env` in a custom definition; they are stripped before merging.

### Adding a preset (contributor guide)

To add a new runtime to the tier-2 gallery:

1. **Verify the ACP entrypoint** from the vendor's own documentation — do not rely on a PR description alone. Test with the actual binary.
2. **Add a `HarnessDefinition` entry** to the `PRESET_HARNESSES` slice in `desktop/src-tauri/src/managed_agents/discovery.rs`. Fill `id`, `label`, `command`, `args`, `install_instructions_url`, `install_hint`. Leave `env` empty unless the harness requires a specific env var to enable ACP mode.
3. **Add the preset id to `BUILTIN_IDS`** in `desktop/src-tauri/src/managed_agents/custom_harnesses.rs` so custom JSON files cannot shadow it.
4. **Add a bundled logo** (64×64 PNG or optimised SVG) to `desktop/public/harness-logos/<id>.png` and add a corresponding entry to `PRESET_LOGOS` in `desktop/src/features/onboarding/ui/RuntimeIcon.tsx`. Record the source and license in `desktop/public/harness-logos/CREDITS.md`. Only bundle a mark whose upstream license permits redistribution; skipping this step is caught by `presetLogos.test.mjs`, which asserts every `PRESET_HARNESSES` id has a mapped logo that exists on disk.
5. Run `cargo test --lib` and `just desktop-typecheck` to verify everything compiles.

The built-in `BUILTIN_IDS` set (`goose`, `claude`, `codex`, `buzz-agent`, and all current preset ids) is the reserved namespace; every other id is available for custom harnesses.

## Using Any ACP Agent

The harness works with any agent that implements the [ACP spec](https://agentclientprotocol.com/) over stdio. The requirements are:

- Accept `initialize` and return a result
- Accept `session/new` with `mcpServers` and return a `sessionId`
- Accept `session/prompt` with a text message and stream `session/update` notifications
- Return a `stopReason` (`end_turn`, `cancelled`, `max_tokens`, etc.)

Set `BUZZ_ACP_AGENT_COMMAND` and `BUZZ_ACP_AGENT_ARGS` to point at your agent binary.

## Testing

See the [root TESTING.md](../../TESTING.md) for the full integration testing guide — automated test suites, multi-agent E2E testing via the ACP harness, and troubleshooting.

## License

Apache-2.0
