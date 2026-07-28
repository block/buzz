---
name: buzz-backend-crabbox
description: >
  Deploy Buzz managed agents onto Crabbox remote boxes from Desktop or the
  install recipe. Use when the user wants remote agent spin-up, Run on Crabbox,
  buzz-backend-crabbox, or leased agent compute outside this computer.
version: 1
---

# Buzz ↔ Crabbox backend

Crabbox is a **Desktop backend provider** for Buzz managed agents — not an LLM
provider and not a substitute for the relay. Buzz still owns identity, keys,
channels, and the agent record. Crabbox only hosts the `buzz-acp` harness on a
remote lease.

## Product surface (what users see)

1. **Agents → create agent**
2. **Run on → Crabbox** (appears after the provider is installed on PATH)
3. Optional config: Crabbox cloud provider, machine class, idle timeout, existing lease
4. Deploy → Desktop calls `buzz-backend-crabbox` with the standard agent payload
5. Agent badge shows **Crabbox**; runtime line shows lease id
6. **Shutdown** sends `!shutdown` (Buzz-native soft stop)
7. **Delete agent** calls provider `destroy` → releases the Crabbox lease

## Install (dev / OSS)

```bash
just install-backend-crabbox
# or: ./examples/buzz-backend-crabbox/install.sh
brew install openclaw/tap/crabbox
crabbox login --url <broker-url>
crabbox doctor
```

Restart Desktop so PATH discovery picks up `~/.local/bin/buzz-backend-crabbox`.

## Agent / operator rules

- **Relay must be reachable from the box.** Reject loopback `ws://localhost:…`
  unless the user has a tunnel; prefer the community’s real relay URL.
- **Do not put model or crabbox secrets in provider config.** Desktop rejects
  secret-shaped config keys. Crabbox auth is `crabbox login`; model keys go in
  agent/persona env vars (forwarded via Crabbox env helper).
- **Trust boundary.** The provider binary receives the agent nsec. Only install
  `buzz-backend-*` from this repo or a source the owner trusts.
- **Stop path.** Prefer channel `!shutdown` / Desktop Shutdown. Then
  `crabbox stop <lease>` so spend stops. There is no protocol `undeploy` in v1.
- **Reuse a warm box.** Set provider config `lease_id` to a slug/`cbx_…` from
  `crabbox list` instead of warming a new machine every deploy.

## Probe

```bash
echo '{"op":"info","request_id":"1"}' | buzz-backend-crabbox
just test-backend-crabbox
```

## Docs

- `docs/backend-providers/crabbox.md`
- `examples/buzz-backend-crabbox/README.md`
- https://crabbox.sh/
