# External agent backend — run the harness yourself

Date: 2026-07-30
Status: approved, implementing

## Problem

Buzz supports agents only where it can spawn them. A user whose agent already
runs in a Docker container on their own VPS has no way to add it.

Hermes Agent is the concrete case. It exists today only as a **local** tier-2
preset (`desktop/src-tauri/src/managed_agents/discovery.rs:1595`,
`command: "hermes-acp"`) that the desktop spawns on this machine.

## Why "remote agent" does not mean "remote transport"

The relay connection lives in **`buzz-acp`** (the harness), not in the vendor
agent — `crates/buzz-acp/src/relay.rs:3825` `do_connect` dials out over
WebSocket. The agent leg is local stdio NDJSON JSON-RPC and nothing else:
`AcpClient` is concretely bound to `ChildStdin`/`ChildStdout`
(`crates/buzz-acp/src/acp.rs:139-176`, spawn at `:451`). There is no `Transport`
trait, no TCP, no SSH, no Docker in that path.

So the answer is **move the harness, not the transport**: run `buzz-acp` inside
the VPS container next to `hermes-acp`.

### Rejected: expose the agent's API over the network (e.g. Tailscale)

1. `hermes-acp` speaks stdio JSON-RPC. It has no network ACP listener, so this
   requires a transport refactor of `AcpClient`.
2. Tools would still execute in the container regardless. `buzz-acp` *declares*
   the `buzz` CLI as an MCP server in `session/new` and the **agent** spawns it
   (`crates/buzz-acp/src/lib.rs:4179-4232`), so the container needs `buzz`
   either way — zero savings, plus a WAN hop per JSON-RPC frame. The OpenClaw
   preset already documents this trap: Buzz-injected `BUZZ_*` env does not reach
   the execution locus when harness and agent are separated.
3. `buzz-acp` needs outbound-only egress. No inbound port, no ACL to maintain.

## What actually blocks the user: identity, not networking

The agent keypair is generated in the desktop (`commands/agents.rs:632`) and the
NIP-OA auth tag is signed by the **owner's** key (`:665-676`). That tag is what
makes the relay admit the agent — `crates/buzz-relay/src/api/mod.rs:82-105`
`MembershipDecision::ViaOwner`. A container cannot self-mint it.

## Design

A third backend kind, `BackendKind::External`, where Buzz:

- mints the agent identity (keypair + NIP-OA auth tag) — already backend-agnostic;
- publishes the kind:0 profile so the agent appears as a real community member
  before the container ever runs — `commands/agents.rs:990` already calls
  `sync_managed_agent_profile` unconditionally;
- hands the user a copy-pasteable env block containing the credentials.

Buzz spawns nothing, deploys nothing, owns no infra. Liveness comes from relay
presence (kind:20001), which the frontend already polls and renders as a
`PresenceDot`.

Generic across all harnesses. Hermes is the first caller, not a special case.

### The variant

`desktop/src-tauri/src/managed_agents/types.rs:6-13`. `Local` stays `#[default]`,
so every `#[serde(default)] backend` is unaffected.

```rust
pub enum BackendKind {
    #[default]
    Local,
    /// User runs `buzz-acp` themselves. Buzz mints identity + publishes profile;
    /// never spawns, deploys, stops, or reads logs. Liveness = relay presence.
    External,
    Provider { id: String, config: serde_json::Value },
}
```

Only **4 Rust sites** need edits. Every other `!= Local` / `== Local` guard is
already correct, because `External` is non-Local and every non-Local branch that
matters pattern-matches `Provider`.

| Site | Problem | Fix |
|---|---|---|
| `managed_agents/runtime.rs:149` | `backend_agent_id` is always `None`, so External reads `"not_deployed"` forever | `if` → 3-arm `match`; `External => ("external", None, "")` |
| `commands/agents.rs:1125-1132` | External falls into the `StartTarget::Provider` branch, `build_deploy_payload` *succeeds* (it never checks backend), then dies at `:1181` `"unsupported backend kind"` | real `match record.backend`, `External` arm errors early and builds no payload |
| `commands/agents.rs:1002-1005` | outer `!= Local` test now misleading | `matches!(…, Provider { .. })` |
| `commands/agents.rs:1030` | same — burns a store lock + 3 disk reads | `matches!(…, Provider { .. })` |

### Env-block generation

New `managed_agents/external_env.rs`, `AppHandle`-free so it is unit-testable.

`commands/agents_deploy.rs` `build_deploy_payload` is deliberately **not**
reused: it emits provider-protocol JSON (the provider binary does the env
translation), it builds `merged_env` from user layers only (`:70-79`) so it omits
the harness-definition env floor and runtime-metadata env vars, and its own doc
comment (`:44-48`) admits `agent_args` is pinned at create time.

Reused resolvers (all authoritative, none reimplemented):

| Need | Reuse |
|---|---|
| command / args / full layered env | `readiness.rs:125` `resolve_effective_harness_descriptor` |
| model / provider / prompt + orphan refusal | `resolve_effective_config(...).require_resolved()` |
| respond-to gate | `runtime.rs:380` `build_respond_to_env` |
| relay URL | `relay::effective_agent_relay_url` |
| session title | `runtime/metadata.rs:45` `resolve_session_title` |
| team instructions | `spawn_hash.rs:41` `effective_team_instructions` |
| fail closed on keyring outage | `storage.rs:199` `spawn_key_refusal` |

Emit order (later wins; `descriptor.env` last so user env wins, matching
`runtime.rs:857-859`): identity → `BUZZ_RELAY_URL` → harness command/args/mcp
(**bare** commands, not host absolute paths) → config → protocol defaults →
respond-to gate → `descriptor.env`.

Excluded: host-process concerns (`PATH`, `RUST_LOG`, `BUZZ_ACP_LAZY_POOL`), all
absolute `resolve_command()` paths, `GIT_CONFIG_*` and the
`git-credential-nostr` path, `BUZZ_ACP_SETUP_PAYLOAD`, the
`BUZZ_MANAGED_AGENT`/`_START_NONCE` desktop-ownership stamps (the orphan sweep
would try to reap it), `MCP_HOOK_SERVERS`, and
`HERMES_ACP_SKIP_CONFIGURED_MCP` — the harness applies that itself
(`crates/buzz-acp/src/config.rs:714` → `acp.rs:497`).

**Accepted debt:** two env assemblers can drift. Mitigated by a
`// mirror: external_env.rs` marker at the spawn site and a doc cross-reference —
the discipline `readiness.rs:25-39` already uses. Extracting a shared assembler
out of a 500-line function that also creates log files and spawns is a larger
risk than the drift. Revisit at a third consumer.

### Credential delivery

Reveal-once modal at create with a copy button, plus a re-reveal from the agent's
Manage dialog (the user will rebuild the container). No file written to disk by
Buzz.

One Tauri command, `get_external_agent_env(pubkey)`, serves both — rather than
adding a field to `CreateManagedAgentResponse`. One code path, re-reveal free.

The keyring read is `load_managed_agents` (it calls `hydrate_keys`,
`storage.rs:267-305`); no new keyring code.

### Security

Revealing an agent nsec to the frontend is the **already-established posture**:
`SecretRevealDialog.tsx:51-58` renders `created.privateKeyNsec` verbatim with a
`CopyButton` on every create, and `commands/identity.rs:190` `get_nsec` exports
the *owner's* nsec with no confirm and no re-auth. This change adds exactly one
thing: repeatability. Required guards:

- refuse unless `backend == External` — without it this is a generic nsec-export
  endpoint for local agents;
- `spawn_key_refusal` — never emit a block with an empty `BUZZ_PRIVATE_KEY`;
- **no `Debug` derive** on the response type (`CreateManagedAgentResponse`
  derives it at `types.rs:570` — a latent footgun; do not copy);
- never log the map or any value; errors name the pubkey only. There is no invoke
  middleware logging results, so the rule is prohibitive: this type must never
  reach `observer.emit`, `retain_*`, or `agent_event_content`;
- frontend copies `ProfileSettingsCard.tsx:97-137` `NsecRevealRow` including its
  late-resolve guard, and does **not** use `useQuery` — the secret must not sit
  in the query cache.

Env keys are POSIX-validated and reserved-filtered upstream
(`env_vars.rs:58-91`), so user env cannot inject `BUZZ_PRIVATE_KEY=` or a key
containing `=`/newline into an `--env-file`-shaped paste.

### `crates/buzz-acp`: zero changes

`BUZZ_AUTH_TAG` alone is sufficient. `resolve_agent_owner` (`lib.rs:117-143`)
verifies it **locally** via `verify_auth_tag` and extracts the owner hex — no
network, no owner discovery. `BUZZ_ACP_AGENT_OWNER` is only the legacy
`auth_tag: None` fallback.

## Out of scope

- **Provider-binary deploy** (`buzz-backend-ssh` / `-tailscale`). The existing
  `BackendKind::Provider` protocol (`managed_agents/backend.rs:19,361`) already
  supports this with zero Rust changes — a follow-up, not this change.
- Logs, stop, undeploy, or status queries against the container.

## Known follow-ups

- `build_deploy_payload` misses the harness-definition env floor and
  runtime-metadata env vars and pins `agent_args`, so provider-deploy and
  external will produce *different* effective envs for the same agent.
- Config edits don't reach a running container:
  `auto_restart_on_config_change: true` is set for every backend
  (`agents.rs:878`) while the restart machinery filters to Local. A "your env
  block changed — re-copy it" hint is the obvious next step.
- `set_managed_agent_start_on_app_launch` (`commands/agent_settings.rs:22-50`)
  has no backend guard; harmless today since both restore paths filter `== Local`.
- No exhaustive `match` on `BackendKind` anywhere, so adding a variant is
  compiler-invisible. Converting the `StartTarget` site to a real `match` gives
  the *next* variant one place to break the build.
