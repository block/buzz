# Adaptive command memory

Command Adviser keeps two views of adviser experience:

- encrypted, append-only Buzz events are the durable history;
- Memory MCP derives the small active view used for the current task.

Corrections append a new record with `supersedes`; the older record remains in
history but is excluded from active recall. Specialist-private records are
visible only to that specialist. Command-Team-shared records are visible to the
named team.

## Mac-local service

Install or refresh the loopback-only Memory MCP service from a reviewed
AgentMemory checkout:

```bash
MEMORY_MCP_SOURCE="/path/to/AgentMemory/MemoryMCPServer" \
  bash scripts/commission-adaptive-memory.sh
```

The service runs as the user's LaunchAgent at
`http://127.0.0.1:18006/mcp`. Its persistent files are under:

```text
~/Library/Application Support/Command Adviser/Memory/
```

Set `memory_url` in `trusted-lan-sources.json` to that endpoint. The agent
runtime then records locally and can continue when the home LAN is absent.
The existing home Memory MCP may remain configured as a normal agent tool for
older connected-history queries.

Check the service:

```bash
launchctl print "gui/$(id -u)/com.navigatorran.command-adviser-memory"
```

Run the repeatable code gates:

```bash
. ./bin/activate-hermit
MEMORY_MCP_REPO="/path/to/AgentMemory" \
PYTHON="/path/to/AgentMemory/MemoryMCPServer/.venv/bin/python" \
  bash scripts/check-adaptive-memory.sh
```

## Failure behaviour

- Relay publication or Memory projection failure leaves work in the local
  SQLite outbox for retry.
- Memory MCP failure does not block an adviser turn; the runtime logs
  `experience_recall_degraded` and continues with core plus recent context.
- Rebuild reads the encrypted Buzz history again; it does not delete that
  history.
- Recall includes source event IDs and labels the material as historical
  evidence, not instructions.

The original plan proposed a second canonical consolidation event layer. It is
intentionally omitted from the first usable release: Memory MCP's deterministic
active-leaf projection already provides current, corrected and superseded
views. This avoids maintaining two equivalent derived heads.

## Rollback

Restore the previous `memory_url` configuration, then unload the local service:

```bash
launchctl bootout \
  "gui/$(id -u)/com.navigatorran.command-adviser-memory"
```

Unloading does not delete the local vault. Do not remove the Memory directory
unless its contents have been backed up and the deletion is explicitly
approved.
