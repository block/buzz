# feat(spawner): first-party server-hosted agents via a buzz-spawner daemon

## Problem

Buzz agents can only be spawned by the desktop app.
`desktop/src-tauri/src/managed_agents/runtime.rs::spawn_agent_child` generates
the keypair, computes the NIP-OA owner attestation, and launches the `buzz-acp`
harness locally. Fizz/Honey/Bumble aren't special processes — they're built-in
personas (`managed_agents/personas.rs`) minted through that same local path.

This means:

- **Agents die with the laptop.** There is no always-on agent.
- **Agent creation requires the desktop app.** Mobile and web can't create one.
- **Self-hosters have no server-side option.** You can host a relay, but not an
  agent. The only server-side path is `BackendKind::Provider` → an external
  `buzz-backend-<id>` binary (`managed_agents/backend.rs:20-120`), which has no
  OSS implementation and still puts the desktop in the control loop.
- **The relay is entirely agent-agnostic** — no spawner, no agent registry, no
  lifecycle hooks. Agents are just pubkeys admitted via NIP-OA owner attestation
  (`crates/buzz-relay/src/api/mod.rs:40-200`).

## Proposal

Add `crates/buzz-spawner` — a standalone daemon that watches the relay for
agent-definition events and reconciles them into Docker-isolated `buzz-acp`
containers. It ships as a second service in `deploy/compose`.

It is deliberately **not** a relay module: the relay's only subprocess today is
`git`, and it goes out of its way to disable repo hooks
(`api/git/transport.rs:930-935`). Adding container execution to the public
WebSocket server isn't acceptable, and isn't necessary. `buzz-pair-relay` and
`buzz-push-gateway` are existing precedent for sidecar service crates.

The daemon is a plain relay client with its own Nostr identity. It never touches
Postgres.

```
owner client ──kind:30178 (spec)──►  relay  ──sub──►  buzz-spawner
     ▲                                                     │
     └──kind:24201 attestation handshake (NIP-44)──────────┤
                                                           ▼
                                              Docker: buzz-acp container
                                                           │
                                                    (agent's own key)
                                                           ▼
                                                         relay
```

## Design points

**Three new event kinds** (`buzz-core/src/kind.rs`), following the existing
30175/30176/30177 agent block:

| Kind | Purpose |
|---|---|
| `30178` `SPAWNER_AGENT_SPEC` | Desired state, owner-authored. `d` = spec slug |
| `30179` `SPAWNER_AGENT_STATUS` | Actual state, spawner-authored. Phase, agent pubkey, last error |
| `24201` `SPAWNER_ATTESTATION` | Ephemeral, NIP-44, `#p`-gated — the handshake |

Specs follow the strict opt-IN projection discipline documented in
`managed_agents/agent_events.rs` — the type must be physically incapable of
carrying an nsec, auth tag, or env blob.

**Attestation is a two-round handshake.** The nsec is minted on the VPS and never
leaves it. But NIP-OA's tag is
`Schnorr(SHA256("nostr:agent-auth:" || agent_pubkey || ":" || conditions), owner_secret)`
(`buzz-sdk/src/nip_oa.rs`) — it binds a specific agent pubkey and needs the
*owner's* secret key. So: owner publishes a spec → spawner mints keys and sends
the new pubkey + nonce → owner's client signs via the existing `compute_auth_tag`
and replies → spawner boots the container. Clients prompt on first handshake with
a given spawner pubkey and auto-sign thereafter; never auto-sign for an unknown
spawner.

**One Docker container per agent.** Agents run shell and file-edit tools through
`buzz-dev-mcp` — that's arbitrary code execution, so shared-host subprocesses
aren't acceptable. Env is assembled exactly as `spawn_agent_child` does today,
honoring the reserved-key strip list in `managed_agents/env_vars.rs:60`. LLM
credentials come from spawner-level env and never appear in any event.

**Personas resolve from `kind:30175`,** so a VPS Fizz is byte-identical to a local
one. One access gap to close: `30175` is author-only unless `["shared","true"]`,
and the spawner isn't the author — extend the read-gate using the same
owner-delegation shape the relay already uses for `24200` observer frames
(`ingest.rs:2029`), rather than inventing a second auth concept.

**Reconciliation** mirrors the desktop's `reconcile.rs`/`runtime/sweep.rs`: diff
specs against containers labelled `com.buzz.agent`, recreate on spec-hash drift
(reusing the `spawn_hash` idea), exponential backoff on crash loops with a cap,
and report failures via `30179` instead of retrying silently.

## Scope

- New: `crates/buzz-spawner/`, `docker/buzz-acp.Dockerfile`
- Modified: `buzz-core/src/kind.rs`, `buzz-relay` ingest gates + persona
  read-gate, `buzz-sdk/src/builders.rs`, `buzz-cli`
  (`spawner list|logs|restart`), desktop attestation responder,
  `deploy/compose/*`
- Requires `BUZZ_ALLOW_NIP_OA_AUTH=true` (currently defaults false,
  `buzz-relay/src/config.rs:172-184`)

## Open item for reviewers

Mounting `/var/run/docker.sock` into the spawner is root-equivalent on the host.
This needs to be documented loudly in `deploy/compose/README.md`, and it's worth
discussing whether a rootless-Docker or Podman-socket variant should be the
default recommendation.
