# Attach Rocky (Hermes default profile) to Buzz

This guide is for an **owner-only** Buzz attachment of an existing Hermes profile.
Hermes keeps config, tools, skills, memory, credentials and approvals. Buzz is the
desktop + phone conversation surface.

## Prerequisites

1. Hermes Agent installed with ACP support:
   ```bash
   hermes acp --check
   ```
2. Profile tool policy enabled in the Hermes home you will attach (Rocky default):
   ```yaml
   # ~/.hermes/config.yaml
   acp:
     tool_policy: profile
   ```
   This uses the profile’s local CLI tool configuration (full capability), not the
   coding-only `hermes-acp` toolset.
3. A Buzz build that includes Hermes runtime discovery and durable `session/load`
   (branch `feat/hermes-native-profile` or later).

## Managed agent settings

| Field | Value |
|---|---|
| Runtime | Hermes Agent |
| Command | `hermes` |
| Args | leave empty (normalises to `acp`) or set `-p <profile> acp` for a non-default profile |
| Parallelism | **1** (one process per profile home) |
| Respond-to | **owner-only** |
| Memory | disable Buzz NIP-AE memory (`BUZZ_ACP_NO_MEMORY=1` / no-memory) so Hermes memory stays sole memory |

### Environment

For Rocky (default profile home):

```text
HERMES_HOME=/Users/openclaw/.hermes
```

Optional (recommended for dogfood):

```text
BUZZ_ACP_NO_MEMORY=1
```

Do **not** put Buzz transport secrets in Hermes config. Keep them on the Buzz agent record only.

## Behaviour notes

- **Full tools:** with `acp.tool_policy: profile`, Rocky gets the same tool surface as interactive CLI for that profile (skills, memory, browser, kanban, cron, delegation, … as configured).
- **Session continuity:** after Buzz/harness restart, `session/load` restores the prior ACP session when the agent advertises `loadSession`.
- **Permissions:** current Buzz auto-approves ACP permission requests. Acceptable for private owner-only dogfood only.
- **Workers:** Chad/Oscar stay Hermes-internal via Rocky’s `delegate_task`. Do not attach them to Buzz unless you later want separate identities.
- **Gateway concurrency:** if Telegram/Discord gateway is also running against the same `HERMES_HOME`, prefer Buzz as the primary interactive surface and pause the gateway if browser/session contention appears.

## Headless harness (optional)

```bash
export BUZZ_PRIVATE_KEY=...
export BUZZ_RELAY_URL=...
export BUZZ_ACP_AGENT_COMMAND=hermes
export BUZZ_ACP_AGENT_ARGS=acp   # or leave empty; default becomes acp
export HERMES_HOME=/Users/openclaw/.hermes
export BUZZ_ACP_NO_MEMORY=1
# respond-to owner-only via your usual harness flags
buzz-acp
```

## Multi-profile later

When you want another Hermes profile on Buzz, add a **second** managed agent with either:

```text
arguments: -p chad acp
```

or

```text
HERMES_HOME=/Users/openclaw/.hermes/profiles/chad
```

Keep `parallelism=1` per profile.
