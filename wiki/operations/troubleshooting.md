# Troubleshooting

Common issues when running Buzz locally.

## "Can't reach the relay" / "relay returned 521 Unknown Status Code"

521 is a [Cloudflare-specific status code](https://en.wikipedia.org/wiki/HTTP_521) meaning Cloudflare's edge can't reach the origin server. This happens when the desktop app is trying to connect to a Cloudflare-proxied URL (e.g. `wss://buzz.block.builderlab.xyz` or a staging/production relay) that is down or unreachable.

**Fix:** Connect to your local relay instead — use `ws://localhost:3000` as the relay URL.

If the app cached a previous cloud URL, clear the stored state:

```bash
just desktop-standalone fresh=1
```

Then start fresh:

```bash
just dev
```

The error is generated in `desktop/src-tauri/src/relay.rs:295` (HTTP fallback) and classified in `desktop/src/shared/lib/relayError.ts` (checks for `relay unreachable:` prefix — 521 doesn't match it, so it's treated as an application error rather than a network error).

## "relay unreachable: could not connect to relay"

The relay process isn't running or isn't reachable on the expected port.

**Checks:**

```bash
# Is the relay binary running?
ps aux | grep buzz-relay

# Is it listening on the right port?
lsof -i :3000

# Are Docker services up?
docker compose ps

# Check health endpoints
curl http://localhost:8080/_liveness
curl http://localhost:8080/_readiness
```

**Fix:** Start the relay:

```bash
just relay
```

## Port already in use

```
Error: relay port 3000 is already in use; refusing to launch desktop against a stale relay.
```

A previous relay instance is still running. Kill it:

```bash
pkill buzz-relay
just dev
```

## "relay rate-limited: retry in Ns"

The relay's Redis-backed rate limiter has been hit. Wait the indicated time and retry. Rate limits are configured via `BUZZ_RATE_LIMIT_*` env vars in `.env`.

## Docker services not healthy

If `docker compose ps` shows `unhealthy`:

```bash
# Check individual service logs
docker compose logs postgres
docker compose logs redis

# Full reset
just reset
```

## Relay starts but immediately exits

Check the relay logs for database connection errors. Ensure Postgres is healthy and `DATABASE_URL` in `.env` is correct (default: `postgres://buzz:buzz_dev@localhost:5432/buzz`).

## "No auth challenge received"

The WebSocket connection succeeded but the relay didn't send a NIP-42 AUTH challenge. This usually means the relay is not configured to require auth (`auth_required: true` in the NIP-11 response). Check `RELAY_URL` in `.env` points to the correct relay.

## "Event rejected by relay"

The relay rejected a published event. Possible causes:

- Event signature verification failed
- Sender is not a member of the target channel
- Event kind is not allowed
- Rate limit exceeded

Check the relay logs for details (`RUST_LOG=debug` is set in `.env`).

## Agent shows response text in a "bubble" but never publishes to the channel

The agent generates text (GPU spikes, LLM runs, text appears in a UI bubble) but the message never shows up in the channel timeline.

### Cause 1: Agent connected to the wrong relay

The agent's `managed-agents.json` may have a hardcoded `relay_url` pointing to a different relay than the one you're using. Check:

```bash
cat ~/Library/Application\ Support/xyz.block.buzz.app.dev.main/agents/managed-agents.json
```

If you see `relay_url` set to a remote URL (e.g. `wss://milimo.communities.buzz.xyz`) instead of `ws://localhost:3000`, the agent publishes responses to the wrong relay.

**Fix:** Edit the file to replace the remote URL with your local relay:

```bash
sed -i '' 's|wss://milimo.communities.buzz.xyz|ws://localhost:3000|g' \
  ~/Library/Application\ Support/xyz.block.buzz.app.dev.main/agents/managed-agents.json
```

Then kill the agent processes — the desktop app will respawn them with the correct URL:

```bash
pkill -f buzz-acp
pkill -f buzz-agent
pkill -f buzz-dev-mcp
```

### Cause 2: LLM returns text but doesn't call the publish tool

Buzz has two separate rendering paths for agent content:

| Aspect | Session Transcript (ACP Panel) | Channel Timeline |
|---|---|---|
| Event source | Kind 24200 observer frames (ACP protocol) | Kind 9 / 40002 Nostr events |
| Agent text | `agent_message_chunk` → transcript item | Only if agent publishes explicitly |
| Where shown | Side panel (click "View activity") | Main channel content area |

The agent generates text and sends it via ACP's `agent_message_chunk` update — this renders in the session transcript panel as a "bubble." But the harness (`buzz-acp`) does **not** auto-publish agent output to the relay. At `crates/buzz-acp/src/lib.rs:3232`, `PromptOutcome::Ok(_)` just returns the agent to the pool without sending anything.

The agent must **explicitly publish** its response by calling one of:
1. `buzz messages send` via the MCP `shell` tool (`buzz-dev-mcp` provides this)
2. The `send_message` ACP tool (dedicated agent tool)
3. The desktop Tauri IPC `send_managed_agent_channel_message` handler

To verify if the agent is calling the tool, check the `buzz-dev-mcp` logs:

```bash
cat ~/Library/Application\ Support/xyz.block.buzz.app.dev.main/agents/logs/*.log | grep -i 'shell\|tool_call\|buzz messages'
```

### Cause 3: Local LLM returns reasoning in `reasoning_content` or lacks native tool-calling

When using local OpenAI-compatible LLM endpoints (e.g. OMLX, Ollama, or local 4-bit/8-bit models like `gemma-4-12b`):

1. **Empty `content` with non-empty `reasoning_content`**: Local reasoning models output text under `reasoning_content` or `reasoning` while leaving `content: ""`. Both `parse_openai` and `parse_responses` in `buzz-agent` (`crates/buzz-agent/src/llm.rs`) fall back to `reasoning` when `text` is empty and no tool calls exist.
2. **Fallback Turn Text Publishing**: When models output text without executing a `buzz messages send` tool call, `buzz-acp` (`crates/buzz-acp/src/pool.rs`) accumulates `agent_message_chunk` text during the turn and posts it directly to the channel as a `kind: 9` event upon `StopReason::EndTurn`.
3. **Cleaning Pseudo-tool Code Blocks**: `clean_agent_text_response` strips trailing ` ```json ... ``` ` parameter blocks and special model tokens (`<|tool_call>`, `<|im_end|>`) so that only clean conversational text is posted to the channel.

## "Agent; owner unavailable" badge next to agent name

The desktop UI displays **"owner unavailable"** when the agent user profile in the Postgres database lacks an `agent_owner_pubkey` mapping to the human owner.

**Fix:** Ensure the active agent pubkeys are inserted into the `users` table with `agent_owner_pubkey` set to the owner's pubkey across all active community host entries (`localhost:3000`, `127.0.0.1:3000`, `localhost`, `127.0.0.1`):

```sql
INSERT INTO users (community_id, pubkey, agent_owner_pubkey, created_at, updated_at, channel_add_policy)
SELECT c.id, decode('<agent_pubkey_hex>', 'hex'), decode('<owner_pubkey_hex>', 'hex'), NOW(), NOW(), 'anyone'::channel_add_policy
FROM communities c
ON CONFLICT (community_id, pubkey) DO UPDATE SET agent_owner_pubkey = EXCLUDED.agent_owner_pubkey;
```

## "failed to parse agent store: missing field 'acp_command'" error banner

The desktop app displays a red warning banner `failed to parse agent store (preserved as .invalid): missing field 'acp_command'` when `managed-agents.json` is missing required non-defaulted fields for `ManagedAgentRecord`.

**Fix:** Ensure `managed-agents.json` contains all required schema fields (`pubkey`, `name`, `relay_url`, `acp_command` [e.g. `"buzz-acp"`], `agent_command` [e.g. `"buzz-agent"`], `mcp_command` [e.g. `"buzz-dev-mcp"`], `turn_timeout_seconds` [e.g. `900`], `agent_args` [`[]`]) and remove any `.invalid` backup file:

```bash
rm -f ~/Library/Application\ Support/xyz.block.buzz.app.dev.main/agents/managed-agents.json.invalid
```

**Related:**
- [DevelopmentSetup](development-setup) — how to start the dev environment
- [Configuration](configuration) — env var reference
- [CLIReference](cli-reference) — just commands
- [Relay](../entities/relay) — relay entity docs

