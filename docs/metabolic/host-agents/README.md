# Host agents (ability S1)

**Control plane** for 24h host-pinned seats. Not a second bus.

| Piece | Path |
|-------|------|
| Registry | `~/.buzz-dev/hosts/<host_role>/registry.json` |
| CLI | `buzz-host-agents` (install from this dir) |
| Ability SoT channel | `buzz-ability-host-agents` · `1f00dcd1-cf71-4410-bab7-32c1d226e61d` |
| Theory SoT | `#host-agents` · `45522703-6bbf-4ab7-90ab-0b1440c8e73a` |

## Install (home or laptop)

```bash
# symlink into PATH
ln -sf "$(pwd)/buzz-host-agents" ~/.local/bin/buzz-host-agents
chmod +x buzz-host-agents

# first-time registry seed (home)
export BUZZ_HOST_ROLE=home
export BUZZ_HOST_ID=asus-g501vw   # or hostname
buzz-host-agents init
buzz-host-agents status
```

## Commands

```bash
buzz-host-agents init              # create registry skeleton
buzz-host-agents path              # print registry path
buzz-host-agents list              # seats from registry
buzz-host-agents status            # relay · ollama · watch · seats (JSON + human)
buzz-host-agents status --post     # also post board card to ability channel
buzz-host-agents arm --preset co-lab-gemma --seat home-grok --room <uuid>
buzz-host-agents disarm --seat home-grok --preset co-lab-gemma
```

## Presets

| Preset | Effect |
|--------|--------|
| `co-lab-watch` | arm adapter watch only (no model) |
| `co-lab-gemma` | watch + local-llm · `BUZZ_DRIVER_DRY_RUN=0` · gemma3:4b |
| `push-nerve` / `codex-home` | Codex-style push L0 / session watcher on host |
| `status-only` | no process; status card only |

Arm writes a small unit file under the host dir and starts a background process group (no Desktop required).

## host-agentd (Remote Agents HTTP control)

Thin daemon for the traveling laptop UI (Hybrid plan: HTTP now, Nostr heartbeats later).

```bash
export HOST_AGENTD_TOKEN='long-random-secret'
export HOST_AGENTD_HOST=127.0.0.1   # or Tailscale IP on home
export HOST_AGENTD_PORT=8787
export BUZZ_HOST_ROLE=home
export BUZZ_HOST_AGENTS="$PWD/buzz-host-agents"
python3 host-agentd.py
```

```bash
# health
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" http://127.0.0.1:8787/v1/health
# status
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" http://127.0.0.1:8787/v1/status
# arm gemma
curl -sS -X POST -H "Authorization: Bearer $HOST_AGENTD_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"preset":"co-lab-gemma","room":"92297894-c2e8-4df1-a710-d1cfd1032d5e"}' \
  http://127.0.0.1:8787/v1/agents/home-grok/arm
```

See `host-agentd.service.example` for systemd --user on home.

## Env

| Var | Default |
|-----|---------|
| `BUZZ_HOST_ROLE` | `home` if hostname matches known home, else `laptop` |
| `BUZZ_HOST_ID` | `hostname -s` |
| `BUZZ_HOST_REGISTRY` | `~/.buzz-dev/hosts/$ROLE/registry.json` |
| `BUZZ_ABILITY_CHANNEL` | `1f00dcd1-cf71-4410-bab7-32c1d226e61d` |
| `BUZZ_ADAPTERS_DIR` | sibling `../adapters` or pack path |

Home already shipped a live `~/.local/bin/buzz-host-agents` (S1a+b). This mono copy is the **portable SoT** for arm recipes (S1c) and laptop dogfood.
