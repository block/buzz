# Per-owner Claude credential provisioning for server agents

Date: 2026-07-27
Status: Approved

## Problem

Server-hosted agents (buzz-spawner) currently receive one host-global set of
Anthropic credentials (`ANTHROPIC_API_KEY` / `CLAUDE_CODE_OAUTH_TOKEN` from the
spawner host's `.env` via `BUZZ_SPAWNER_AGENT_ENV` passthrough). Every user's
agents bill against the operator's key. Each user should instead provide their
own Claude OAuth token (or API key) via the desktop agents settings UI, and the
spawner should provision it into that user's agents.

## Decisions

- **Scope:** per-owner. One token per owner pubkey covers all server agents
  that owner owns on a given spawner.
- **Transport:** new NIP-44-encrypted frame variants on the existing kind:24201
  attestation channel (ephemeral, owner ↔ spawner). Credentials never appear in
  any public event, hash, or `PromptMaterial`.
- **UI placement:** a credential card in the Server Agents section of desktop
  agent settings, one per connected spawner.
- **Apply timing:** on token save/change, the spawner restarts that owner's
  running agents so the credential takes effect immediately.
- **No fallback (breaking):** server agents require an owner token. The
  host-global Anthropic credential passthrough no longer applies to
  Claude/Anthropic credentials. Agents without an owner token are held stopped
  and surfaced as `needs_credential`.
- **Token types:** OAuth token or API key, classified by prefix:
  `sk-ant-oat*` → `CLAUDE_CODE_OAUTH_TOKEN`, otherwise → `ANTHROPIC_API_KEY`.

## 1. Protocol (crates/buzz-sdk/src/spawner.rs)

- `AttestationFrame::CredentialUpdate { credential: String }` — owner → spawner.
  Sets/replaces the owner's token; empty string clears it.
- `AttestationFrame::CredentialAck { accepted: bool, message: Option<String> }`
  — spawner → owner delivery confirmation. Deliberately no hash echo: unlike
  prompt updates, credentials must never appear in any hash or public event.

## 2. Spawner (crates/buzz-spawner)

- **Storage:** new `credentials.json` in the state dir (0600, atomic write),
  map of owner pubkey → token. Separate from `agents.json` so agent records
  stay credential-free.
- **Env assembly (`env.rs`):** `build_agent_env` gains the owner credential and
  injects the prefix-classified env var after operator passthrough, so the
  owner token always wins over any host-global value.
- **No fallback:** if the owner has no stored token, the spawner does not start
  (or stops) that owner's agents and publishes kind:30179 status with a new
  `needs_credential: true` field.
- **On `CredentialUpdate`:** store the token, ack, then restart all running
  agents owned by that pubkey and start any held in `needs_credential`. On
  clear: stop the owner's agents and mark `needs_credential`.

## 3. Desktop

- **UI:** "Your Claude credential" card in `ServerAgentsSection.tsx`, one per
  connected spawner. Password-type input (OAuth token or API key), Save/Clear,
  status line ("Provisioned" after ack / "Not set — your server agents can't
  run"). Write-only: never echoed back, never persisted on the desktop — the
  spawner is the source of truth. Agents in `needs_credential` show a warning
  badge in the server agents list and edit dialogs.
- **Transport:** mirror the prompt-update path — new Tauri command
  `send_spawner_credential_update` (Rust builds/signs the kind:24201 event so
  the token never sits in JS-visible storage), published over the WebSocket via
  `spawnerRelay.ts`. No persistent queue: fire, await ack with a timeout, show
  an error toast on failure. A queued plaintext credential on disk is exactly
  what we don't want.

## 4. Testing

- Rust unit tests: frame round-trip encrypt/decrypt, prefix classification,
  env assembly with/without owner token, daemon restart-on-update and
  needs_credential behavior (existing daemon test patterns).
- Desktop: unit tests for the card's state machine; screenshot spec extending
  `server-agent-editing-screenshots.spec.ts` for the card and the
  `needs_credential` badge.

## Out of scope (YAGNI)

Non-Anthropic providers, per-agent overrides, token validation against
Anthropic's API, desktop keychain storage (nothing is persisted on desktop).
