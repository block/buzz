# Server-Hosted Agent Editing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** First-party editing of a server-hosted agent's prompt/model/provider from the desktop Edit agent dialog, pushed to the spawner via `AttestationFrame::PromptUpdate` with a queued/acked delivery and a server-aware dialog UI.

**Architecture:** The spawner advertises its AI catalog (providers + models) in its kind:10180 announcement and echoes an applied `prompt_hash` in kind:30179 status. The desktop detects server residency, scopes the Edit dialog's AI fields to the spawner's catalog, and on save sends an encrypted PromptUpdate frame (built/signed in Rust via a new Tauri command). A persisted latest-write-wins queue resends until the status hash acks.

**Tech Stack:** Rust (buzz-sdk, buzz-spawner, Tauri commands), TypeScript/React 19 (desktop), node `.test.mjs` unit tests, `cargo test`.

**Spec:** `docs/superpowers/specs/2026-07-27-server-agent-editing-design.md`

## Global Constraints

- No `unsafe`; no new `unwrap()`/`expect()` in production paths — use `?`.
- New public API needs doc comments.
- Desktop text sizes: rem tokens only.
- Run `just ci` before PR; `just fix-all` for formatting.
- Existing behavior already in place (do NOT reimplement): `AttestationFrame::PromptUpdate` exists in `crates/buzz-sdk/src/spawner.rs`; `apply_prompt_update` in `crates/buzz-spawner/src/daemon.rs` already validates ownership and forces a container restart by clearing `spec_hash`.
- Commit after every task; each task's tests must pass before commit.

---

### Task 1: SDK — AI catalog on announcement, prompt_hash on status, PromptMaterial::hash()

**Files:**
- Modify: `crates/buzz-sdk/src/spawner.rs` (structs `SpawnerAnnouncement`, `SpawnerAgentStatus`, `impl PromptMaterial`)
- Modify: `crates/buzz-sdk/Cargo.toml` (add `sha2` if not already a dependency)

**Interfaces:**
- Produces: `SpawnerAiProvider { id: String, models: Vec<String> }`; `SpawnerAnnouncement.ai: Option<Vec<SpawnerAiProvider>>`; `SpawnerAgentStatus.prompt_hash: Option<String>`; `PromptMaterial::hash(&self) -> String` (lowercase sha256 hex of the serde_json serialization).

- [ ] **Step 1: Write failing tests** in the existing `#[cfg(test)] mod tests` of `spawner.rs`:

```rust
#[test]
fn announcement_ai_catalog_round_trips() {
    let mut a = sample_announcement(); // reuse the struct literal from announcement_round_trips_through_an_event
    a.ai = Some(vec![SpawnerAiProvider {
        id: "anthropic".into(),
        models: vec!["claude-opus-5".into(), "claude-sonnet-5".into()],
    }]);
    let json = serde_json::to_string(&a).unwrap();
    let back: SpawnerAnnouncement = serde_json::from_str(&json).unwrap();
    assert_eq!(back.ai, a.ai);
    // Old announcements without the field still parse.
    let legacy: SpawnerAnnouncement = serde_json::from_str(
        r#"{"name":"x","max_agents":1,"agents_running":0}"#).unwrap();
    assert!(legacy.ai.is_none());
}

#[test]
fn prompt_material_hash_is_stable_and_content_sensitive() {
    let a = PromptMaterial { model: Some("m1".into()), ..Default::default() };
    let b = PromptMaterial { model: Some("m1".into()), ..Default::default() };
    let c = PromptMaterial { model: Some("m2".into()), ..Default::default() };
    assert_eq!(a.hash(), b.hash());
    assert_ne!(a.hash(), c.hash());
    assert_eq!(a.hash().len(), 64);
}

#[test]
fn status_prompt_hash_round_trips() {
    let s = SpawnerAgentStatus { phase: SpawnPhase::Running, agent_pubkey: None,
        spec_hash: None, error: None, restart_count: 0,
        prompt_hash: Some("ab".repeat(32)) };
    let back: SpawnerAgentStatus =
        serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back.prompt_hash, s.prompt_hash);
}
```

- [ ] **Step 2: Run** `cargo test -p buzz-sdk spawner` → FAIL (unknown field/method).
- [ ] **Step 3: Implement.** Add to `spawner.rs`:

```rust
/// One inference provider a spawner host can run, with its model ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnerAiProvider {
    /// Provider id, e.g. "anthropic".
    pub id: String,
    /// Model ids this host can run for the provider.
    #[serde(default)]
    pub models: Vec<String>,
}
```

On `SpawnerAnnouncement` add field (after `max_memory_mib`):

```rust
    /// Providers/models this host can run, so a client scopes its picker to
    /// what the server actually supports. Self-reported, like every field here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<Vec<SpawnerAiProvider>>,
```

On `SpawnerAgentStatus` add:

```rust
    /// Hash of the prompt material this agent is running with (see
    /// [`PromptMaterial::hash`]), so a client can tell whether a pushed
    /// prompt update has been applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
```

In `impl PromptMaterial`:

```rust
    /// Lowercase sha256 hex of this material's JSON serialization.
    ///
    /// Serialization skips `None` fields, so two materials with the same set
    /// values hash identically regardless of construction order.
    pub fn hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let json = serde_json::to_string(self).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(json.as_bytes());
        hex::encode(h.finalize())
    }
```

Fix every struct-literal construction of `SpawnerAnnouncement` / `SpawnerAgentStatus` across the workspace (`cargo build` will list them; expected sites: `crates/buzz-spawner/src/daemon.rs`, sdk tests, `owner_sim.rs`) by adding `ai: None` / `prompt_hash: None`.

- [ ] **Step 4: Run** `cargo test -p buzz-sdk && cargo build -p buzz-spawner` → PASS.
- [ ] **Step 5: Commit** `feat(sdk): spawner AI catalog, prompt_hash status field, PromptMaterial::hash`

---

### Task 2: Spawner — advertise AI catalog and echo prompt_hash in status

**Files:**
- Modify: `crates/buzz-spawner/src/config.rs` (new `ai_catalog` field parsed from `BUZZ_SPAWNER_AI_CATALOG`)
- Modify: `crates/buzz-spawner/src/daemon.rs` (`announce()`, `publish_status()` call sites)
- Modify: `.env.example` (document `BUZZ_SPAWNER_AI_CATALOG`)

**Interfaces:**
- Consumes: `SpawnerAiProvider`, `PromptMaterial::hash()` from Task 1.
- Produces: kind:10180 announcements carrying `ai`; kind:30179 statuses carrying `prompt_hash` whenever the store record has cached prompt material.

- [ ] **Step 1: Write failing tests** in `config.rs` tests:

```rust
#[test]
fn ai_catalog_parses_from_env_json() {
    let parsed = parse_ai_catalog(Some(
        r#"[{"id":"anthropic","models":["claude-opus-5"]}]"#.into()));
    let providers = parsed.unwrap();
    assert_eq!(providers[0].id, "anthropic");
    assert_eq!(providers[0].models, vec!["claude-opus-5"]);
    assert!(parse_ai_catalog(None).is_none());
    assert!(parse_ai_catalog(Some("not json".into())).is_none());
}
```

And in `daemon.rs` tests (follow the existing daemon test-harness pattern; if none covers `publish_status`, test at the point statuses are built): a record with `prompt = Some(material)` publishes a status whose `prompt_hash == material.hash()`; a record with `prompt = None` publishes `prompt_hash: None`.

- [ ] **Step 2: Run** `cargo test -p buzz-spawner` → FAIL.
- [ ] **Step 3: Implement.**
  - `config.rs`: add `pub ai_catalog: Option<Vec<buzz_sdk::spawner::SpawnerAiProvider>>`, populated by a new `fn parse_ai_catalog(raw: Option<String>) -> Option<Vec<SpawnerAiProvider>>` (logs a `warn!` and returns `None` on malformed JSON) fed with `non_empty_env("BUZZ_SPAWNER_AI_CATALOG")`.
  - `daemon.rs` `announce()`: set `ai: self.config.ai_catalog.clone()`.
  - `daemon.rs` `publish_status()`: add a `prompt_hash: Option<String>` parameter (or resolve it inside from `self.store.get(owner, slug).and_then(|r| r.prompt.as_ref()).map(|p| p.hash())` — prefer the internal lookup so call sites stay unchanged) and set it on the built `SpawnerAgentStatus`.
  - `.env.example`: `# BUZZ_SPAWNER_AI_CATALOG='[{"id":"anthropic","models":["claude-opus-5"]}]'`
- [ ] **Step 4: Run** `cargo test -p buzz-spawner` → PASS.
- [ ] **Step 5: Commit** `feat(spawner): advertise AI catalog, echo applied prompt_hash in status`

---

### Task 3: Tauri command `send_spawner_prompt_update`

**Files:**
- Modify: `desktop/src-tauri/src/commands/spawner.rs`
- Modify: `desktop/src-tauri/src/lib.rs` (register command in the invoke handler list next to `respond_to_spawner_attestation`)

**Interfaces:**
- Consumes: `AttestationFrame::PromptUpdate`, `PromptMaterial`, `build_spawner_attestation` (all already imported in this file).
- Produces: Tauri command `send_spawner_prompt_update(spawner_pubkey, spec_slug, agent_pubkey, prompt: PromptMaterial) -> SpawnerPromptUpdateOut { event_json: String, prompt_hash: String }`. The renderer publishes `event_json` over the WebSocket exactly like the attestation response.

- [ ] **Step 1: Write failing test** in the file's `mod tests` (mirror `the_signed_tag_verifies_for_the_requested_agent_only`'s setup helpers):

```rust
#[test]
fn prompt_update_round_trips_to_the_spawner() {
    let owner = Keys::generate();
    let spawner = Keys::generate();
    let prompt = PromptMaterial { model: Some("claude-opus-5".into()), ..Default::default() };
    let out = build_prompt_update_event(
        &owner, &spawner.public_key().to_hex(), "honey",
        &Keys::generate().public_key().to_hex(), prompt.clone()).unwrap();
    assert_eq!(out.prompt_hash, prompt.hash());
    // Spawner can decrypt and reads back the same frame.
    let event: nostr::Event = serde_json::from_str(&out.event_json).unwrap();
    let plain = nostr::nips::nip44::decrypt(
        spawner.secret_key(), &owner.public_key(), event.content.as_str()).unwrap();
    let frame: AttestationFrame = serde_json::from_str(&plain).unwrap();
    match frame {
        AttestationFrame::PromptUpdate { spec_slug, prompt: p, .. } => {
            assert_eq!(spec_slug, "honey");
            assert_eq!(p, prompt);
        }
        other => panic!("wrong frame: {other:?}"),
    }
}
```

- [ ] **Step 2: Run** `cargo test --manifest-path desktop/src-tauri/Cargo.toml spawner` → FAIL.
- [ ] **Step 3: Implement.** A pure helper plus a thin command (identity access copied from `respond_to_spawner_attestation`):

```rust
/// Output of [`send_spawner_prompt_update`].
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnerPromptUpdateOut {
    pub event_json: String,
    pub prompt_hash: String,
}

fn build_prompt_update_event(
    owner: &Keys,
    spawner_pubkey: &str,
    spec_slug: &str,
    agent_pubkey: &str,
    prompt: PromptMaterial,
) -> Result<SpawnerPromptUpdateOut, String> {
    let frame = AttestationFrame::PromptUpdate {
        spec_slug: spec_slug.to_string(),
        agent_pubkey: agent_pubkey.to_string(),
        prompt: prompt.clone(),
    };
    frame.validate().map_err(|e| e.to_string())?;
    let spawner_pk = PublicKey::parse(spawner_pubkey).map_err(|e| e.to_string())?;
    let plaintext = serde_json::to_string(&frame).map_err(|e| e.to_string())?;
    let ciphertext = nostr::nips::nip44::encrypt(
        owner.secret_key(), &spawner_pk, plaintext,
        nostr::nips::nip44::Version::V2,
    ).map_err(|e| e.to_string())?;
    let event = build_spawner_attestation(spawner_pubkey, &ciphertext)
        .map_err(|e| e.to_string())?
        .sign_with_keys(owner)
        .map_err(|e| e.to_string())?;
    Ok(SpawnerPromptUpdateOut {
        event_json: serde_json::to_string(&event).map_err(|e| e.to_string())?,
        prompt_hash: prompt.hash(),
    })
}

#[tauri::command]
pub async fn send_spawner_prompt_update(
    state: tauri::State<'_, AppState>, // match the state type respond_to_spawner_attestation uses
    spawner_pubkey: String,
    spec_slug: String,
    agent_pubkey: String,
    prompt: PromptMaterial,
) -> Result<SpawnerPromptUpdateOut, String> {
    let keys = /* same owner-keys resolution as respond_to_spawner_attestation */;
    build_prompt_update_event(&keys, &spawner_pubkey, &spec_slug, &agent_pubkey, prompt)
}
```

(Adjust encryption call to match exactly how `respond_to_spawner_attestation` encrypts — reuse its helper if one exists.) Register the command in `lib.rs`.

- [ ] **Step 4: Run** `cargo test --manifest-path desktop/src-tauri/Cargo.toml spawner` → PASS.
- [ ] **Step 5: Commit** `feat(desktop): Tauri command to build signed spawner prompt-update frames`

---

### Task 4: TS API layer — parse `ai`/`prompt_hash`, send prompt updates

**Files:**
- Modify: `desktop/src/shared/api/spawnerRelay.ts` (`SpawnerAnnouncement`, `parseSpawnerAnnouncement`, `SpawnerAgentStatus`, `parseSpawnerStatus`; new `sendSpawnerPromptUpdate`)
- Modify: `desktop/src/shared/api/tauriSpawner.ts` (wrapper for the new command)
- Test: extend the existing spawnerRelay parsing tests (`desktop/src/shared/api/*.test.mjs` pattern; create `spawnerRelayAi.test.mjs` beside it if parse tests live inline elsewhere)

**Interfaces:**
- Consumes: Tauri command `send_spawner_prompt_update` (Task 3); WS publish helper used by `respondToSpawnerAttestation`.
- Produces:
  - `type SpawnerAiProvider = { id: string; models: string[] }`
  - `SpawnerAnnouncement.ai?: SpawnerAiProvider[]`
  - `SpawnerAgentStatus.promptHash?: string | null`
  - `buildSpawnerPromptUpdate(input: { spawnerPubkey; specSlug; agentPubkey; prompt: SpawnerPromptMaterial }): Promise<{ event: RelayEvent; promptHash: string }>` in `tauriSpawner.ts`
  - `sendSpawnerPromptUpdate(same input): Promise<string /* promptHash */>` in `spawnerRelay.ts` (builds then publishes over the WS, mirroring `respondToSpawnerAttestation`)

- [ ] **Step 1: Write failing tests** for the two parsers:

```js
test("parseSpawnerAnnouncement surfaces the ai catalog", () => {
  const a = parseSpawnerAnnouncement(JSON.stringify({
    name: "vps", max_agents: 4, agents_running: 1,
    ai: [{ id: "anthropic", models: ["claude-opus-5"] }],
  }));
  assert.deepEqual(a.ai, [{ id: "anthropic", models: ["claude-opus-5"] }]);
});
test("parseSpawnerStatus surfaces prompt_hash", () => {
  const s = parseSpawnerStatus(JSON.stringify({ phase: "running", prompt_hash: "ab".repeat(32) }));
  assert.equal(s.promptHash, "ab".repeat(32));
});
```

- [ ] **Step 2: Run** `node --test desktop/src/shared/api/` → FAIL.
- [ ] **Step 3: Implement** the type/parse additions (malformed `ai` → `undefined`, matching the file's defensive style) and the two send helpers (`invokeTauri("send_spawner_prompt_update", …)`; publish via the same WS path `respondToSpawnerAttestation` uses).
- [ ] **Step 4: Run tests** → PASS. Also `cd desktop && pnpm exec biome check src/shared/api`.
- [ ] **Step 5: Commit** `feat(desktop): spawner AI catalog + prompt-hash parsing and prompt-update send path`

---

### Task 5: Pending prompt-update queue with status ack

**Files:**
- Create: `desktop/src/features/agents/spawnerPromptUpdateQueue.ts`
- Test: `desktop/src/features/agents/spawnerPromptUpdateQueue.test.mjs`
- Modify: `desktop/src/features/communities/useCommunityInit.ts` (add `resetSpawnerPromptUpdateQueue()` to `resetCommunityState()`)
- Modify: `desktop/src/features/agents/spawnerStatusStore.ts` (on ingest, call `ackSpawnerPromptUpdate(spawnerPubkey, slug, status.promptHash)`)

**Interfaces:**
- Consumes: `sendSpawnerPromptUpdate` (Task 4), `useSpawnerStatuses`/status ingest (existing), `SpawnerPromptMaterial` (existing).
- Produces:
  - `enqueueSpawnerPromptUpdate(input: { spawnerPubkey: string; specSlug: string; agentPubkey: string; prompt: SpawnerPromptMaterial }): Promise<void>` — persists (latest-write-wins per `spawnerPubkey:agentPubkey`), sends immediately, records `promptHash`.
  - `ackSpawnerPromptUpdate(spawnerPubkey: string, specSlug: string, promptHash: string | null | undefined): void` — clears a matching pending entry.
  - `retryPendingSpawnerPromptUpdates(): Promise<void>` — resends all pending; called when spawner status/announcement ingest shows the spawner alive.
  - `usePendingSpawnerPromptUpdate(agentPubkey: string): { pending: boolean; queuedAt: number } | null`
  - `resetSpawnerPromptUpdateQueue(): void`
  - Persistence: keyed in `localStorage` under `buzz:spawner-prompt-queue:<relayOrigin>` (JSON map), following how other per-relay UI state in `features/agents` persists — check `spawnerPreference.ts` first and reuse its storage helper if one exists.

- [ ] **Step 1: Write failing tests** for the pure core (export a reducer so tests avoid Tauri):

```js
import { queueReducer } from "../spawnerPromptUpdateQueue.ts"; // adjust to repo's .test.mjs import convention

test("enqueue is latest-write-wins per agent", () => {
  let s = queueReducer(new Map(), { type: "enqueue", key: "sp:ag", promptHash: "h1", queuedAt: 1 });
  s = queueReducer(s, { type: "enqueue", key: "sp:ag", promptHash: "h2", queuedAt: 2 });
  assert.equal(s.get("sp:ag").promptHash, "h2");
});
test("matching ack clears, stale ack does not", () => {
  let s = queueReducer(new Map(), { type: "enqueue", key: "sp:ag", promptHash: "h2", queuedAt: 2 });
  assert.equal(queueReducer(s, { type: "ack", key: "sp:ag", promptHash: "h1" }).size, 1);
  assert.equal(queueReducer(s, { type: "ack", key: "sp:ag", promptHash: "h2" }).size, 0);
});
```

- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** the module: pure `queueReducer` + a thin store (module map + subscribers, `useSyncExternalStore` hook, persistence load/save, send-with-catch — a failed send leaves the entry pending, no toast). Wire `resetSpawnerPromptUpdateQueue` into `resetCommunityState()` and the ack call into status ingest; call `retryPendingSpawnerPromptUpdates()` from announcement ingest (spawner seen ⇒ online).
- [ ] **Step 4: Run tests** → PASS.
- [ ] **Step 5: Commit** `feat(desktop): queued, status-acked spawner prompt updates`

---

### Task 6: Server-residency + dialog policy module

**Files:**
- Create: `desktop/src/features/agents/ui/serverAgentEditPolicy.ts`
- Test: `desktop/src/features/agents/ui/serverAgentEditPolicy.test.mjs`

**Interfaces:**
- Consumes: `SpawnerAiProvider` (Task 4).
- Produces:

```ts
export type ServerAgentEditContext = {
  spawnerPubkey: string;
  specSlug: string;
  agentPubkey: string;
  spawnerName: string;
};
/** Resolve whether the agent being edited lives on a spawner. */
export function resolveServerAgentEditContext(input: {
  relocatedToSpawner: string | null | undefined; // from the ManagedAgent, when editing one
  deployedSpawnerPubkey: string | null;          // from useServerAgents.isDeployed, when editing a persona
  agentPubkey: string | null;
  slug: string | null;
  spawnerNameFor: (pubkey: string) => string;
}): ServerAgentEditContext | null;
/** Model options for a server agent: the spawner's catalog, or null → free-text fallback. */
export function serverModelOptions(
  ai: SpawnerAiProvider[] | undefined,
  provider: string | null,
): { providers: string[]; models: string[] } | null;
```

- [ ] **Step 1: Write failing tests**: relocated agent resolves to a context; plain local agent resolves to null; `serverModelOptions` returns the selected provider's models, all providers' ids, and null when `ai` is undefined.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** (pure functions, no React).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(desktop): server-agent edit policy`

---

### Task 7: Server-aware Edit dialog UI

**Files:**
- Create: `desktop/src/features/agents/ui/ServerRunsOnBanner.tsx`
- Modify: `desktop/src/features/agents/ui/AgentDefinitionDialog.tsx`
- Modify: `desktop/src/features/agents/ui/AgentInstanceEditDialog.tsx`
- Modify: `desktop/src/features/agents/ui/UnifiedAgentsSection.tsx` (pending chip on the agent row)

**Interfaces:**
- Consumes: Task 6 policy, `useSpawnerDirectory` (existing), `usePendingSpawnerPromptUpdate` (Task 5), `spawnerLabel`/`runtimeLabel` from `ServerAgentsSection`.

- [ ] **Step 1: Build `ServerRunsOnBanner`** — a read-only row:

```tsx
export function ServerRunsOnBanner({ spawnerName, runtime, pending }: {
  spawnerName: string; runtime?: string | null; pending: boolean;
}) {
  return (
    <div className="flex items-center gap-2 rounded-md border border-input bg-muted/40 px-3 py-2 text-sm">
      <ServerIcon className="h-4 w-4 shrink-0 text-muted-foreground" aria-hidden />
      <span>
        Runs on <span className="font-medium">{spawnerName}</span> · Server
        {runtime ? <span className="text-muted-foreground"> · {runtime}</span> : null}
      </span>
      {pending ? (
        <span className="ml-auto rounded-full bg-amber-500/15 px-2 py-0.5 text-2xs font-medium text-amber-600">
          Update pending — server offline
        </span>
      ) : null}
    </div>
  );
}
```

(Icon: reuse whatever server glyph `UnifiedAgentsSection` shows next to relocated agents.)

- [ ] **Step 2: Wire into both dialogs.** Resolve the context via Task 6 at the top of each dialog. When non-null: render the banner above the name field; hide the harness (`AgentHarnessField`/runtime picker) section; source model/provider options from `serverModelOptions(directory.get(spawnerPubkey)?.ai, provider)` — when it returns null render the existing free-text input with helper text "Model list unavailable from this server"; under the AI section render helper text "Applied on the server. Saving restarts the agent." (existing `text-xs text-muted-foreground` style).
- [ ] **Step 3: Pending chip on rows.** In `UnifiedAgentsSection`, where `isRelocated` renders the server icon, add the amber "update pending" chip when `usePendingSpawnerPromptUpdate(agent.pubkey)` is non-null.
- [ ] **Step 4: Verify visually**: `just desktop-screenshot --name server-edit-dialog …` is not usable for spawner state; instead run `cd desktop && pnpm run build && pnpm exec biome check src && pnpm test` (unit level) and rely on Task 9's owner_sim run for live verification.
- [ ] **Step 5: Commit** `feat(desktop): server-aware Edit agent dialog`

---

### Task 8: Save path — push prompt updates on save

**Files:**
- Modify: `desktop/src/features/agents/useAgentManagement.ts` (`submitUpdate`)
- Modify: `desktop/src/features/agents/ui/AgentInstanceEditDialog.tsx` (its save handler, for relocated managed agents)

**Interfaces:**
- Consumes: `enqueueSpawnerPromptUpdate` (Task 5), `resolveServerAgentEditContext` (Task 6).

- [ ] **Step 1:** After a successful persona/instance save, when the edit context is non-null, call:

```ts
await enqueueSpawnerPromptUpdate({
  spawnerPubkey: ctx.spawnerPubkey,
  specSlug: ctx.specSlug,
  agentPubkey: ctx.agentPubkey,
  prompt: {
    system_prompt: input.systemPrompt || undefined,
    model: input.model || undefined,
    provider: input.provider || undefined,
  },
});
```

Never block or fail the save on this — the queue owns delivery; errors log via the file's existing logger.

- [ ] **Step 2:** Run `cd desktop && pnpm test && pnpm exec biome check src` → PASS.
- [ ] **Step 3: Commit** `feat(desktop): push prompt/model edits to the spawner on save`

---

### Task 9: End-to-end verification

**Files:**
- Modify: `crates/buzz-spawner/examples/owner_sim.rs` (extend with an edit → PromptUpdate → restart → status-hash assertion leg)

- [ ] **Step 1:** Extend `owner_sim` after its existing relocation flow: send a `PromptUpdate` with a new model, wait for the next kind:30179 status, assert `prompt_hash == material.hash()` and that the container restarted (phase cycles through provisioning back to running).
- [ ] **Step 2:** Run the sim against a local relay (`just relay` + spawner per the example's header docs) and confirm output.
- [ ] **Step 3:** Full gate: `just ci` (plus `just test` if relay crates were touched — they were not beyond buzz-spawner; run `cargo test -p buzz-spawner -p buzz-sdk`).
- [ ] **Step 4: Commit** `test(spawner): owner_sim exercises prompt-update push and ack`

---

## Self-review notes

- Spec §1 discovery → Tasks 1, 2, 4, 6. Spec §2 dialog → Tasks 6, 7. Spec §3 push+restart → Tasks 1, 2, 3, 4, 8 (restart already existed). Spec §4 queue+ack → Tasks 2, 4, 5, 7. Error handling → Task 5 Step 3 (silent pending, no toast on offline) — the spec's ">24h stale warning toast" is deliberately deferred to a follow-up; noted here so it isn't silently lost.
- Type names are consistent across tasks (`SpawnerAiProvider`, `promptHash`, `enqueueSpawnerPromptUpdate`).
- Implementers must mirror exact state/helper usage from `respond_to_spawner_attestation` (Task 3) rather than inventing new key-resolution code.
