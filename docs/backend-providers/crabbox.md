# Backend provider: Crabbox

**First-class remote run destination for Buzz managed agents.**

[Crabbox](https://crabbox.sh) hosts the agent harness on a leased remote box.
Buzz Desktop still owns identity, keys, channels, presence, and the agent
record. Crabbox does not replace the relay or LLM provider settings.

```text
Create agent → Run on → Crabbox → deploy
        │
        ▼
buzz-backend-crabbox  →  crabbox warmup / cp / run
        │
        ▼
remote buzz-acp (+ staged tools)  ──WS──▶  your Buzz relay
```

## Buzz product surface

| Surface | Behavior |
|---------|----------|
| **Run on** picker | Shows **Crabbox** (friendly name from `info` probe) when `buzz-backend-crabbox` is on PATH |
| Config fields | Crabbox provider, machine class, idle timeout, existing lease, remote workdir |
| Deploy | Desktop → provider `deploy` → warm lease, stage toolchain, start harness |
| Agent list | `Remote · crabbox · <lease-id-or-slug>` |
| Shutdown | Desktop **Shutdown** / channel `!shutdown` (Buzz-native) |
| Lease cleanup | `crabbox stop <lease>` (cost control) |

Install:

```bash
just install-backend-crabbox
brew install openclaw/tap/crabbox
crabbox login --url <broker-url>
crabbox doctor
# restart Buzz Desktop
```

Offline tests: `just test-backend-crabbox`.

## What deploy does

1. Warms a Crabbox lease (or reuses `lease_id` from provider config).
2. Stages local Buzz toolchain onto the box: **required** `buzz-acp`; best-effort
   `buzz`, `buzz-agent`, `buzz-dev-mcp`, credential helpers, plus the agent’s
   `agent_command` / `mcp_command` basenames when resolvable on PATH or in
   Desktop/cargo locations.
3. Installs a Crabbox env helper with the agent’s key, relay URL, and runtime
   env — **not** on shell argv.
4. Starts `buzz-acp` in the background under that helper.
5. Returns the lease id/slug as `agent_id` for Desktop (`backendAgentId`).

## Protocol (stdin / stdout JSON)

| Op | Purpose |
|----|---------|
| `info` | Name (**Crabbox**), version, description, config JSON Schema (enums) |
| `deploy` | Warm/reuse lease, stage binaries, start harness → `agent_id` |
| `stop` | Kill remote harness; keep lease warm |
| `destroy` | Kill harness + `crabbox stop` (Desktop agent delete) |

Safety: remote shell paths are shell-quoted; workdir/agent_id validated;
error text redacts nsec/API-key shapes; loopback relays rejected.

Request/response shapes match Desktop’s backend provider contract
(`discover_backend_providers` / `provider_deploy` / `provider_destroy` in
`desktop/src-tauri/src/managed_agents/backend.rs`).

**Soft stop** is still the in-channel `!shutdown` mention (Buzz-native).
**Hard cleanup** on agent delete calls provider `destroy` so the lease is released.

## Relay reachability

The remote box must reach `agent.relay_url`. The provider **rejects** loopback
relay URLs (`localhost`, `127.0.0.1`, `::1`). Point the agent at the community’s
reachable relay, or arrange a tunnel into the box.

## Security

- Provider config cannot carry secrets (Desktop validates key names).
- Crabbox broker credentials stay in local Crabbox user config (`crabbox login`).
- Agent env is written to a mode-`0600` temp profile, forwarded with
  `--allow-env`, and installed as a remote env helper.
- Prefer short idle timeouts for experiments; stop leases when done.
- Only install `buzz-backend-*` binaries from this repository or a trusted source.

## Troubleshooting

| Symptom | Check |
|---------|--------|
| Crabbox missing from Run on | `just install-backend-crabbox`; restart Desktop |
| probe fails | `echo '{"op":"info","request_id":"1"}' \| buzz-backend-crabbox` |
| deploy: crabbox not found | `brew install openclaw/tap/crabbox` |
| deploy: buzz-acp not found | `just install-backend-crabbox` |
| deploy: loopback relay | set a reachable `relay_url` on the agent |
| agent dies immediately | `crabbox ssh --id <lease>` → `tail -n 100 /work/buzz-agent/logs/agent.log` |
| box gone after idle | raise idle timeout or reuse **Existing lease** |

## Related

- Example + install: [`examples/buzz-backend-crabbox/`](../../examples/buzz-backend-crabbox/)
- Skill: [`.agents/skills/buzz-backend-crabbox/SKILL.md`](../../.agents/skills/buzz-backend-crabbox/SKILL.md)
- Desktop discovery: `desktop/src-tauri/src/managed_agents/backend.rs`
- Deploy payload: `desktop/src-tauri/src/commands/agents_deploy.rs`
- Crabbox docs: <https://crabbox.sh/>
