# Server-hosted agent editing — design

Date: 2026-07-27
Status: approved

## Problem

Once an agent is relocated to a spawner (kind:30178 + NIP-AS attestation), the
desktop's Edit agent dialog still presents it as a local agent: it offers the
local harness catalog and local model list, and saving edits only the persona.
The spawner caches the prompt material delivered at approval time and never
sees later edits — the protocol's `AttestationFrame::PromptUpdate` exists in
`buzz-sdk` and is handled by `buzz-spawner`, but no desktop code sends it.
Users cannot change a server agent's model, provider, or system prompt after
relocation, and the dialog gives no hint the agent runs on a server.

## Goals

- The Edit agent dialog makes server residency explicit and scopes its fields
  to the server's capabilities.
- Saving pushes prompt, model, and provider to the spawner first-party, with
  the change applied immediately (container restart, identity preserved).
- Available providers/models are discovered from the spawner, not assumed.
- Updates survive spawner downtime via a local queue with a status-based ack.

## Non-goals

- Changing the server agent's harness/runtime from the dialog.
- Moving an agent back to local or between spawners from this dialog.
- Any new relay-persisted event kinds (approach C was rejected).

## Design

### 1. Server model/provider discovery

The spawner adds an `ai` block to its kind:10180 announcement:

```json
{ "ai": { "providers": [ { "id": "anthropic", "models": ["claude-opus-5", "..."] } ] } }
```

Populated from the host's harness catalog and refreshed on spawner startup.
The desktop's `spawnerDirectoryStore` (which already ingests kind:10180)
parses and exposes it. If a spawner's announcement lacks the block, the Edit
dialog falls back to a free-text model input with a "model list unavailable
from this server" hint. It never shows the local model catalog for a
server-hosted agent.

### 2. Server-aware Edit dialog

The dialog determines server residency from: a managed agent whose
`relocatedToSpawner` is set, or a persona deployed to a spawner
(`useServerAgents.isDeployed`). When server-hosted:

- A read-only "Runs on: <spawner name> · Server" row renders near the top,
  reusing `spawnerLabel` / `runtimeLabel` from `ServerAgentsSection`.
- The AI configuration section sources provider/model options from the
  spawner's advertised `ai` block, with helper text
  "Applied on the server. Saving restarts the agent."
- The local harness picker is hidden — the server owns the harness.

### 3. Push path: PromptUpdate + restart

On save for a server-hosted agent, in addition to the normal persona write,
the desktop sends `AttestationFrame::PromptUpdate` (encrypted kind:24201)
carrying `system_prompt`, `team_instructions` (unchanged), `model`,
`provider`, and a `prompt_hash` (hash of the material, computed in Rust).

- A new Tauri command (mirroring `respond_to_spawner_attestation`) builds and
  signs the frame so key material and encryption stay in Rust.
- `buzz-spawner`'s `apply_prompt_update` is extended to mark the record as
  drifted; the reconciler restarts the container with the new prompt/model
  env. The agent keeps its key and identity.
- The spawner includes the applied `prompt_hash` in its kind:30179 status
  event for that agent — this is the acknowledgement.

### 4. Pending-update queue + ack

A persisted store (Tauri-store-backed, keyed `spawnerPubkey:agentPubkey`,
latest-write-wins) holds the most recent unacked update per agent.

- Sends immediately on save; resends when `spawnerStatusStore` shows the
  spawner online.
- Clears when a kind:30179 status arrives whose `prompt_hash` matches the
  queued update's hash.
- While pending, the agent row and Edit dialog show an
  "update pending — server offline" chip.
- The in-memory layer resets via `resetCommunityState()`; the persisted queue
  is scoped per relay so communities do not cross-contaminate.

## Error handling

- Spawner offline at save: persona saves normally; the update queues and the
  pending chip appears. No error toast — this is an expected state.
- Spawner rejects or never acks: the chip persists; hovering shows the last
  send attempt. A stale (>24h) pending update surfaces a warning toast on app
  start.
- Non-owner PromptUpdate frames are already rejected by the daemon
  (`ignoring prompt update from a non-owner`); unchanged.

## Testing

- `.test.mjs` unit tests: queue reducer (send/ack/supersede/reset), dialog
  server-mode policy (which fields show/hide, option sourcing, fallback).
- Rust tests in `buzz-spawner`: `apply_prompt_update` marks drift → reconcile
  restarts; status event carries `prompt_hash`; non-owner rejection.
- `owner_sim` example extended to exercise the full edit → push → restart →
  ack loop.
