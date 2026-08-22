# systemd units for always-on Buzz agents

These units run [`buzz-acp`](../../crates/buzz-acp/README.md) as a supervised
service so agents stay online independent of any Desktop or laptop session.

See the full framework: [docs/self-host-backend-framework.md](../../docs/self-host-backend-framework.md).

## Files

| File | Purpose |
|------|---------|
| `buzz-acp@.service` | Template unit (`buzz-acp@coder`) |
| `buzz-acp.env.example` | Environment skeleton (copy per agent) |

## Install

```bash
sudo cp buzz-acp@.service /etc/systemd/system/
sudo mkdir -p /etc/buzz-acp
sudo cp buzz-acp.env.example /etc/buzz-acp/coder.env
sudo edit /etc/buzz-acp/coder.env   # keys + BUZZ_RELAY_URL
sudo systemctl daemon-reload
sudo systemctl enable --now buzz-acp@coder
sudo systemctl status buzz-acp@coder
```

User units: install under `~/.config/systemd/user/` and use `systemctl --user`.

## Requirements

- `buzz-acp` installed at `/usr/local/bin/buzz-acp` (or edit `ExecStart`)
- Relay reachable at the **same host string** clients use
- Agent pubkey is a relay/channel member as required by your join policy
