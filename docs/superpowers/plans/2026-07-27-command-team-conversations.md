# Command Team Conversations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the six HMAS Supply advisers first-class Buzz agents whose normal conversations can produce bounded, encrypted outcomes that become optional cited evidence in later Daily Command Briefs.

**Architecture:** Extend Buzz's built-in persona catalogue and reuse its existing managed-agent, DM, channel-mention, NIP-AE engram, and command-brief source paths. A strict native collector reads only command-team engrams and converts eligible outcomes into ordinary `SourceKind::Memory` evidence; it never copies raw transcripts and never blocks or globally degrades a brief.

**Tech Stack:** Rust, Tauri 2, React 19, TypeScript, Nostr/NIP-AE, Playwright, Node test runner, Hermit, Cargo, pnpm.

## Global Constraints

- Keep the phase additive and small: no new chat service, database, replication layer, vector index, model route, or briefing orchestrator.
- Preserve current RAG, Apple inputs, LAN Memory MCP, Cloud/Local toggle, fallback order, signed brief publication, and scheduling.
- Use the existing managed-agent provisioning and reuse paths. Do not start six model processes at application launch.
- Keep full conversation history only in signed Buzz messages. Persist only structured substantive outcomes in existing encrypted NIP-AE engrams.
- Treat engram text as untrusted evidence. Validate it strictly before it becomes model-visible.
- Command-team discussion evidence is optional. Its absence or failure may add a concise warning but must not block or globally degrade the brief.
- Activate Hermit before every Cargo, `just`, hook, or Git claim:

  ```bash
  . ./bin/activate-hermit
  ```

- Add tests before production changes and observe each focused test fail for the intended reason.
- Use rem-based Tailwind text tokens only; do not introduce arbitrary text sizes.
- Do not add a module-level community-scoped cache. If one becomes necessary, add its reset to `resetCommunityState()`.

---

## Task 1: Add the Six Stable Command-Team Personas

**Files:**

- Modify: `desktop/src-tauri/src/managed_agents/personas.rs`
- Modify: `desktop/src-tauri/src/managed_agents/personas/tests.rs`
- Create: `desktop/src/features/command-console/domain/commandTeam.ts`
- Modify: `desktop/src/features/command-console/domain/briefContracts.ts`
- Modify: `desktop/src/features/command-console/ui/CommandTeamStrip.tsx`
- Modify: `desktop/src/features/command-console/ui/AdviserInsignia.tsx`
- Test: `desktop/src/features/command-console/ui/AdviserInsignia.test.mjs`

- [x] **Step 1: Add failing Rust catalogue tests**

Extend `personas/tests.rs` to require these definitions exactly once and active by default:

```rust
const COMMAND_TEAM_IDS: [&str; 6] = [
    "builtin:command-chief-of-staff",
    "builtin:command-operations",
    "builtin:command-navigation",
    "builtin:command-daily-routine",
    "builtin:command-reporting",
    "builtin:command-plans",
];
```

Assert each persona has:

- the expected display name;
- a symbolic `data:image/svg+xml` avatar;
- no forced runtime or model;
- a role-specific system prompt;
- the shared outcome-recording protocol;
- `default_active = true`.

Pin these prompt guarantees:

- navigation does not make navigational decisions or issue executable navigation orders;
- only substantive accepted outcomes are recorded;
- the agent uses the current context's channel and Buzz event identifiers;
- acknowledgement occurs only after `buzz mem` succeeds;
- raw transcript text is not copied into memory.

- [x] **Step 2: Run the persona test and confirm the intended failure**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas::tests
```

Expected: failures because the six command-team definitions do not yet exist.

- [x] **Step 3: Add one shared frontend command-team identity contract**

Create `commandTeam.ts` with a typed ordered mapping:

```ts
export interface CommandTeamPersona {
  adviser: AdviserId;
  personaId: string;
  label: string;
  detail: string;
}

export const COMMAND_TEAM_PERSONAS: readonly CommandTeamPersona[] = [
  {
    adviser: "chief_of_staff",
    personaId: "builtin:command-chief-of-staff",
    label: "Chief of Staff",
    detail: "Consolidation, challenge, priorities, and decisions",
  },
  // operations, navigation, daily-routine, reporting, plans
];

export function isCommandTeamPersonaId(id: string): boolean;
export function commandAdviserForPersona(id: string): AdviserId | undefined;
```

Use this mapping in `CommandTeamStrip` and `AdviserInsignia` so the console and My Agents refer to the same stable IDs and approved symbols.

- [x] **Step 4: Implement the built-in personas and shared memory protocol**

Add six `BuiltInPersona` entries. Use compact percent-encoded SVG data URLs for the symbolic avatars:

- Chief of Staff: anchor;
- Operations: radar plot;
- Navigation: sextant;
- Daily Routine: ship's bell;
- Reporting: clipboard/report;
- Plans: route/waypoints.

Each system prompt must contain its role boundary plus an identical recording protocol. The protocol must:

1. read channel, thread root, and triggering event ID from the ACP context;
2. skip greetings, filler, repeated information, and unaccepted exploration;
3. create the strict `command-discussion-outcome-v1` JSON contract;
4. derive `outcome_id` as lowercase SHA-256 over the UTF-8 string
   `<persona-id>\n<channel-id>\n<triggering-event-id>`;
5. write `mem/command-brief/<adviser>/<yyyy-mm-dd>/<outcome-id>` through `buzz mem set`;
6. say `Recorded for future briefs` only after a successful command;
7. report a failed write without suppressing the conversational answer;
8. update the same slug for corrections, use `buzz mem rm` for forgetting, and populate `supersedes` for later invalidations.

Keep `runtime` and `model` unset so the current Cloud/Local preference remains authoritative.

- [x] **Step 5: Run focused Rust and frontend identity tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas::tests
cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/command-console/ui/AdviserInsignia.test.mjs
```

- [x] **Step 6: Commit the persona contract**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri/src/managed_agents/personas.rs \
  desktop/src-tauri/src/managed_agents/personas/tests.rs \
  desktop/src/features/command-console/domain/commandTeam.ts \
  desktop/src/features/command-console/domain/briefContracts.ts \
  desktop/src/features/command-console/ui/CommandTeamStrip.tsx \
  desktop/src/features/command-console/ui/AdviserInsignia.tsx \
  desktop/src/features/command-console/ui/AdviserInsignia.test.mjs
git commit -m "feat(command-team): add built-in adviser personas"
```

---

## Task 2: Group the Advisers in My Agents and Reuse One Conversation Path

**Files:**

- Modify: `desktop/src/features/agents/ui/unifiedAgentGroups.ts`
- Create: `desktop/src/features/agents/ui/unifiedAgentGroups.test.mjs`
- Create: `desktop/src/features/agents/openPersonaConversation.ts`
- Create: `desktop/src/features/agents/openPersonaConversation.test.mjs`
- Create: `desktop/src/features/agents/usePersonaConversation.ts`
- Modify: `desktop/src/features/agents/ui/UnifiedAgentsSection.tsx`
- Modify: `desktop/src/features/agents/ui/AgentsView.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandTeamStrip.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs`

- [x] **Step 1: Add failing grouping and conversation tests**

Test that:

- the six definitions are rendered together under `Command Team`;
- each stable persona appears exactly once;
- existing non-command personas retain their existing grouping;
- an existing instance is preferred over creating a duplicate;
- an uninstantiated adviser is created using `buildInstanceInputForDefinition`;
- a stopped reusable instance is started;
- a running reusable instance is not restarted;
- `spawnError` prevents DM navigation and surfaces an error;
- successful provisioning/reuse opens a DM and navigates to it.

The pure operation should accept injected dependencies:

```ts
export interface OpenPersonaConversationDependencies {
  definitions: ManagedAgentPersonaRecord[];
  managedAgents: ManagedAgentRecord[];
  createAgent(input: CreateManagedAgentInput): Promise<CreateManagedAgentResult>;
  startAgent(pubkey: string): Promise<void>;
  openDm(pubkeys: string[]): Promise<Channel>;
  navigate(channelId: string): void;
  refetch(): Promise<void>;
}

export async function openPersonaConversation(
  personaId: string,
  dependencies: OpenPersonaConversationDependencies,
): Promise<void>;
```

- [x] **Step 2: Run the frontend tests and confirm they fail**

```bash
cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/agents/ui/unifiedAgentGroups.test.mjs \
  src/features/agents/openPersonaConversation.test.mjs \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs
```

Expected: missing module, grouping, and Message-action failures.

- [x] **Step 3: Implement Command Team grouping**

Change `buildUnifiedGroups` to emit a dedicated `Command Team` group based on `isCommandTeamPersonaId`. Preserve all existing agent/profile selection behavior. Do not clone persona records or generate parallel console-only definitions.

- [x] **Step 4: Implement the shared open-or-reuse operation**

In `openPersonaConversation.ts`:

1. resolve an active definition by stable persona ID;
2. call `findReusablePersonaAgent`/`pickPreferredManagedAgent`;
3. if absent, resolve the allowed runtime and call `buildInstanceInputForDefinition`;
4. create with `spawnAfterCreate: true`;
5. if present but not running/deployed, start it through the existing mutation;
6. call the existing DM mutation with the selected agent pubkey;
7. navigate with `goChannel`;
8. refetch managed-agent and relay data;
9. never create a second instance merely because the first is stopped.

Wrap the operation in `usePersonaConversation` to bind existing queries, mutations, navigation, per-persona pending state, and user-visible error handling.

- [x] **Step 5: Add Message actions to both existing surfaces**

- `UnifiedAgentsSection`: render `Message` for command-team persona cards and call the shared hook.
- `CommandTeamStrip`: render the same action using the same stable persona ID and hook.
- Disable only the selected persona's Message action while it is being opened.
- Keep the existing Start control and profile panel behavior intact.

Do not modify `useMentionSendFlow`: active built-in persona definitions are already provisioned/reused and attached when mentioned in a channel. Add a regression assertion instead of replacing that working path.

- [x] **Step 6: Run the focused frontend suite**

```bash
cd desktop
node --import ./test-loader.mjs --experimental-strip-types --test \
  src/features/agents/ui/unifiedAgentGroups.test.mjs \
  src/features/agents/openPersonaConversation.test.mjs \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs
```

- [x] **Step 7: Commit the reusable conversation path**

```bash
. ./bin/activate-hermit
git add desktop/src/features/agents \
  desktop/src/features/command-console/ui/CommandTeamStrip.tsx \
  desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs
git commit -m "feat(command-team): open reusable adviser conversations"
```

---

## Task 3: Parse and Select Strict Discussion Outcomes

**Files:**

- Modify: `desktop/src-tauri/src/commands/engrams.rs`
- Create: `desktop/src-tauri/src/command_brief/sources/command_team_discussions.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources.rs`

- [x] **Step 1: Add failing parser and selection tests**

In the new module, write table-driven unit tests for:

- a valid active outcome;
- unknown fields;
- wrong schema;
- uppercase/non-64-character outcome IDs;
- malformed RFC3339 timestamps;
- malformed channel UUID or event IDs;
- slug/body adviser, date, or outcome mismatch;
- writing persona claiming another adviser;
- unsupported status or brief section;
- oversized strings/arrays;
- duplicate/self `supersedes`;
- active outcomes retained regardless of age;
- closed outcomes retained for 90 days and then excluded;
- `superseded` status excluded;
- a valid newer outcome excluding each referenced predecessor;
- active-before-closed, newest-first ordering;
- at most six outcomes per adviser and 24 total;
- deterministic resolution of duplicates across instances.

Define the strict contract with `#[serde(deny_unknown_fields)]`:

```rust
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandDiscussionOutcome {
    schema: String,
    outcome_id: String,
    adviser: AdviserId,
    recorded_at: DateTime<Utc>,
    origin: OutcomeOrigin,
    status: OutcomeStatus,
    summary: String,
    decisions: Vec<String>,
    actions: Vec<OutcomeAction>,
    risks: Vec<String>,
    assumptions: Vec<String>,
    unresolved_questions: Vec<String>,
    brief_sections: Vec<BriefSection>,
    review_at: Option<DateTime<Utc>>,
    supersedes: Vec<String>,
}
```

- [x] **Step 2: Run the parser tests and confirm the intended failure**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_team_discussions
```

Expected: module/type/function failures before implementation.

- [x] **Step 3: Refactor the existing owner-gated engram reader**

Extract the body of the current Tauri command without changing its authorization or result shape:

```rust
pub(crate) async fn read_agent_memory_listing(
    agent_pubkey: &str,
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<AgentMemoryListing, String>;
```

The existing `get_agent_memory` command delegates to it. Preserve:

- managed-agent or verified NIP-OA ownership gate;
- decryption behavior;
- current-head selection by `d` tag;
- tombstone exclusion;
- existing 5,000-entry truncation behavior and listing metadata.

- [x] **Step 4: Implement strict parsing, validation, and bounded selection**

In `command_team_discussions.rs`, add:

```rust
pub(super) const COMMAND_TEAM_COLLECTION: &str = "command_team_discussions";

#[derive(Clone, Default)]
pub(super) struct CommandTeamDiscussionBatch {
    pub candidates: Vec<CandidateSource>,
    pub limitations: Vec<String>,
}

pub(super) async fn load_command_team_discussions(
    app: &tauri::AppHandle,
    observed_at: DateTime<Utc>,
) -> Result<CommandTeamDiscussionBatch, String>;
```

The loader must:

1. identify managed agents by the six authoritative persona IDs;
2. read each agent independently with `read_agent_memory_listing`;
3. treat per-agent failures as bounded limitations and continue;
4. inspect only `mem/command-brief/<adviser>/<yyyy-mm-dd>/<outcome-id>`;
5. parse strict JSON and validate bounded text/arrays, IDs, timestamps, enum values, and persona identity;
6. recompute the SHA-256 outcome ID from authoritative persona ID and the
   validated origin fields, rejecting a mismatch;
7. report truncated uncertainty only through a warning, not a global degraded section;
8. apply supersession, retention, ordering, and caps before creating candidates;
9. canonicalize the structured outcome with JCS for `quote`;
10. include persona ID, author pubkey, origin channel, thread root, and event IDs in `location`.

Use:

- engram event ID as `source_id` and `chunk_id`;
- outcome ID as `document_id`;
- `SourceKind::Memory`;
- `command_team_discussions` as `collection`;
- outcome `recorded_at` as source timestamp;
- collection time as `retrieved_at` and `observed_at`.

- [x] **Step 5: Run focused Rust tests**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_team_discussions
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands::engrams
```

- [x] **Step 6: Commit the outcome reader**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri/src/commands/engrams.rs \
  desktop/src-tauri/src/command_brief/sources.rs \
  desktop/src-tauri/src/command_brief/sources/command_team_discussions.rs
git commit -m "feat(command-team): read structured discussion outcomes"
```

---

## Task 4: Add Outcomes as Optional Daily Command Brief Evidence

**Files:**

- Modify: `desktop/src-tauri/src/command_brief/sources.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources/canonical.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/providers.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator.rs`

- [x] **Step 1: Add failing source-collector tests**

Extend `FakeBackend` and add tests proving:

- a valid command-team outcome appears in the canonical ledger as Memory evidence;
- its collection, persona, author, engram event, and Buzz origin are preserved;
- empty command-team evidence is a normal result;
- one adviser read failure emits one concise limitation while other advisers remain available;
- malformed and unrelated-agent entries never become model-visible;
- command-team omission at the 48-item ledger cap is reported but does not add a globally degraded Memory or adviser section;
- ordinary LAN Memory candidate omissions retain their current degradation behavior;
- cancellation still propagates.

- [x] **Step 2: Run source tests and confirm the intended failure**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::sources_tests
```

- [x] **Step 3: Load command-team evidence for both backend modes**

Extend the source abstraction:

```rust
pub(crate) trait SourceBackend: Send + Sync {
    fn command_team_discussions(&self) -> CommandTeamDiscussionBatch;
    // existing methods unchanged
}
```

Both `ProductionSourceBackendLoader` and `TrustedLanSourceBackendLoader` must receive an `AppHandle`, load the local batch, and store it in their backend. Pass `app.clone()` in `orchestrator.rs` so no ownership regression is introduced.

The `Arc<T>` forwarding implementation of `SourceBackend` must delegate the new
method. A top-level command-team load error becomes an empty batch plus one
bounded limitation; it must not make either backend loader fail.

This local evidence path must operate identically whether the current RAG source is production or trusted LAN.

- [x] **Step 4: Merge the optional candidates without global degradation**

In `SourceCollector::freeze_with_cancellation`:

1. append command-team candidates once;
2. append bounded command-team limitations;
3. continue with existing RAG, LAN Memory, and Apple collection;
4. canonicalize all accepted evidence through the existing ledger.

In `canonical.rs`, classify omission/rejection counts by both source kind and whether `collection == "command_team_discussions"`. Apply existing degradation only to non-command-team Memory evidence. For command-team losses, emit a concise optional-source limitation and leave `degraded_sections` unchanged.

Do not add a new `SourceKind`; downstream prompts and cloud/local routing already accept bounded Memory evidence.

- [x] **Step 5: Run focused and adjacent Rust suites**

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::sources_tests
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief::orchestrator
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief
```

- [x] **Step 6: Commit brief integration**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri/src/command_brief
git commit -m "feat(command-brief): cite adviser discussion outcomes"
```

---

## Task 5: Prove the Normal Buzz User Journey

**Files:**

- Modify: `desktop/src/testing/e2eBridge.ts`
- Create: `desktop/tests/e2e/command-team-conversations.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Modify: `desktop/tests/e2e/agents.spec.ts`
- Modify: `desktop/tests/e2e/command-console.spec.ts`

- [x] **Step 1: Add the six definitions to the mock bridge**

Update `resetMockPersonas` so the command-team definitions mirror native defaults:

- stable persona IDs and labels;
- symbolic avatars;
- active by default;
- no pre-created managed instances unless a test seeds one.

Keep existing Fizz, Honey, and Bumble behavior so unrelated test scenarios remain meaningful.

- [x] **Step 2: Add a failing focused Playwright journey**

The new spec must prove:

1. My Agents has a `Command Team` section containing each adviser once;
2. clicking Message for an uninstantiated adviser provisions and starts it, then opens its Buzz DM;
3. returning to the Command Console and clicking Message for the same adviser reuses its pubkey and DM;
4. mentioning that adviser in a normal channel attaches/reuses the same instance instead of creating another;
5. Message errors remain visible and do not navigate to an unrelated channel.

Use mock bridge inspection to assert instance count and stable pubkey, not only visible navigation.

- [x] **Step 3: Run the E2E spec and confirm the intended failure**

```bash
cd desktop
pnpm run build:e2e
pnpm exec playwright test tests/e2e/command-team-conversations.spec.ts --project=smoke
```

If port 4173 is already serving stale code, terminate that preview process, rebuild, and rerun.

- [x] **Step 4: Register the spec and make the focused journey pass**

Add the spec to the smoke project's `testMatch`. Update existing Agents and Command Console assertions only where the six default definitions intentionally change the visible catalogue.

Before any manual screenshot, use the shared animation wait. No PR screenshots are required unless the implementation materially differs from the already approved naval UI.

- [x] **Step 5: Run adjacent desktop tests**

```bash
cd desktop
pnpm run build:e2e
pnpm exec playwright test \
  tests/e2e/command-team-conversations.spec.ts \
  tests/e2e/agents.spec.ts \
  tests/e2e/command-console.spec.ts \
  --project=smoke
```

- [x] **Step 6: Commit E2E coverage**

```bash
. ./bin/activate-hermit
git add desktop/src/testing/e2eBridge.ts \
  desktop/tests/e2e/command-team-conversations.spec.ts \
  desktop/playwright.config.ts \
  desktop/tests/e2e/agents.spec.ts \
  desktop/tests/e2e/command-console.spec.ts
git commit -m "test(command-team): cover adviser conversation reuse"
```

---

## Task 6: Native Acceptance, Regression Gate, and Handoff

**Files:**

- Modify if evidence requires correction: files changed in Tasks 1-5 only
- Create: `docs/superpowers/evidence/2026-07-27-command-team-conversations-acceptance.md`

- [x] **Step 1: Run the full repository gate**

```bash
. ./bin/activate-hermit
just ci
```

If relay, database, or authorization code changed unexpectedly, also run:

```bash
. ./bin/activate-hermit
just test
```

- [ ] **Step 2: Run one controlled native conversation**

With the local relay and desktop app running through the repository's normal launch path:

1. open My Agents and confirm six Command Team definitions;
2. Message one adviser and confirm a single managed instance is provisioned;
3. send a substantive controlled prompt that establishes one harmless planning action;
4. inspect Agent Memory and verify exactly one encrypted `command-discussion-outcome-v1` head under the expected slug;
5. resend the same triggering event only through a controlled retry mechanism and verify the logical outcome remains one current head;
6. send a trivial greeting in a fresh exchange and verify no new command-brief outcome;
7. ask the adviser to correct, close, supersede, then forget controlled test outcomes and verify current-head behavior.

Use synthetic unclassified test content only.

- [ ] **Step 3: Generate a subsequent Daily Command Brief**

Generate one brief and verify:

- the controlled outcome can appear under Evidence;
- its citation identifies adviser, engram event, and Buzz channel/event origin;
- no raw transcript is in the source quote;
- disabling or breaking one synthetic adviser memory read yields a partial brief and concise warning;
- existing Apple, RAG, LAN Memory, and route indicators retain their previous behavior.

- [ ] **Step 4: Record reproducible acceptance evidence**

Create the evidence document with:

- commit SHA;
- exact focused/full commands and exit status;
- selected model route;
- adviser persona ID and managed-agent pubkey;
- test outcome slug and engram event ID;
- generated brief audit ID;
- evidence citation fields;
- any remaining user-visible limitation.

Do not include secret values, raw tokens, or sensitive transcript content.

- [ ] **Step 5: Review scope and repository cleanliness**

```bash
git diff --check
git status --short
git diff --stat origin/codex/command-adviser-naval-ui...HEAD
```

Confirm the diff contains no:

- new storage or replication service;
- route/fallback changes;
- transcript copying;
- adviser auto-start at launch;
- external actions;
- unrelated formatting churn.

- [ ] **Step 6: Save high-value project memory**

Using Memory MCP with `agent = "CODEX"`, record only:

1. the final command-team conversation architecture and stable persona IDs;
2. the strict outcome slug/schema, retention, bounds, and fail-soft collection behavior;
3. the live DM-to-engram-to-brief acceptance result and any durable gotcha.

- [ ] **Step 7: Commit and push the completed phase**

```bash
. ./bin/activate-hermit
git add docs/superpowers/evidence/2026-07-27-command-team-conversations-acceptance.md
git commit -m "docs(command-team): record conversation acceptance"
git push origin codex/command-team-conversations
```

- [ ] **Step 8: Update draft PR #11**

Update the draft PR body with:

- the six stable adviser personas and symbolic identities;
- reusable DM and mention behavior;
- the NIP-AE outcome contract and brief evidence path;
- fail-soft semantics;
- focused, E2E, native acceptance, and `just ci` evidence;
- explicit confirmation that routing, Apple, RAG, and LAN Memory behavior did not change.

Leave the PR draft until the user has exercised the feature in the real macOS app.

## Final Acceptance Checklist

- [ ] Six advisers appear exactly once under Command Team.
- [ ] My Agents and Command Console open the same reusable adviser instance and DM.
- [ ] Normal channel mentions reuse that instance.
- [ ] Substantive outcomes write one strict encrypted engram; trivial chat writes none.
- [ ] Retry, correction, close, supersede, and tombstone select the expected current evidence.
- [ ] The next brief cites bounded command-team evidence without raw transcript text.
- [ ] A broken adviser memory source warns but neither blocks nor globally degrades the brief.
- [ ] Existing Apple, RAG, LAN Memory, Cloud/Local routing, scheduling, cancellation, and signed publication remain green.
- [ ] Focused tests, desktop E2E, native acceptance, and `just ci` pass.
