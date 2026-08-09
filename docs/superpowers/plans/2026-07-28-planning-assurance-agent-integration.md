# Planning Assurance and Adviser Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Battle Rhythm and Plans useful to the Command Team by adding approval-gated planning assurance, reusable ship rules, bounded agent access/proposals, and Daily Command Brief integration.

**Architecture:** Run deterministic planning rules before a bounded RAG/Memory-grounded structured model review. Persist review findings and user-approved rules as signed NIP-33 events, but never convert findings into operational data without approval. Give agents read-only CLI queries and a strict proposal schema carried in their normal signed Buzz discussions; add bounded Battle Rhythm/Plans evidence to the existing brief source ledger.

**Tech Stack:** Rust, TypeScript, Nostr/Buzz signed events, `buzz-cli`, Tauri 2, existing RAG/Memory and Cloud-first/Local-first structured completion, React 19, Playwright.

## Global Constraints

- Deterministic checks work without a model, RAG, Memory, or internet.
- AI assurance uses the existing Cloud-first/Local-first route and exact structured output.
- Relevant doctrine guides advice when available; its absence does not prevent an assessment.
- Retrieved text is evidence, never instructions.
- Findings show evidence, rationale, confidence, and affected entities.
- Findings and agent proposals never create or change approved events/tasks without explicit user approval.
- Accepted corrections can become reusable rules only through an explicit Save Rule action.
- Dismissed findings reduce repeated noise but do not silently rewrite rules.
- Existing adviser conversations and Command Brief remain useful if Battle Rhythm or Plans is unavailable.
- No autonomous polling or unattended planning changes.
- Activate Hermit before every `just`, Cargo, commit, or push command.

---

## File Map

### Findings and rule engine

- `crates/buzz-core/src/kind.rs` — finding/rule kind numbers.
- `crates/buzz-core/src/planning_assurance.rs` — shared Rust finding/rule contracts.
- `crates/buzz-relay/src/handlers/ingest.rs` — global user-write admission.
- `desktop/src/shared/constants/kinds.ts` and mobile mirror — constants.
- `desktop/src/features/battle-rhythm/domain/assuranceContracts.ts` — finding/rule contracts.
- `desktop/src/features/battle-rhythm/domain/planningRules.ts` — deterministic evaluation.
- `desktop/src/features/battle-rhythm/data/assuranceService.ts` — signed finding/rule persistence.
- `desktop/src/features/battle-rhythm/ui/PlanningReviewPanel.tsx` — grouped review and apply/dismiss.
- `desktop/src/features/battle-rhythm/ui/PlanningRuleDialog.tsx` — explicit rule promotion.

### Knowledge-backed review

- `desktop/src-tauri/src/command_services/planning_assurance.rs` — evidence collection and model review.
- `desktop/src-tauri/src/commands/battle_rhythm.rs` — `run_planning_assurance`.
- `desktop/src/shared/api/tauriBattleRhythm.ts` — strict client.
- Existing `desktop/src-tauri/src/command_services/structured_completion.rs` — provider order from Calendar Slice 1.
- Existing RAG/Memory source clients under `desktop/src-tauri/src/command_brief/sources/`.

### Agent and brief integration

- `crates/buzz-cli/src/commands/planning.rs` — read-only planning commands.
- `crates/buzz-cli/src/lib.rs` — command tree.
- `crates/buzz-cli/src/client.rs` — exact event queries.
- `desktop/src-tauri/src/managed_agents/personas.rs` — adviser behaviour and proposal schema.
- `desktop/src-tauri/src/command_brief/sources/battle_rhythm.rs` — bounded signed-event evidence.
- `desktop/src-tauri/src/command_brief/sources.rs` and `types.rs` — source-kind registration.
- `desktop/src-tauri/src/command_brief/orchestrator/assembly.rs` — prompt/context inclusion.
- `desktop/src/features/command-console/domain/briefContracts.ts` — visible contract.
- `desktop/src/features/command-console/ui/DailyCommandBrief.tsx` and `briefPresentation.ts` — display.

---

### Task 1: Persist assurance findings and explicit ship planning rules

**Files:**
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-relay/src/handlers/ingest.rs`
- Modify: `desktop/src/shared/constants/kinds.ts`
- Modify: `mobile/lib/shared/relay/nostr_models.dart`
- Create: `crates/buzz-core/src/planning_assurance.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Create: `crates/buzz-core/tests/planning_assurance_contracts.rs`
- Create: `desktop/src/features/battle-rhythm/domain/assuranceContracts.ts`
- Create: `desktop/src/features/battle-rhythm/domain/assuranceContracts.test.mjs`
- Create: `desktop/src/features/battle-rhythm/data/assuranceService.ts`
- Create: `desktop/src/features/battle-rhythm/data/assuranceService.test.mjs`

**Interfaces:**
- Produces: `KIND_PLANNING_RULE = 30635`
- Produces: `KIND_PLANNING_FINDING = 30636`
- Produces: `PlanningRule`, `PlanningFinding`
- Produces: matching Rust `PlanningRuleV1`, `PlanningFindingV1`

- [ ] **Step 1: Add kind/scope tests and constants**

Both kinds are parameterized replaceable, owner-authored, global
`UsersWrite` events. Mirror them in desktop/mobile constants and assert their
NIP-33 shape.

- [ ] **Step 2: Write strict finding tests**

Define:

```ts
export type PlanningFindingCategory =
  | "sourceConflict"
  | "missingPrerequisite"
  | "suspiciousTiming"
  | "possibleOmission"
  | "unresolvedAmbiguity";

export type PlanningFindingStatus =
  | "open"
  | "accepted"
  | "dismissed"
  | "resolved";
```

Require bounded rationale, confidence from 0 to 1, evidence references,
affected entity IDs, source revision IDs, and an optional exact proposed
calendar/task/constraint change.

- [ ] **Step 3: Write strict rule tests**

A rule has:

- stable ID/name;
- enabled state;
- anchor event predicate;
- required event predicate;
- signed working-day or minute offset/window;
- optional applicability conditions;
- provenance;
- user-approved timestamp; and
- version.

Reject a rule that can match itself, has an unbounded window, or lacks an
approved timestamp.

- [ ] **Step 4: Implement parsers, event codecs, and service**

Use stable `d` tags. Fetch with explicit author/kind filters. A new run updates
the same logical finding when rule/source/entity identity matches instead of
creating duplicate warnings.

Implement matching bounded `serde` contracts in
`buzz-core::planning_assurance` and run shared JSON fixtures through both Rust
and TypeScript parsers.

- [ ] **Step 5: Run tests and commit**

```bash
. ./bin/activate-hermit
cargo test -p buzz-core planning
cargo test -p buzz-core --test planning_assurance_contracts
cargo test -p buzz-relay planning
cd desktop
pnpm test -- src/features/battle-rhythm/domain/assuranceContracts.test.mjs src/features/battle-rhythm/data/assuranceService.test.mjs
cd ..
git add crates/buzz-core/src/kind.rs crates/buzz-core/src/planning_assurance.rs crates/buzz-core/src/lib.rs crates/buzz-core/tests/planning_assurance_contracts.rs crates/buzz-relay/src/handlers/ingest.rs desktop/src/shared/constants/kinds.ts mobile/lib/shared/relay/nostr_models.dart desktop/src/features/battle-rhythm
git commit -m "feat: persist planning assurance findings"
```

### Task 2: Evaluate deterministic prerequisite and consistency rules

**Files:**
- Create: `desktop/src/features/battle-rhythm/domain/planningRules.ts`
- Create: `desktop/src/features/battle-rhythm/domain/planningRules.test.mjs`
- Modify: `desktop/src/features/battle-rhythm/domain/deterministicChecks.ts`
- Modify: `desktop/src/features/battle-rhythm/domain/deterministicChecks.test.mjs`
- Create: `desktop/src/features/battle-rhythm/ui/PlanningReviewPanel.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/PlanningReviewPanel.test.mjs`
- Create: `desktop/src/features/battle-rhythm/ui/PlanningRuleDialog.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/BattleRhythmScreen.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/ImportReviewDialog.tsx`

**Interfaces:**
- Produces: `evaluatePlanningRules(context) -> PlanningFinding[]`
- Consumes: live calendar events, source revisions, tasks, constraints, and approved rules

- [ ] **Step 1: Write the Monday-sailing fixture**

```javascript
test("requires Friday securing-for-sea activity before Monday sailing", () => {
  const findings = evaluatePlanningRules({
    events: [sailing("2026-08-03T08:00:00+10:00")],
    rules: [securingForSeaPriorWorkingDayRule()],
    workingCalendar: mondayToFriday(),
  });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].category, "missingPrerequisite");
  assert.equal(findings[0].proposedChange.start, "2026-07-31");
});
```

- [ ] **Step 2: Add benign controls**

Assert no finding when:

- the required event exists on Friday;
- the sailing is cancelled;
- the rule is disabled;
- the event is outside the rule's applicability; or
- the Friday is excluded and the required event exists on the calculated prior
  working day.

- [ ] **Step 3: Implement rule matching**

Normalize event titles only for predicate matching; never use normalized text
as an event identity. Calculate working-day offsets with the same working
calendar as the plan engine.

- [ ] **Step 4: Extend source-consistency checks**

Match FAS/Longcast/Shortcast activities by explicit linked identity first,
then bounded type/location/title/date similarity. Ambiguous matches produce
`unresolvedAmbiguity`, not an automatic conflict.

- [ ] **Step 5: Build Planning Review**

Group Open findings by category and severity. Each card shows:

- finding;
- evidence/source;
- rationale;
- confidence/basis;
- proposed change; and
- Apply, Dismiss, or Save as Rule actions.

Apply opens the normal Event/Task/Constraint editor prefilled with the proposal
and publishes only after its existing approval action.

- [ ] **Step 6: Run focused tests**

Run:

```bash
cd desktop
pnpm test -- src/features/battle-rhythm/domain/planningRules.test.mjs src/features/battle-rhythm/domain/deterministicChecks.test.mjs src/features/battle-rhythm/ui/PlanningReviewPanel.test.mjs
pnpm typecheck
```

Expected: PASS with zero findings in every benign control.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src/features/battle-rhythm
git commit -m "feat: detect Battle Rhythm planning gaps"
```

### Task 3: Add bounded doctrine/knowledge-backed AI assurance

**Files:**
- Create: `desktop/src-tauri/src/command_services/planning_assurance.rs`
- Create: `desktop/src-tauri/src/command_services/planning_assurance_tests.rs`
- Modify: `desktop/src-tauri/src/command_services/mod.rs`
- Modify: `desktop/src-tauri/src/commands/battle_rhythm.rs`
- Modify: `desktop/src/shared/api/tauriBattleRhythm.ts`
- Modify: `desktop/src/shared/api/tauriBattleRhythm.test.mjs`
- Modify: `desktop/src/features/battle-rhythm/ui/PlanningReviewPanel.tsx`

**Interfaces:**
- Produces: `run_planning_assurance(request) -> PlanningAssuranceResponse`
- Consumes: shared `complete_json`, trusted RAG/Memory clients, deterministic findings

- [ ] **Step 1: Write response-contract tests**

The native response contains:

```rust
pub struct PlanningAssuranceResponse {
    pub deterministic: Vec<PlanningFinding>,
    pub ai: Vec<PlanningFinding>,
    pub doctrine_status: EvidenceStatus,
    pub memory_status: EvidenceStatus,
    pub provider: Option<String>,
}
```

Reject more than 10 AI findings, unknown source IDs, confidence outside
`0.0..=1.0`, unbounded text, and proposed changes that fail the operational
contract parser.

- [ ] **Step 2: Add fixture evidence/backend tests**

Test:

- applicable doctrine evidence is passed as inert cited material;
- no doctrine still permits an assessment;
- RAG unavailable still returns deterministic findings;
- Memory unavailable does not block;
- malformed model output is discarded;
- retrieved prompt-injection text cannot change the schema/system prompt; and
- provider exhaustion returns deterministic findings only.

- [ ] **Step 3: Implement bounded evidence collection**

Build role-neutral queries from the affected activity, source type, location,
and finding category. Search `ADF Doctrine` first, then broader approved RAG
and relevant Memory. Cap total model-visible evidence and retain source IDs,
quoted locations, and retrieval time.

- [ ] **Step 4: Implement the exact model review**

The system prompt requires:

- facts versus assessment;
- use of only supplied source IDs;
- no direct actions;
- no invented doctrine;
- at most 10 findings; and
- exact `PlanningFinding[]` JSON.

Use the persistent provider preference and existing fallback order.

- [ ] **Step 5: Merge and deduplicate**

Deterministic findings win identity collisions. An AI finding may add rationale
or evidence but cannot lower the severity or hide a deterministic finding.

- [ ] **Step 6: Surface provider/evidence state**

Show **Rules only** when no model is available and show the selected provider
and evidence freshness inside the collapsed system/evidence area of Planning
Review.

- [ ] **Step 7: Run native and frontend tests**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml planning_assurance
cd desktop
pnpm test -- src/shared/api/tauriBattleRhythm.test.mjs src/features/battle-rhythm
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri/src/command_services desktop/src-tauri/src/commands/battle_rhythm.rs desktop/src/shared/api/tauriBattleRhythm* desktop/src/features/battle-rhythm
git commit -m "feat: add knowledge-backed planning assurance"
```

### Task 4: Give Command Team agents bounded planning access and proposals

**Files:**
- Create: `crates/buzz-cli/src/commands/planning.rs`
- Create: `crates/buzz-cli/src/commands/planning_tests.rs`
- Modify: `crates/buzz-cli/src/commands/mod.rs`
- Modify: `crates/buzz-cli/src/lib.rs`
- Modify: `crates/buzz-cli/src/client.rs`
- Modify: `desktop/src-tauri/src/managed_agents/personas.rs`
- Modify: `desktop/src-tauri/src/managed_agents/personas/tests.rs`
- Create: `desktop/src/features/battle-rhythm/domain/agentProposal.ts`
- Create: `desktop/src/features/battle-rhythm/domain/agentProposal.test.mjs`
- Modify: `desktop/src/features/battle-rhythm/ui/PlanningReviewPanel.tsx`

**Interfaces:**
- Produces: `buzz planning calendar list`
- Produces: `buzz planning plans list`
- Produces: `buzz planning plan show`
- Produces: `command-planning-proposal-v1`

- [ ] **Step 1: Write CLI parse/query tests**

Commands:

```text
buzz planning calendar list --owner <hex> --from <RFC3339> --to <RFC3339>
buzz planning plans list --owner <hex>
buzz planning plan show --owner <hex> --id <uuid>
```

Assert every relay query contains explicit `kinds`, `authors`, bounded limit,
and required time/project filters. No CLI subcommand publishes operational
calendar/task/constraint events.

- [ ] **Step 2: Implement compact read-only CLI output**

Return stable snake_case JSON suitable for model tools. Include source,
approval, task critical/float result, and open constraints where applicable.

- [ ] **Step 3: Define and test the proposal schema**

Normal signed adviser messages may include:

```json
{
  "schema": "command-planning-proposal-v1",
  "proposal_id": "uuid",
  "adviser": "operations",
  "rationale": "string",
  "evidence": ["source-id"],
  "changes": [
    {
      "kind": "calendar_event",
      "operation": "create",
      "value": {}
    }
  ]
}
```

The parser accepts only known Command Team advisers, bounded changes, exact
operational contracts, and evidence IDs visible in the discussion.

- [ ] **Step 4: Update persona guidance**

Operations coordinates programme changes; Plans handles task networks;
Navigation, Logistics, N2, Daily Routine, and Reporting query relevant
planning data. Agents:

- seek doctrine when relevant;
- still assess when doctrine is absent;
- distinguish facts/assumptions/assessment;
- preserve dissent; and
- use the proposal schema instead of claiming a change was applied.

N2 uses future locations and deployments from Battle Rhythm to focus existing
bounded World Monitor collection for the relevant region and horizon. This
does not add autonomous polling or increase the approved daily call pools.

- [ ] **Step 5: Add proposals to Planning Review**

Parse only signed messages from managed Command Team agents. Show proposal
origin and thread link. Apply routes through the existing editor and explicit
approval; Dismiss records local review state without changing the discussion.

- [ ] **Step 6: Run CLI/persona/frontend tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-cli planning
cargo test --manifest-path desktop/src-tauri/Cargo.toml managed_agents::personas
cd desktop
pnpm test -- src/features/battle-rhythm/domain/agentProposal.test.mjs
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-cli desktop/src-tauri/src/managed_agents/personas.rs desktop/src-tauri/src/managed_agents/personas/tests.rs desktop/src/features/battle-rhythm
git commit -m "feat: connect advisers to Battle Rhythm plans"
```

### Task 5: Add Battle Rhythm and Plans to the Daily Command Brief

**Files:**
- Create: `desktop/src-tauri/src/command_brief/sources/battle_rhythm.rs`
- Create: `desktop/src-tauri/src/command_brief/sources/battle_rhythm_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/sources.rs`
- Modify: `desktop/src-tauri/src/command_brief/types.rs`
- Modify: `desktop/src-tauri/src/command_brief/types_tests.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/assembly.rs`
- Modify: `desktop/src-tauri/src/command_brief/personas.rs`
- Modify: `desktop/src-tauri/src/command_brief/personas_tests.rs`
- Modify: `desktop/src/features/command-console/domain/briefContracts.ts`
- Modify: `desktop/src/features/command-console/domain/briefContracts.test.mjs`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs`
- Modify: `desktop/src/features/command-console/ui/briefPresentation.ts`

**Interfaces:**
- Produces: `SourceKind::BattleRhythm`, `SourceKind::Planning`
- Consumes: signed current event/project/task/constraint/finding heads

- [ ] **Step 1: Write source-collector tests**

For the brief's local date/time zone, collect bounded:

- today's programme and Shortcast;
- next seven days of briefs/reports/deadlines;
- changes since the prior brief;
- 30/60/90-day major activities;
- overdue or slipping critical tasks;
- upcoming task milestones;
- open mission constraints; and
- open source conflicts/missing prerequisites.

Exclude cancelled, superseded, malformed, out-of-window, and unapproved data.

- [ ] **Step 2: Implement signed-event collection**

Query exact kinds/authors and validate content using shared Rust planning
contracts. Convert each item to a `ValidatedSource` with source event ID,
logical entity ID, observation time, source revision, and concise quote.

- [ ] **Step 3: Add brief prompt/contract coverage**

Operations receives programme conflicts; Plans receives critical tasks and
constraints; Navigation/Logistics/N2/Reporting/Daily Routine receive bounded
role-relevant items. The Chief of Staff must expose unresolved critical
matters and cannot silently remove dissent.

- [ ] **Step 4: Update decision-first presentation**

Add compact visible blocks for:

- Today's Battle Rhythm;
- Critical path and milestones; and
- Mission constraints and planning conflicts.

Keep raw sources/provider state inside the collapsed Evidence and system
status area.

- [ ] **Step 5: Prove fail-soft behaviour**

Tests must show:

- Battle Rhythm query failure leaves other brief sections;
- schedule-calculation failure produces a visible limitation;
- one malformed task is excluded without discarding valid tasks; and
- rules-only assurance still appears when AI assurance is unavailable.

- [ ] **Step 6: Run brief tests**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml command_brief
cd desktop
pnpm test -- src/features/command-console
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri/src/command_brief desktop/src/features/command-console
git commit -m "feat: brief Battle Rhythm and plan risks"
```

### Task 6: Prove assurance quality and the end-to-end command workflow

**Files:**
- Create: `desktop/tests/e2e/planning-assurance.spec.ts`
- Create: `desktop/tests/e2e/planning-assurance-screenshots.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Create: `docs/testing/planning-assurance-live-acceptance.md`
- Modify: `docs/testing/battle-rhythm-live-acceptance.md`
- Modify: `docs/testing/plans-live-acceptance.md`

**Interfaces:**
- Consumes: Tasks 1–5 and both prerequisite implementation plans
- Produces: verified end-to-end Battle Rhythm/Plans checkpoint

- [ ] **Step 1: Build a fixed synthetic assurance corpus**

Include:

- FAS says Tuesday while Shortcast says Monday;
- Monday sailing without securing-for-sea activity;
- Monday sailing with the Friday activity;
- a cancelled sailing;
- an ambiguous same-name activity;
- a Gantt dependency cycle;
- a critical seaboat repair;
- a non-critical task with float;
- an open mission constraint outside the schedule path; and
- a benign routine week.

- [ ] **Step 2: Gate deterministic quality**

Require every seeded deterministic defect to be found and zero findings on the
benign controls. Fail the test on duplicate finding identity.

- [ ] **Step 3: Gate AI contract quality with a fixture provider**

The fixture returns:

- one valid doctrine-cited omission;
- one duplicate of a deterministic finding;
- one unknown citation;
- one malformed proposed event; and
- 12 total findings.

Assert only the valid bounded finding survives, the deterministic duplicate is
not duplicated, unknown/malformed findings are rejected, and the 10-finding
cap is enforced.

- [ ] **Step 4: Test user approval boundaries**

Verify:

- Review shows findings without changing data;
- Apply opens an editor;
- closing the editor changes nothing;
- approving the editor creates the signed event;
- Save Rule creates a signed rule only after confirmation; and
- the next run no longer repeats a dismissed identical finding.

- [ ] **Step 5: Test adviser-to-brief workflow**

Use a managed Operations adviser fixture to emit a valid signed planning
proposal. Approve it, generate a Daily Command Brief, and assert the activity,
critical task, and open constraint appear in their correct decision-first
sections with evidence.

- [ ] **Step 6: Capture distinct Planning Review screenshots**

Capture rule-only, AI-assisted, source-conflict, missing-prerequisite, and
mission-constraint states. Wait for animations and verify unique hashes.

- [ ] **Step 7: Run the complete automated gate**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core planning
cargo test -p buzz-cli planning
cargo test --manifest-path desktop/src-tauri/Cargo.toml
cd desktop
pnpm check
pnpm test
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts tests/e2e/plans.spec.ts tests/e2e/planning-assurance.spec.ts tests/e2e/planning-assurance-screenshots.spec.ts --project=smoke
cd ..
just apple-inputs-test
```

Expected: PASS.

- [ ] **Step 8: Perform live macOS acceptance**

Using the signed application:

1. import a local FAS/Longcast/Shortcast combination containing a known date
   disagreement;
2. verify the source conflict;
3. create a Monday sailing without Friday preparation;
4. verify the missing-prerequisite proposal;
5. run Cloud first and Local first assurance;
6. record useful findings and false positives for each;
7. approve one proposed event;
8. ask Operations about the deployment in a DM;
9. approve one valid agent proposal; and
10. generate a real Daily Command Brief containing the updated Battle Rhythm,
    critical path, and mission constraint.

Record exact providers, documents, findings, false positives, duration, and
visible degraded states in
`docs/testing/planning-assurance-live-acceptance.md`.

- [ ] **Step 9: Run the repository gate and commit**

```bash
. ./bin/activate-hermit
just ci
git add desktop docs/testing
git commit -m "test: prove planning assurance and adviser workflow"
```
