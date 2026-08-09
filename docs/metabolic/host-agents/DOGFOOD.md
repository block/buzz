# Remote Agents dogfood (laptop ↔ headless home)

## Rebuild?

| Who | Need Desktop rebuild? |
|-----|------------------------|
| home-grok / host-agentd | **No** — CLI + Python daemon |
| Traveling laptop Desktop | **Yes** — `feat/remote-agents-desktop` |
| Home Desktop GUI | Optional only |

## Home (already P3 GREEN)

```bash
# ensure daemon
systemctl --user status host-agentd.service
curl -sS -H "Authorization: Bearer $(cat ~/.buzz-dev/hosts/home/controller.token)" \
  http://127.0.0.1:8787/v1/health
```

Keep bind on `127.0.0.1` until laptop tunnel is proven (Codex gate).

## Laptop → home over Tailscale (mesh, no tunnel)

Tailscale is already the secure network. Prefer **mesh HTTP** to
host-agentd on the home Tailscale IP — do **not** require an SSH local
forward for day-to-day Remote Agents.

Home OS login user is **`asus`** (not laptop `trev`, not Grok session ids).

### Tailscale SSH (shell)

```bash
# works — real OS user on asus-g501vw
ssh asus@asus-g501vw
# or
ssh asus@100.79.175.63

# root also works under current tailnet SSH policy
ssh root@asus-g501vw
```

If you see `policy does not permit you to SSH as user "trev"`: that user
does not exist on home. Use `asus` (or `root`).

Enable/re-enable Tailscale SSH **on home** (once):

```bash
sudo tailscale set --ssh
```

### host-agentd bind (mesh)

On home, `HOST_AGENTD_HOST` should be the **Tailscale IP** (not 127.0.0.1):

```bash
# ~/.buzz-dev/hosts/home/host-agentd.env
HOST_AGENTD_HOST=100.79.175.63
HOST_AGENTD_PORT=8787
# HOST_AGENTD_TOKEN=…  (never post in public rooms)
systemctl --user restart host-agentd.service
ss -ltnp | grep 8787   # expect 100.79.175.63:8787
```

### Prove from laptop (no tunnel)

```bash
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" \
  http://100.79.175.63:8787/v1/health
# or MagicDNS:
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" \
  http://asus-g501vw.tailb74de6.ts.net:8787/v1/health
# expect: {"ok": true, "service": "host-agentd"}
```

- connection refused → daemon down or still bound to 127.0.0.1  
- `401` → fix the Bearer token  
- `200` + ok → Desktop Host URL is ready  

### Optional: SSH local forward (legacy)

Only if you intentionally keep host-agentd on loopback:

```bash
ssh -N -L 8787:127.0.0.1:8787 asus@asus-g501vw
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" http://127.0.0.1:8787/v1/health
```

## Desktop UI

```bash
cd <buzz-repo>
git checkout feat/remote-agents-desktop
just desktop-dev   # or just dev
# Agents → Remote Agents → Host
#   baseUrl: http://100.79.175.63:8787
#            (or http://asus-g501vw.tailb74de6.ts.net:8787)
#   token:   (from DM)
#   default room: agent-metabolism UUID
# Refresh → Arm co-lab-gemma / Stop
```

## Negative checks

```bash
# no auth
curl -sS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8787/v1/status
# expect 401

# bad token
curl -sS -o /dev/null -w '%{http_code}\n' \
  -H 'Authorization: Bearer wrong' http://127.0.0.1:8787/v1/status
# expect 401

# unknown preset
curl -sS -X POST -H "Authorization: Bearer $HOST_AGENTD_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"preset":"rm-rf"}' \
  http://127.0.0.1:8787/v1/agents/home-grok/arm
# expect 400 unknown preset
```

## Location proof (P6)

```bash
curl -sS -H "Authorization: Bearer $HOST_AGENTD_TOKEN" \
  http://127.0.0.1:8787/v1/location-proof | head
python3 location_proof.py --write
python3 location_proof.py --print-board   # optional post body for ability channel
```
