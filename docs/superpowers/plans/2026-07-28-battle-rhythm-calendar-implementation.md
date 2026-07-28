# Battle Rhythm Calendar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a usable Battle Rhythm calendar to Command Adviser with manual events, reviewed FAS/Longcast/Shortcast imports, revision replacement, and one-way Apple Calendar publication.

**Architecture:** Store approved sources and calendar events as owner-authored NIP-33 events, with immutable revision events recording each applied import diff. Render those events through a dedicated React feature and route. Use a bounded native import pipeline for DOCX/XLSX/PDF extraction and the existing provider order for structured interpretation; extend the existing Swift EventKit helper for one-way publication.

**Tech Stack:** Rust, Nostr/Buzz signed events, Tauri 2, React 19, TypeScript, TanStack Router/Query, Swift/EventKit/PDFKit/Vision, Rust `zip`/`quick-xml`/`calamine`, Playwright.

## Global Constraints

- Battle Rhythm is a first-class sidebar destination at `/battle-rhythm`.
- Command Adviser is authoritative; Apple Calendar is a one-way mirror.
- Re-import replaces only entries owned by the selected source and coverage period.
- Manual events and other source-owned entries are never removed by re-import.
- Every import is previewed and explicitly approved before signed events are published.
- Import failure leaves the prior approved revision active.
- The internal calendar remains usable without Apple permission, RAG, Memory, or a model.
- Do not add a standalone service or database.
- Use rem-based Tailwind text tokens; do not add arbitrary text-size literals.
- Activate Hermit before every `just`, Cargo, commit, or push command.

---

## File Map

### Signed contracts and relay admission

- `crates/buzz-core/src/kind.rs` — canonical kind numbers.
- `crates/buzz-relay/src/handlers/ingest.rs` — admit the new global owner-authored kinds.
- `desktop/src/shared/constants/kinds.ts` — desktop mirrors.
- `mobile/lib/shared/relay/nostr_models.dart` — mobile mirrors.
- `crates/buzz-core/src/battle_rhythm.rs` — shared Rust source/event/revision contracts.
- `desktop/src/features/battle-rhythm/domain/contracts.ts` — strict TypeScript contracts.
- `desktop/src/features/battle-rhythm/domain/eventCodec.ts` — signed-event encoding and parsing.
- `desktop/src/features/battle-rhythm/data/battleRhythmService.ts` — relay queries and writes.

### Navigation and calendar UI

- `desktop/src/app/routes.ts` — `/battle-rhythm` route.
- `desktop/src/app/routes/battle-rhythm.tsx` — lazy route screen.
- `desktop/src/app/navigation/useAppNavigation.ts` — `goBattleRhythm`.
- `desktop/src/app/AppShell.helpers.ts` — selected view derivation.
- `desktop/src/app/AppShell.tsx` — sidebar callback.
- `desktop/src/features/sidebar/ui/AppSidebar.tsx` — view/callback props.
- `desktop/src/features/sidebar/ui/AppSidebarPinnedHeader.tsx` — visible Battle Rhythm item.
- `desktop/src/features/battle-rhythm/ui/BattleRhythmScreen.tsx` — feature shell.
- `desktop/src/features/battle-rhythm/ui/YearTimeline.tsx` — 12/24-month band view.
- `desktop/src/features/battle-rhythm/ui/MonthCalendar.tsx` — month view.
- `desktop/src/features/battle-rhythm/ui/WeekCalendar.tsx` — week view.
- `desktop/src/features/battle-rhythm/ui/DayShortcast.tsx` — Time/Event/I-C/Remarks view.
- `desktop/src/features/battle-rhythm/ui/EventEditorDialog.tsx` — manual event editor.
- `desktop/src/features/battle-rhythm/ui/SourceHistoryDialog.tsx` — import revision history and reviewed rollback.
- `desktop/src/features/battle-rhythm/domain/dateRange.ts` — date-window calculations.
- `desktop/src/features/battle-rhythm/hooks.ts` — React Query reads/mutations.

### Import and revision

- `desktop/src-tauri/Cargo.toml` — direct `quick-xml` and `calamine` dependencies.
- `desktop/src-tauri/src/command_services/planning_import.rs` — native picker, hashing, DOCX/XLSX extraction, bounded import response.
- `desktop/apple-inputs/Sources/PDFExtractor.swift` — PDFKit text extraction plus Vision OCR fallback.
- `desktop/apple-inputs/Sources/Protocol.swift` — bounded `extract_pdf` operation.
- `desktop/apple-inputs/Sources/main.swift` — operation dispatch.
- `desktop/apple-inputs/BuzzAppleInputs.xcodeproj/project.pbxproj` — compile the new Swift source.
- `desktop/src-tauri/src/commands/battle_rhythm.rs` — Tauri import and interpretation commands.
- `desktop/src-tauri/src/command_services/structured_completion.rs` — provider-neutral exact JSON completion.
- `desktop/src-tauri/src/command_services/mod.rs` — shared completion module export.
- `desktop/src-tauri/src/command_brief/cloud.rs` — reuse shared cloud transport.
- `desktop/src-tauri/src/command_brief/lmstudio.rs` — reuse shared local transport.
- `desktop/src-tauri/src/command_brief/orchestrator/providers.rs` — provider-order regression coverage.
- `desktop/src-tauri/src/commands/mod.rs` and `desktop/src-tauri/src/lib.rs` — command registration.
- `desktop/src/shared/api/tauriBattleRhythm.ts` — strict frontend command client.
- `desktop/src/features/battle-rhythm/domain/importDiff.ts` — identity matching and source-owned diff.
- `desktop/src/features/battle-rhythm/domain/deterministicChecks.ts` — structural/date checks.
- `desktop/src/features/battle-rhythm/ui/ImportReviewDialog.tsx` — source/revision selection, corrections, diff, approval.

### Apple publication

- `desktop/apple-inputs/Sources/EventKitWriter.swift` — dedicated calendar and event reconciliation.
- `desktop/apple-inputs/Sources/Protocol.swift` — `reconcile_calendar` request.
- `desktop/apple-inputs/Sources/main.swift` — writer dispatch.
- `desktop/src-tauri/src/command_services/apple_inputs.rs` — typed publication request/response.
- `desktop/src/shared/api/tauriAppleInputs.ts` — frontend publication types.
- `desktop/src/features/battle-rhythm/data/applePublication.ts` — stable projection and retry state.

### Verification

- Unit tests beside each domain/native module.
- `desktop/tests/e2e/battle-rhythm.spec.ts` — sidebar, views, manual event, revision review.
- `desktop/tests/e2e/battle-rhythm-screenshots.spec.ts` — distinct calendar views.
- `desktop/apple-inputs/Tests/EventKitWriterTests.swift` — one-way reconciliation.

---

### Task 1: Register and admit the Battle Rhythm event vocabulary

**Files:**
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-relay/src/handlers/ingest.rs`
- Modify: `desktop/src/shared/constants/kinds.ts`
- Modify: `mobile/lib/shared/relay/nostr_models.dart`
- Test: `crates/buzz-core/src/kind.rs`
- Test: `crates/buzz-relay/src/handlers/ingest.rs`

**Interfaces:**
- Produces: `KIND_BATTLE_RHYTHM_SOURCE = 30630`
- Produces: `KIND_BATTLE_RHYTHM_EVENT = 30631`
- Produces: `KIND_BATTLE_RHYTHM_REVISION = 46310`
- Consumes: existing NIP-33 global-event storage and `Scope::UsersWrite`

- [ ] **Step 1: Add failing kind-shape tests**

```rust
#[test]
fn battle_rhythm_kind_shapes_are_stable() {
    assert!(is_parameterized_replaceable(KIND_BATTLE_RHYTHM_SOURCE));
    assert!(is_parameterized_replaceable(KIND_BATTLE_RHYTHM_EVENT));
    assert!(!is_parameterized_replaceable(KIND_BATTLE_RHYTHM_REVISION));
    assert!(!is_ephemeral(KIND_BATTLE_RHYTHM_REVISION));
}
```

- [ ] **Step 2: Run the focused tests and verify unresolved constants fail**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core battle_rhythm_kind_shapes_are_stable
```

Expected: compile failure because the three constants do not exist.

- [ ] **Step 3: Define the constants and mirror them**

Add documented constants in `kind.rs`, identical numeric constants in the
desktop and mobile registries, and compile-time assertions for their kind
shapes.

- [ ] **Step 4: Add relay scope/global-kind tests**

```rust
#[test]
fn battle_rhythm_events_are_owner_global_user_writes() {
    for kind in [
        KIND_BATTLE_RHYTHM_SOURCE,
        KIND_BATTLE_RHYTHM_EVENT,
        KIND_BATTLE_RHYTHM_REVISION,
    ] {
        assert_eq!(required_scope_for_kind(kind, &make_dummy_event()).unwrap(), Scope::UsersWrite);
        assert!(is_global_only_kind(kind));
        assert!(!requires_h_channel_scope(kind));
    }
}
```

- [ ] **Step 5: Admit the three kinds**

Add all three kinds to `required_scope_for_kind` and `is_global_only_kind`.
Do not add them to `AUTHOR_ONLY_KINDS` or `P_GATED_KINDS`; Command Team agents
need bounded read access on the closed workspace relay.

- [ ] **Step 6: Run registry and relay tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core kind
cargo test -p buzz-relay battle_rhythm_events_are_owner_global_user_writes
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-core/src/kind.rs crates/buzz-relay/src/handlers/ingest.rs desktop/src/shared/constants/kinds.ts mobile/lib/shared/relay/nostr_models.dart
git commit -m "feat: register battle rhythm events"
```

### Task 2: Build strict calendar contracts and signed relay persistence

**Files:**
- Create: `desktop/src/features/battle-rhythm/domain/contracts.ts`
- Create: `desktop/src/features/battle-rhythm/domain/contracts.test.mjs`
- Create: `crates/buzz-core/src/battle_rhythm.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Create: `crates/buzz-core/tests/battle_rhythm_contracts.rs`
- Create: `desktop/src/features/battle-rhythm/domain/eventCodec.ts`
- Create: `desktop/src/features/battle-rhythm/domain/eventCodec.test.mjs`
- Create: `desktop/src/features/battle-rhythm/data/battleRhythmService.ts`
- Create: `desktop/src/features/battle-rhythm/data/battleRhythmService.test.mjs`
- Create: `desktop/src/features/battle-rhythm/hooks.ts`

**Interfaces:**
- Produces: `BattleRhythmSource`, `BattleRhythmEvent`, `BattleRhythmRevisionChunk`
- Produces: Rust `BattleRhythmSourceV1`, `BattleRhythmEventV1`, `BattleRhythmRevisionChunkV1`
- Produces: `fetchBattleRhythm(ownerPubkey, range)`
- Produces: `publishManualEvent(input)` and `applyImportRevision(approvedDiff)`
- Consumes: `signRelayEvent`, `relayClient.fetchEvents`, `relayClient.publishEvent`

- [ ] **Step 1: Write strict parser tests**

Cover exact keys, ISO-8601 timestamps, `start < end`, all-day values, bounded
text, valid source ownership, coverage order, and rejection of unknown fields.

```javascript
test("manual event parser rejects source revision ownership", () => {
  assert.throws(() =>
    parseBattleRhythmEvent({
      schemaVersion: 1,
      id: "event-1",
      ownership: { kind: "manual", sourceId: "fas" },
      title: "Sail",
      type: "passage",
      start: "2026-08-03T08:00:00+10:00",
      end: "2026-08-03T09:00:00+10:00",
      allDay: false,
      timeZone: "Australia/Sydney",
      status: "approved",
    }),
  );
});
```

- [ ] **Step 2: Run the parser test and verify the missing module fails**

Run:

```bash
cd desktop
pnpm test -- src/features/battle-rhythm/domain/contracts.test.mjs
```

Expected: FAIL because `contracts.ts` does not exist.

- [ ] **Step 3: Implement immutable contracts**

Use discriminated ownership:

```ts
export type EventOwnership =
  | { readonly kind: "manual" }
  | {
      readonly kind: "source";
      readonly sourceId: string;
      readonly revisionId: string;
      readonly sourceLocation: string;
    };
```

Freeze parsed arrays/objects and cap one import revision at 2,000 proposed
entries.

Implement the matching `serde` contracts in `buzz-core::battle_rhythm`. Add a
shared JSON fixture consumed by Rust and TypeScript tests so field names,
statuses, ownership, and bounds cannot drift.

- [ ] **Step 4: Write codec tests**

Assert:

- source/event `d` tags use stable IDs;
- `start`, `end`, `source`, and `revision` tags match parsed content;
- revision kind `46310` is immutable and contains added/changed/removed
  before/after values;
- large revisions split into ordered chunks whose serialized content is at
  most 240 KiB per event;
- every chunk carries one revision ID, chunk index/count, and full-manifest
  hash;
- a malformed event is ignored on read; and
- replacing an event uses `createdAt > prior.created_at`.

- [ ] **Step 5: Implement event codecs**

Expose:

```ts
export function buildSourceEvent(
  source: BattleRhythmSource,
  priorCreatedAt?: number,
): Promise<RelayEvent>;

export function buildCalendarEvent(
  event: BattleRhythmEvent,
  priorCreatedAt?: number,
): Promise<RelayEvent>;

export function buildRevisionEvents(
  revision: BattleRhythmRevision,
): Promise<readonly RelayEvent[]>;
```

- [ ] **Step 6: Write service tests for atomic ordering**

The service must publish replacement event heads first, every immutable
revision chunk second, and the source's active-revision pointer last. A missing
or hash-invalid chunk makes the new revision ineligible for activation. A
failure before the pointer write leaves the old revision selected.

- [ ] **Step 7: Implement relay service and React Query hooks**

`fetchBattleRhythm` must issue explicit kind filters and author filtering,
deduplicate by `(kind, d-tag)`, parse strictly, and return only events whose
range overlaps the requested window.

- [ ] **Step 8: Run desktop unit tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core --test battle_rhythm_contracts
cd desktop
pnpm test -- src/features/battle-rhythm
pnpm typecheck
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-core/src/battle_rhythm.rs crates/buzz-core/src/lib.rs crates/buzz-core/tests/battle_rhythm_contracts.rs desktop/src/features/battle-rhythm
git commit -m "feat: persist battle rhythm calendar data"
```

### Task 3: Add the sidebar destination and usable calendar views

**Files:**
- Modify: `desktop/src/app/routes.ts`
- Create: `desktop/src/app/routes/battle-rhythm.tsx`
- Modify: `desktop/src/app/navigation/useAppNavigation.ts`
- Modify: `desktop/src/app/navigation/useAppNavigation.test.mjs`
- Modify: `desktop/src/app/AppShell.helpers.ts`
- Modify: `desktop/src/app/AppShell.helpers.test.mjs`
- Modify: `desktop/src/app/AppShell.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebar.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebarPinnedHeader.tsx`
- Create: `desktop/src/features/battle-rhythm/domain/dateRange.ts`
- Create: `desktop/src/features/battle-rhythm/domain/dateRange.test.mjs`
- Create: `desktop/src/features/battle-rhythm/ui/BattleRhythmScreen.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/YearTimeline.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/MonthCalendar.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/WeekCalendar.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/DayShortcast.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/EventEditorDialog.tsx`
- Create: `desktop/src/features/battle-rhythm/ui/SourceHistoryDialog.tsx`
- Test: `desktop/tests/e2e/battle-rhythm.spec.ts`

**Interfaces:**
- Produces: routes `/battle-rhythm`
- Produces: sidebar `selectedView: "battleRhythm"`
- Consumes: Task 2 hooks and event mutation functions

- [ ] **Step 1: Write route derivation and navigation tests**

```javascript
test("battle rhythm route selects its own sidebar destination", () => {
  assert.deepEqual(deriveShellRoute("/battle-rhythm"), {
    selectedChannelId: null,
    selectedView: "battleRhythm",
  });
});
```

Also assert `goBattleRhythm()` builds `/battle-rhythm`.

- [ ] **Step 2: Run navigation tests and verify failure**

Run:

```bash
cd desktop
pnpm test -- src/app/AppShell.helpers.test.mjs src/app/navigation/useAppNavigation.test.mjs
```

Expected: FAIL because the route/view does not exist.

- [ ] **Step 3: Wire route, shell, and sidebar**

Add a `CalendarDays` icon and visible **Battle Rhythm** label to
`AppSidebarPrimaryMenu`. Do not wrap it in a preview `FeatureGate`.

- [ ] **Step 4: Write date-window tests**

Test:

- 12- and 24-month year ranges;
- Monday-start week boundaries;
- month leading/trailing cells;
- daylight-saving transitions in `Australia/Sydney`; and
- timed/all-day overlap inclusion.

- [ ] **Step 5: Implement date-window helpers**

Keep display calculations in one module and pass explicit time zone strings;
do not scatter `new Date()` window calculations across view components.

- [ ] **Step 6: Write the manual-event E2E test**

The test must:

1. open Battle Rhythm from the sidebar;
2. switch Year → Month → Week → Day;
3. create a manual timed event;
4. verify it appears in Week and Day;
5. edit it; and
6. add a weekly recurrence with one excluded occurrence;
7. verify the recurrence and exclusion in Week/Day; and
8. verify the source label remains **Manual**.

- [ ] **Step 7: Build the calendar screen and focused components**

Keep each view below the desktop file-size limit. Use:

- band rows for the Year view;
- a seven-column grid for Month;
- all-day header plus timed lanes for Week; and
- Time/Event/I-C/Remarks table for Day.

The event editor supports timed/all-day values, time zone, location,
responsible owner, participants, remarks, daily/weekly/monthly recurrence, and
per-occurrence exclusions. The Day view supports a separate routine-state
label such as Alongside, Cruising Watches, or Defence Watches.

The toolbar exposes Today, range navigation, view selector, New Event, Import
Document, Planning Review, filters, and Apple status.

- [ ] **Step 8: Extend the E2E bridge**

Add mock handlers for Battle Rhythm queries/publishes and deterministic fixture
events. Reset all Battle Rhythm module-level caches in
`resetCommunityState()`.

- [ ] **Step 9: Run UI checks**

Run:

```bash
cd desktop
pnpm test -- src/features/battle-rhythm src/app
pnpm check
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src/app desktop/src/features/sidebar desktop/src/features/battle-rhythm desktop/src/testing/e2eBridge.ts desktop/tests/e2e/battle-rhythm.spec.ts
git commit -m "feat: add Battle Rhythm calendar workspace"
```

### Task 4: Extract and review FAS, Longcast, and Shortcast documents

**Files:**
- Modify: `desktop/src-tauri/Cargo.toml`
- Create: `desktop/src-tauri/src/command_services/planning_import.rs`
- Create: `desktop/src-tauri/src/command_services/planning_import_tests.rs`
- Create: `desktop/apple-inputs/Sources/PDFExtractor.swift`
- Modify: `desktop/apple-inputs/Sources/Protocol.swift`
- Modify: `desktop/apple-inputs/Sources/main.swift`
- Modify: `desktop/apple-inputs/BuzzAppleInputs.xcodeproj/project.pbxproj`
- Create: `desktop/apple-inputs/Tests/PDFExtractorTests.swift`
- Create: `desktop/src-tauri/src/commands/battle_rhythm.rs`
- Create: `desktop/src-tauri/src/command_services/structured_completion.rs`
- Modify: `desktop/src-tauri/src/command_services/mod.rs`
- Modify: `desktop/src-tauri/src/command_brief/cloud.rs`
- Modify: `desktop/src-tauri/src/command_brief/lmstudio.rs`
- Modify: `desktop/src-tauri/src/command_brief/orchestrator/providers.rs`
- Modify: `desktop/src-tauri/src/commands/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src/shared/api/tauriBattleRhythm.ts`
- Create: `desktop/src/shared/api/tauriBattleRhythm.test.mjs`
- Create: `desktop/src/features/battle-rhythm/domain/importDiff.ts`
- Create: `desktop/src/features/battle-rhythm/domain/importDiff.test.mjs`
- Create: `desktop/src/features/battle-rhythm/domain/deterministicChecks.ts`
- Create: `desktop/src/features/battle-rhythm/domain/deterministicChecks.test.mjs`
- Create: `desktop/src/features/battle-rhythm/ui/ImportReviewDialog.tsx`

**Interfaces:**
- Produces: `pick_battle_rhythm_document() -> ExtractedPlanningDocument | null`
- Produces: `interpret_battle_rhythm_document(request) -> ImportProposal`
- Produces: `compareImportRevision(existing, proposal) -> ImportDiff`
- Consumes: Task 2 `applyImportRevision`

- [ ] **Step 1: Add bounded extraction fixtures and failing Rust tests**

Create synthetic DOCX and XLSX fixtures at test runtime. Assert:

- DOCX table rows retain cell text and row order;
- XLSX output retains sheet name, cell coordinates, values, and merged ranges;
- unsupported extensions are rejected;
- file size is capped at 50 MiB;
- output is capped at 4 MiB; and
- SHA-256, filename, and page/sheet metadata are present.

- [ ] **Step 2: Add direct parser dependencies**

Add direct dependencies for `quick-xml` and `calamine`; retain the existing
`zip` dependency. Use `cargo add` so lockfiles record the resolved compatible
versions.

- [ ] **Step 3: Implement native picker and DOCX/XLSX extraction**

The Tauri command opens a single-file dialog filtered to `docx`, `xlsx`, and
`pdf`, validates the selected regular file, hashes it, and returns bounded
structured blocks. It does not persist the document bytes.

- [ ] **Step 4: Write Swift PDF extraction tests**

Use a text PDF fixture and a rendered image-only fixture. Assert text PDF
extraction uses PDFKit and the image-only fixture returns Vision OCR text with
page numbers and bounded confidence.

- [ ] **Step 5: Implement `extract_pdf` in the existing helper**

Add a bounded request containing an absolute selected path and return page
records. The Rust parent still owns path validation, timeout, response bounds,
and cancellation.

- [ ] **Step 6: Write strict import-proposal parsing tests**

Define:

```ts
export type ImportProposal = {
  readonly schemaVersion: 1;
  readonly sourceType: "fas" | "longcast" | "shortcast";
  readonly proposedCoverage: { readonly start: string; readonly end: string };
  readonly events: readonly ProposedBattleRhythmEvent[];
  readonly uncertainties: readonly ImportUncertainty[];
};
```

Reject missing evidence locations, dates outside the proposed coverage,
unknown event types, more than 2,000 events, and model prose outside the JSON
contract.

- [ ] **Step 7: Add a provider-neutral structured interpretation seam**

Extract the common JSON-completion transport from the current LM Studio and
cloud adviser clients into a private
`command_services::structured_completion` module. Expose:

```rust
pub(crate) async fn complete_json(
    app: &AppHandle,
    system_prompt: &str,
    input: &serde_json::Value,
    schema_name: &str,
    cancellation: CancellationToken,
) -> Result<serde_json::Value, StructuredCompletionError>;
```

It must use the existing Cloud-first/Local-first provider order and must not
change Daily Command Brief behaviour. Add regression tests around the existing
provider ordering before switching the brief clients to the shared transport.

- [ ] **Step 8: Implement interpretation and fallback**

The structured prompt receives only bounded extracted blocks and asks for the
exact `ImportProposal` contract. If every model route is unavailable, return
the extracted table/cell blocks to a manual mapping screen; do not reject the
document or create events.

- [ ] **Step 9: Write diff tests**

Cover:

- unchanged identity match;
- changed time/title/owner;
- source-owned removal inside coverage;
- no removal outside coverage;
- manual event preservation;
- other-source preservation; and
- ambiguous identity requiring confirmation.

- [ ] **Step 10: Implement source-owned diff and deterministic checks**

Initial checks cover duplicate proposals, invalid time ranges, events outside
coverage, and conflicting matched FAS/Longcast/Shortcast dates. Missing
prerequisite rules are added in the integration plan.

- [ ] **Step 11: Build the Import Review dialog**

The dialog sequence is:

1. select type and New/Revision;
2. confirm source and coverage;
3. review extracted entries and uncertainties;
4. inspect Added/Changed/Removed/Unchanged;
5. resolve ambiguous matches; and
6. approve the selected diff.

Keep the dialog open on error and display the prior revision as still active.

Source History shows immutable revisions. Rollback previews the inverse of a
selected revision and publishes that inverse as a new approved revision; it
does not rewind or delete signed history.

- [ ] **Step 12: Run native and desktop tests**

Run:

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml planning_import
just apple-inputs-test
cd desktop
pnpm test -- src/shared/api/tauriBattleRhythm.test.mjs src/features/battle-rhythm/domain
pnpm typecheck
```

Expected: PASS.

- [ ] **Step 13: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src-tauri desktop/apple-inputs desktop/src/shared/api/tauriBattleRhythm.ts desktop/src/features/battle-rhythm
git commit -m "feat: import Battle Rhythm planning documents"
```

### Task 5: Publish approved entries one-way to Apple Calendar

**Files:**
- Create: `desktop/apple-inputs/Sources/EventKitWriter.swift`
- Create: `desktop/apple-inputs/Tests/EventKitWriterTests.swift`
- Modify: `desktop/apple-inputs/Sources/Protocol.swift`
- Modify: `desktop/apple-inputs/Sources/main.swift`
- Modify: `desktop/apple-inputs/BuzzAppleInputs.xcodeproj/project.pbxproj`
- Modify: `desktop/src-tauri/src/command_services/apple_inputs.rs`
- Modify: `desktop/src/shared/api/tauriAppleInputs.ts`
- Modify: `desktop/src/shared/api/tauriAppleInputs.test.mjs`
- Create: `desktop/src/features/battle-rhythm/data/applePublication.ts`
- Create: `desktop/src/features/battle-rhythm/data/applePublication.test.mjs`
- Modify: `desktop/src/features/battle-rhythm/ui/BattleRhythmScreen.tsx`

**Interfaces:**
- Produces: `reconcile_calendar`
- Produces: `publishBattleRhythmToApple(events) -> ApplePublicationStatus`
- Consumes: approved calendar events from Task 2

- [ ] **Step 1: Write writer fixture tests**

Use a fixture store abstraction to prove:

- the dedicated calendar is created once;
- stable external IDs update rather than duplicate;
- removed authoritative entries are deleted;
- unrelated calendars and untagged events are untouched;
- title/time changes overwrite Apple-side edits on reconciliation; and
- permission denial returns a degraded status without throwing away input.

- [ ] **Step 2: Add the bounded publication protocol**

Define:

```swift
struct CalendarProjection: Codable {
    let externalID: String
    let title: String
    let start: Date
    let end: Date
    let isAllDay: Bool
    let location: String?
    let notes: String?
}
```

Cap one request at 2,000 projections and reject duplicate `externalID` values.

- [ ] **Step 3: Implement EventKit reconciliation**

Create/select **HMAS Supply Battle Rhythm**, attach the stable external ID in
the event URL or structured notes marker, upsert exact projections, and delete
only previously managed IDs absent from the authoritative request.

- [ ] **Step 4: Extend Rust and TypeScript request unions**

Add `reconcile_calendar` without weakening the existing
`deny_unknown_fields`/exact-key parsing. Return counts for created, updated,
deleted, and unchanged entries plus permission/error status.

- [ ] **Step 5: Implement publication projection and retry state**

Project only approved calendar events. Store no independent Apple copy in
Battle Rhythm state; the stable external ID is `battle-rhythm:<event-id>`.

- [ ] **Step 6: Add publication UI behaviour**

Show:

- Published;
- Changes pending;
- Permission required;
- Retry publication; or
- Publication unavailable.

Calendar navigation and editing must remain enabled in every state.

- [ ] **Step 7: Run Apple and frontend tests**

Run:

```bash
. ./bin/activate-hermit
just apple-inputs-test
cargo test --manifest-path desktop/src-tauri/Cargo.toml apple_inputs
cd desktop
pnpm test -- src/shared/api/tauriAppleInputs.test.mjs src/features/battle-rhythm/data/applePublication.test.mjs
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
. ./bin/activate-hermit
git add desktop/apple-inputs desktop/src-tauri/src/command_services/apple_inputs.rs desktop/src/shared/api/tauriAppleInputs* desktop/src/features/battle-rhythm
git commit -m "feat: publish Battle Rhythm to Apple Calendar"
```

### Task 6: Prove the real Slice 1 user journey

**Files:**
- Create: `desktop/tests/e2e/battle-rhythm-screenshots.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Create: `docs/testing/battle-rhythm-live-acceptance.md`

**Interfaces:**
- Consumes: Tasks 1–5 complete
- Produces: verified usable calendar checkpoint

- [ ] **Step 1: Add E2E revision fixtures**

Seed:

- one manual event;
- one FAS source;
- a first approved source revision; and
- a second proposal containing one add, one change, one removal, and one
  unchanged entry.

- [ ] **Step 2: Test the complete revision workflow**

Assert the second import:

- shows the four diff categories;
- requires approval;
- removes only the prior-source omitted entry;
- keeps the manual event;
- creates no duplicate;
- exposes Apple publication status;
- rolls back to the first revision through a reviewed inverse diff; and
- retains both original revisions plus the rollback revision in history.

- [ ] **Step 3: Capture distinct scoped screenshots**

Capture separate Year, Month, Week, Day, and Import Review subjects with
`waitForAnimations`. Verify all hashes differ:

```bash
shasum -a 256 test-results/battle-rhythm/*.png
```

- [ ] **Step 4: Run the desktop gate**

Run:

```bash
. ./bin/activate-hermit
cd desktop
pnpm check
pnpm test
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts tests/e2e/battle-rhythm-screenshots.spec.ts --project=smoke
cd ..
cargo test --manifest-path desktop/src-tauri/Cargo.toml
just apple-inputs-test
```

Expected: every command PASS.

- [ ] **Step 5: Perform live macOS acceptance**

Using the signed application:

1. open Battle Rhythm from the sidebar;
2. create and edit a manual event;
3. import one supplied Shortcast;
4. inspect/correct the proposal;
5. approve it;
6. import a revised synthetic copy;
7. verify no duplicate and manual preservation;
8. grant Calendar permission;
9. verify the dedicated Apple Calendar contains only published Battle Rhythm
   entries; and
10. deny or revoke permission and verify the internal calendar still works.

Record exact build, document, result, and any bounded parser limitation in
`docs/testing/battle-rhythm-live-acceptance.md`.

- [ ] **Step 6: Run the repository gate**

Run:

```bash
. ./bin/activate-hermit
just ci
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
. ./bin/activate-hermit
git add desktop/tests/e2e desktop/playwright.config.ts docs/testing/battle-rhythm-live-acceptance.md
git commit -m "test: prove Battle Rhythm calendar journey"
```
