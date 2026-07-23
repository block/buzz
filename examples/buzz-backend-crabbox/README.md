# buzz-backend-crabbox

First-class **Run on → Crabbox** backend for Buzz Desktop.

Buzz keeps ownership of the agent (keys, channels, relay). Crabbox hosts the
harness on a remote lease. See the product runbook:
[`docs/backend-providers/crabbox.md`](../../docs/backend-providers/crabbox.md).

```text
Buzz Desktop ──JSON──▶ buzz-backend-crabbox ──CLI──▶ crabbox lease
                              │
                    stage buzz-acp + tools
                              ▼
                       remote buzz-acp ──▶ Buzz relay
```

## One-shot install

```bash
just install-backend-crabbox
brew install openclaw/tap/crabbox
crabbox login --url <broker-url>
```

Restart Desktop. Create an agent → **Run on → Crabbox**.

## Manual install

```bash
cargo build --release -p buzz-acp -p buzz-agent -p buzz-cli -p buzz-dev-mcp
export PATH="$PWD/target/release:$PATH"
./examples/buzz-backend-crabbox/install.sh   # → ~/.local/bin/buzz-backend-crabbox
```

## Protocol

| Op | Result |
|----|--------|
| `info` | `{ ok, name: "Crabbox", version, description, config_schema }` |
| `deploy` | Warm/reuse lease, stage toolchain, start harness → `{ ok, agent_id }` |
| `stop` | Kill remote harness; keep lease |
| `destroy` | Kill harness + `crabbox stop` (Desktop agent delete) |

```bash
echo '{"op":"info","request_id":"1"}' | buzz-backend-crabbox | jq .
just test-backend-crabbox
```

## Security

Desktop warns that the provider receives the agent private key. Secrets use
Crabbox env helpers (not argv). Loopback relay URLs are rejected.

## Lifecycle

| Action | How |
|--------|-----|
| Start / redeploy | Desktop **Deploy** |
| Soft stop | Desktop **Shutdown** / channel `!shutdown` |
| Delete agent | Desktop delete → provider `destroy` → lease released |
| Manual release | `crabbox stop <lease-id-or-slug>` |
