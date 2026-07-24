# Attach Rocky (Hermes default profile) to Buzz

This guide is for an **owner-only** Buzz attachment of an existing Hermes profile.
Hermes keeps config, tools, skills, memory, credentials and approvals. Buzz is the
desktop + phone conversation surface.

```text
Buzz Desktop / relay  <-->  buzz-acp  <-->  hermes acp (stdio ACP)
```

## Prerequisites

1. Hermes Agent installed with ACP support:
   ```bash
   hermes acp --check   # or: hermes-acp --check
   ```
2. Profile tool policy enabled in the Hermes home you will attach (Rocky default):
   ```yaml
   # ~/.hermes/config.yaml
   acp:
     tool_policy: profile
   ```
   This uses the profile’s local CLI tool configuration (full capability), not the
   coding-only `hermes-acp` toolset. Requires Hermes
   [PR #70326](https://github.com/NousResearch/hermes-agent/pull/70326) or equivalent.
3. A Buzz build that includes Hermes runtime discovery and durable `session/load`
   (this PR / branch `feat/hermes-native-profile` or later).

## Managed agent settings

| Field | Value |
|---|---|
| Runtime | Hermes Agent |
| Command | `hermes` |
| Args | leave empty (normalises to `acp`) or set `-p <profile> acp` for a non-default profile |
| Parallelism | **1** (one process per profile home — higher values spawn N ACP processes against the same home and fail cold-start turns) |
| Respond-to | **owner-only** |
| Memory | disable Buzz NIP-AE memory so Hermes memory stays sole memory |

### Environment

For Rocky (default profile home):

```text
HERMES_HOME=/Users/you/.hermes
BUZZ_ACP_NO_MEMORY=true
```

Important:

- `BUZZ_ACP_NO_MEMORY` must be the string **`true`** (not `1`). Other values exit the harness with code 2.
- Do **not** put Buzz transport secrets in Hermes config. Keep them on the Buzz agent record only.
- Prefer **instance** config (not only definition) for `parallelism=1` and env — definition defaults can be overridden by instance values.

## Behaviour notes

- **Full tools:** with `acp.tool_policy: profile`, Rocky gets the same tool surface as interactive CLI for that profile (skills, memory, browser, kanban, cron, delegation, … as configured).
- **Session continuity:** after Buzz/harness restart, `session/load` restores the prior ACP session when the agent advertises `loadSession`. History replay stays on the wire log path and is not re-published to Buzz.
- **Permissions:** current Buzz auto-approves ACP permission requests. Acceptable for private owner-only dogfood only.
- **Workers:** Chad/Oscar stay Hermes-internal via Rocky’s `delegate_task`. Do not attach them to Buzz unless you later want separate identities.
- **Gateway concurrency:** if Telegram/Discord gateway is also running against the same `HERMES_HOME`, prefer Buzz as the primary interactive surface and pause the gateway if browser/session contention appears.
- **Process cleanup:** Hermes console scripts often run under `python`/`python3`. Desktop treats those exact names as marker-owned interpreter candidates (same pattern as `node`).

## Live dogfood (2026-07-24)

Owner-only Rocky DM on Desktop succeeded after:

1. `acp.tool_policy: profile` on the Hermes home
2. Managed agent: command `hermes`, args empty, `HERMES_HOME` set, `BUZZ_ACP_NO_MEMORY=true`
3. Instance **parallelism forced to 1** (definition-only edits were not enough while instance still had 24)
4. Harness restart after the instance change

Observed: full Hermes tools (skills, memory, browser, …), successful multi-turn DM replies, durable session bind via `session/load` path.

## Headless harness (optional)

```bash
export BUZZ_PRIVATE_KEY=...
export BUZZ_RELAY_URL=...
export BUZZ_ACP_AGENT_COMMAND=hermes
export BUZZ_ACP_AGENT_ARGS=acp   # or leave empty; default becomes acp
export HERMES_HOME=/Users/you/.hermes
export BUZZ_ACP_NO_MEMORY=true
# respond-to owner-only via your usual harness flags
# keep parallelism at 1 for a single profile home
buzz-acp
```

## Multi-profile later

When you want another Hermes profile on Buzz, add a **second** managed agent
with **unique ACP args** so durable session bindings do not collide:

```text
command: hermes
arguments: -p chad acp
HERMES_HOME=/Users/you/.hermes/profiles/chad   # optional, for the process env
parallelism: 1
```

Do **not** run two managed agents with identical command/args (e.g. both
`hermes` + empty/`acp` args) and only different `HERMES_HOME` values. The durable
session store is keyed by command + args + channel, not by environment or
managed-agent identity, so those two agents would overwrite each other's channel
bindings. Until the store is namespaced by agent identity, unique `-p <profile>
acp` args are required for multi-profile.

Keep `parallelism=1` per profile.

## Scope relative to other PRs

- **This PR:** local Desktop/managed Hermes runtime discovery, arg normalisation, durable `session/load`, onboarding picker, owner Rocky dogfood guide. Intentionally does **not** include external-agent directory / kind `10100` / relay-observer presentation work.
- **#2468 (nytemode):** broader external-agent hosting + VPS observer path. Currently conflicting with `main`. Ready-to-steal pieces already folded here: command-specific `hermes acp --check` readiness as AdapterMissing, exact `python`/`python3` process recognition.
- **Deeper native Hermes integration** (first-class Nous product surface inside Buzz, shared memory protocols, etc.) is still best done by the Nous Research team upstream.

## Staying alive (OAuth / long-lived ACP)

Rocky over Buzz is a **long-lived** `hermes acp` process. Telegram/Discord/gateway
stay warm and refresh tokens; a parked ACP worker can hold a stale xAI access
token after hours of idle.

**One-time ops**
1. Rocky managed agent: **Start on app launch** on
2. **Parallelism = 1** (instance, not only definition)
3. After this Hermes fix is installed: restart Rocky once so new ACP sessions
   get `credential_pool` and can self-heal on `403 bad-credentials`

**You should not need reauth** when Telegram still works. Prefer restart Rocky
in Buzz Agents if a single turn fails with OAuth 403 after long idle.

**Fixed upstream** (Hermes): ACP now wires `credential_pool` like the gateway,
and 403 auth refreshes the same path as 401 for xAI OAuth.

