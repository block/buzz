# Plans and Critical Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a separate Plans workspace with deployment Gantt charts, working-day critical-path calculation, mission constraints, and linked due-date projections in Battle Rhythm.

**Architecture:** Store projects, tasks, and constraints as owner-authored NIP-33 events. Put the deterministic schedule engine in `buzz-core` so the desktop, CLI, and later brief integration use one implementation. Render a focused React Gantt workspace and derive read-only calendar/Apple milestones from approved task due dates.

**Tech Stack:** Rust, `chrono`, Nostr/Buzz signed events, Tauri 2, React 19, TypeScript, TanStack Router/Query, CSS grid/virtualization, Playwright.

## Global Constraints

- Plans is a first-class sidebar destination at `/plans`, separate from Battle Rhythm and the existing developer `/projects` feature.
- Plans and Battle Rhythm remain separate entities.
- A task due date appears at the top of its calendar day as a read-only linked milestone.
- Editing the task updates the milestone; the milestone cannot be edited independently.
- The critical path is calculated from dependencies, duration, working days, constraints, and the mission-ready milestone.
- Mission constraints remain visible independently of schedule float.
- A constraint is not resolved merely because an administrative checklist is complete.
- Full OPLIM and operational-risk workflows are not implemented in this plan.
- Do not add an external project-management service or database.
- Use rem-based Tailwind text tokens and keep UI files below repository size limits.
- Activate Hermit before every `just`, Cargo, commit, or push command.

---

## File Map

### Shared planning domain

- `crates/buzz-core/src/kind.rs` — planning kind numbers.
- `crates/buzz-core/src/lib.rs` — planning module export.
- `crates/buzz-core/src/planning.rs` — strict Rust project/task/constraint types and schedule engine.
- `crates/buzz-relay/src/handlers/ingest.rs` — admit planning events.
- `desktop/src/shared/constants/kinds.ts` and `mobile/lib/shared/relay/nostr_models.dart` — mirrors.
- `desktop/src/features/plans/domain/contracts.ts` — renderer contracts.
- `desktop/src/features/plans/domain/eventCodec.ts` — event conversion.
- `desktop/src/features/plans/data/plansService.ts` — relay persistence.
- `desktop/src-tauri/src/commands/plans.rs` — shared schedule command.
- `desktop/src/shared/api/tauriPlans.ts` — strict command client.

### Navigation and UI

- `desktop/src/app/routes.ts` — `/plans` and `/plans/$planId`.
- `desktop/src/app/routes/plans.tsx` and `plans.$planId.tsx` — lazy routes.
- `desktop/src/app/navigation/useAppNavigation.ts` — list/detail navigation.
- `desktop/src/app/AppShell.helpers.ts`, `AppShell.tsx` — selected view and callbacks.
- `desktop/src/features/sidebar/ui/AppSidebar.tsx` and `AppSidebarPinnedHeader.tsx` — visible Plans item.
- `desktop/src/features/plans/ui/PlansScreen.tsx` — project list.
- `desktop/src/features/plans/ui/PlanDetailScreen.tsx` — plan workspace.
- `desktop/src/features/plans/ui/GanttChart.tsx` — timeline renderer.
- `desktop/src/features/plans/ui/TaskTable.tsx` — WBS/task editor.
- `desktop/src/features/plans/ui/TaskEditorDialog.tsx` — task form.
- `desktop/src/features/plans/ui/MissionConstraintsPanel.tsx` — operational blockers.
- `desktop/src/features/plans/ui/ConstraintEditorDialog.tsx` — constraint form.
- `desktop/src/features/plans/hooks.ts` — queries/mutations.

### Battle Rhythm linkage and import

- `desktop/src/features/plans/domain/calendarProjection.ts` — task milestone projection.
- `desktop/src/features/battle-rhythm/hooks.ts` — merge projected milestones at read time.
- `desktop/src/features/battle-rhythm/ui/MonthCalendar.tsx`, `WeekCalendar.tsx`, and `DayShortcast.tsx` — render/open projections.
- `desktop/src/features/battle-rhythm/data/applePublication.ts` — publish task milestones.
- `desktop/src/features/plans/ui/PlanImportReviewDialog.tsx` — reviewed Gantt import.
- `desktop/src-tauri/src/commands/battle_rhythm.rs` — interpret deployment-plan documents.

---

### Task 1: Register planning events and strict project persistence

**Files:**
- Modify: `crates/buzz-core/src/kind.rs`
- Modify: `crates/buzz-relay/src/handlers/ingest.rs`
- Modify: `desktop/src/shared/constants/kinds.ts`
- Modify: `mobile/lib/shared/relay/nostr_models.dart`
- Create: `desktop/src/features/plans/domain/contracts.ts`
- Create: `desktop/src/features/plans/domain/contracts.test.mjs`
- Create: `desktop/src/features/plans/domain/eventCodec.ts`
- Create: `desktop/src/features/plans/domain/eventCodec.test.mjs`
- Create: `desktop/src/features/plans/data/plansService.ts`
- Create: `desktop/src/features/plans/data/plansService.test.mjs`
- Create: `desktop/src/features/plans/hooks.ts`
- Create: `crates/buzz-core/src/planning.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Create: `crates/buzz-core/tests/planning_contracts.rs`

**Interfaces:**
- Produces: `KIND_PLANNING_PROJECT = 30632`
- Produces: `KIND_PLANNING_TASK = 30633`
- Produces: `KIND_MISSION_CONSTRAINT = 30634`
- Produces: `PlanningProject`, `PlanningTask`, `MissionConstraint`
- Produces: matching Rust `PlanningProjectV1`, `PlanningTaskV1`, `MissionConstraintV1`

- [ ] **Step 1: Add failing kind and relay-scope tests**

Assert all three kinds are parameterized replaceable, global, accepted with
`UsersWrite`, and do not require an `h` tag.

- [ ] **Step 2: Define and mirror the constants**

Add documented constants and compile-time kind-shape assertions. Update relay
scope/global-kind lists and the desktop/mobile mirrors.

- [ ] **Step 3: Run kind and relay tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core planning
cargo test -p buzz-relay planning_events_are_owner_global_user_writes
```

Expected: PASS.

- [ ] **Step 4: Write strict contract tests**

Cover:

- project purpose, mission-ready date, status, owner, and linked activity IDs;
- task WBS, parent, owner, start, due date, duration, completion, and
  finish-to-start dependency IDs;
- constraint type, severity, owner, linked mission requirement/capability/task,
  status, required date, and disposition;
- parent/dependency references cannot equal the task ID;
- percentage complete is an integer from 0 to 100; and
- unknown fields and invalid status values are rejected.

- [ ] **Step 5: Implement immutable renderer contracts**

Use these closed statuses:

```ts
export type ProjectStatus = "draft" | "active" | "complete" | "cancelled";
export type TaskStatus = "notStarted" | "inProgress" | "blocked" | "complete" | "cancelled";
export type ConstraintStatus =
  | "open"
  | "mitigated"
  | "resolved"
  | "missionChanged"
  | "oplimCandidate"
  | "riskCandidate";
```

Implement matching `serde` contracts in `buzz-core::planning`. Use one shared
JSON fixture in Rust and TypeScript tests to lock field names, status
vocabularies, and bounds.

- [ ] **Step 6: Write event-codec and service tests**

Assert stable `d` tags, `project` and `due` tags, monotonic replacement
timestamps, explicit author/kind queries, malformed event rejection, and
project deletion refusing to orphan live tasks or constraints.

- [ ] **Step 7: Implement codecs, service, and hooks**

Publish project/task/constraint changes individually as signed NIP-33 heads.
The service returns a consistent project aggregate by validating every task
and constraint against a live project ID.

- [ ] **Step 8: Run desktop domain tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core --test planning_contracts
cd desktop
pnpm test -- src/features/plans
pnpm typecheck
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-core/src/kind.rs crates/buzz-core/src/planning.rs crates/buzz-core/src/lib.rs crates/buzz-core/tests/planning_contracts.rs crates/buzz-relay/src/handlers/ingest.rs desktop/src/shared/constants/kinds.ts mobile/lib/shared/relay/nostr_models.dart desktop/src/features/plans
git commit -m "feat: persist Command Adviser plans"
```

### Task 2: Implement one working-day critical-path engine

**Files:**
- Modify: `crates/buzz-core/src/planning.rs`
- Modify: `crates/buzz-core/src/lib.rs`
- Create: `crates/buzz-core/tests/planning_schedule.rs`
- Create: `desktop/src-tauri/src/commands/plans.rs`
- Modify: `desktop/src-tauri/src/commands/mod.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Create: `desktop/src/shared/api/tauriPlans.ts`
- Create: `desktop/src/shared/api/tauriPlans.test.mjs`

**Interfaces:**
- Produces: `calculate_schedule(input: &PlanningScheduleInput) -> Result<PlanningSchedule, PlanningScheduleError>`
- Produces: Tauri `calculate_plan_schedule`
- Consumes: Task 1 project/task JSON

- [ ] **Step 1: Write a known-network failing test**

```rust
#[test]
fn calculates_critical_path_and_float() {
    // A(2d) -> B(3d) -> D(1d) = 6d
    // A(2d) -> C(1d) -> D(1d) = 4d, so C has 2d float.
    let schedule = calculate_schedule(&fixture_network()).unwrap();
    assert_eq!(schedule.project_duration_workdays, 6);
    assert_eq!(schedule.task("A").unwrap().total_float_workdays, 0);
    assert_eq!(schedule.task("B").unwrap().total_float_workdays, 0);
    assert_eq!(schedule.task("C").unwrap().total_float_workdays, 2);
    assert_eq!(schedule.task("D").unwrap().total_float_workdays, 0);
}
```

- [ ] **Step 2: Run the focused test and verify failure**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core --test planning_schedule
```

Expected: compile failure because the planning module does not exist.

- [ ] **Step 3: Implement graph validation**

Reject:

- duplicate task IDs;
- missing dependency IDs;
- self-dependencies;
- cycles, returning the bounded cycle IDs;
- zero/negative duration for incomplete leaf tasks;
- summary tasks used as dependencies; and
- mission-ready dates earlier than a fixed task constraint.

- [ ] **Step 4: Implement working-day arithmetic**

The input contains:

```rust
pub struct WorkingCalendar {
    pub working_weekdays: BTreeSet<Weekday>,
    pub excluded_dates: BTreeSet<NaiveDate>,
}
```

Default to Monday–Friday. Include excluded dates in every forward/backward
calculation.

- [ ] **Step 5: Implement forward/backward passes**

Return earliest start/finish, latest start/finish, total float, critical flag,
and project duration for each leaf task. Summary dates are derived from their
descendants and are never placed directly on the critical path.

- [ ] **Step 6: Add edge-case tests**

Test:

- non-working weekend crossing;
- excluded holiday;
- fixed start date;
- completed tasks using actual completion;
- overdue unfinished tasks;
- multiple critical paths;
- disconnected task; and
- incomplete task data returning a structured error.

- [ ] **Step 7: Expose and strictly parse the Tauri command**

The command accepts the exact input contract and returns camelCase schedule
output. The TypeScript client rejects unknown/missing fields and non-finite
float values.

- [ ] **Step 8: Run Rust and API tests**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core planning
cargo test --manifest-path desktop/src-tauri/Cargo.toml plans
cd desktop
pnpm test -- src/shared/api/tauriPlans.test.mjs
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
. ./bin/activate-hermit
git add crates/buzz-core/src/planning.rs crates/buzz-core/src/lib.rs crates/buzz-core/tests/planning_schedule.rs desktop/src-tauri/src/commands/plans.rs desktop/src-tauri/src/commands/mod.rs desktop/src-tauri/src/lib.rs desktop/src/shared/api/tauriPlans*
git commit -m "feat: calculate plan critical paths"
```

### Task 3: Add the separate Plans routes and editable Gantt workspace

**Files:**
- Modify: `desktop/src/app/routes.ts`
- Create: `desktop/src/app/routes/plans.tsx`
- Create: `desktop/src/app/routes/plans.$planId.tsx`
- Modify: `desktop/src/app/navigation/useAppNavigation.ts`
- Modify: `desktop/src/app/navigation/useAppNavigation.test.mjs`
- Modify: `desktop/src/app/AppShell.helpers.ts`
- Modify: `desktop/src/app/AppShell.helpers.test.mjs`
- Modify: `desktop/src/app/AppShell.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebar.tsx`
- Modify: `desktop/src/features/sidebar/ui/AppSidebarPinnedHeader.tsx`
- Create: `desktop/src/features/plans/ui/PlansScreen.tsx`
- Create: `desktop/src/features/plans/ui/PlanDetailScreen.tsx`
- Create: `desktop/src/features/plans/ui/GanttChart.tsx`
- Create: `desktop/src/features/plans/ui/TaskTable.tsx`
- Create: `desktop/src/features/plans/ui/TaskEditorDialog.tsx`
- Test: `desktop/tests/e2e/plans.spec.ts`

**Interfaces:**
- Produces: `/plans`, `/plans/$planId`
- Produces: sidebar `selectedView: "plans"`
- Consumes: Tasks 1–2 hooks and `calculatePlanSchedule`

- [ ] **Step 1: Write route and sidebar selection tests**

Assert `/plans` and `/plans/<id>` select `"plans"` and do not select the
existing `"projects"` developer view.

- [ ] **Step 2: Wire the route and visible menu item**

Use the installed Lucide `ChartGantt` icon, label it **Plans**, and do not
place it behind a preview gate.

- [ ] **Step 3: Write a Gantt E2E test**

Create a project and the A/B/C/D network from Task 2. Assert:

- WBS rows and bars align;
- A/B/D are marked critical;
- C shows two working days of float;
- changing B from three to four days recalculates the mission-ready date; and
- reopening the plan retains the signed task data.

- [ ] **Step 4: Implement project list and detail screen**

The list shows mission-ready date, progress, critical health, next milestone,
and open constraints. The detail screen owns the task table, Gantt, mission
constraints, and import action.

- [ ] **Step 5: Implement task table/editor**

Support task title, WBS, parent, owner, start, due, duration, progress, status,
and finish-to-start dependencies. Block Save when the shared schedule command
reports a cycle or missing dependency.

- [ ] **Step 6: Implement the Gantt renderer**

Use a shared horizontal date scale and virtualized vertical task list. Render:

- critical bars in the naval warning colour;
- non-critical bars in the standard accent;
- completion fill;
- dependency connectors;
- Today marker;
- mission-ready milestone; and
- an accessible textual critical/float label on each task row.

- [ ] **Step 7: Extend the E2E bridge and community reset**

Mock planning kinds and schedule calculation. Reset any plan aggregate or
schedule caches on community switch.

- [ ] **Step 8: Run UI checks**

Run:

```bash
cd desktop
pnpm test -- src/features/plans src/app
pnpm check
pnpm exec playwright test tests/e2e/plans.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src/app desktop/src/features/sidebar desktop/src/features/plans desktop/src/testing/e2eBridge.ts desktop/tests/e2e/plans.spec.ts
git commit -m "feat: add deployment Plans workspace"
```

### Task 4: Add mission constraints without pretending to implement OPLIM/risk

**Files:**
- Create: `desktop/src/features/plans/ui/MissionConstraintsPanel.tsx`
- Create: `desktop/src/features/plans/ui/MissionConstraintsPanel.test.mjs`
- Create: `desktop/src/features/plans/ui/ConstraintEditorDialog.tsx`
- Modify: `desktop/src/features/plans/ui/PlanDetailScreen.tsx`
- Modify: `desktop/src/features/plans/ui/TaskEditorDialog.tsx`
- Modify: `desktop/tests/e2e/plans.spec.ts`

**Interfaces:**
- Produces: constraint creation/edit/disposition flow
- Consumes: Task 1 `MissionConstraint` persistence

- [ ] **Step 1: Write seaboat-davit behaviour tests**

Seed:

- mission requirement `Conduct seaboat operations`;
- capability milestone `Seaboat capability available`;
- repair task `Repair port seaboat davit`; and
- an open defect constraint linking all three.

Assert the constraint remains open when the task is only administratively
completed and resolves only after an explicit constraint disposition.

- [ ] **Step 2: Implement constraint editor**

Require description, owner, severity, status, and at least one linked mission
requirement, capability, task, or milestone. `oplimCandidate` and
`riskCandidate` require a disposition note but do not generate a fake OPLIM or
risk assessment.

- [ ] **Step 3: Implement the panel**

Group constraints by Open, Candidate disposition, Mitigated, and Resolved.
Always show open critical constraints above the Gantt fold and include the
linked task/milestone.

- [ ] **Step 4: Add critical-path relationship presentation**

When a linked repair task is critical, state **On calculated critical path**.
When it has float but blocks a mission requirement, state **Mission-critical
constraint outside calculated path**.

- [ ] **Step 5: Run focused tests**

Run:

```bash
cd desktop
pnpm test -- src/features/plans/ui/MissionConstraintsPanel.test.mjs
pnpm exec playwright test tests/e2e/plans.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src/features/plans desktop/tests/e2e/plans.spec.ts
git commit -m "feat: track plan mission constraints"
```

### Task 5: Project task deadlines into Battle Rhythm and Apple Calendar

**Files:**
- Create: `desktop/src/features/plans/domain/calendarProjection.ts`
- Create: `desktop/src/features/plans/domain/calendarProjection.test.mjs`
- Modify: `desktop/src/features/battle-rhythm/hooks.ts`
- Modify: `desktop/src/features/battle-rhythm/ui/MonthCalendar.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/WeekCalendar.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/DayShortcast.tsx`
- Modify: `desktop/src/features/battle-rhythm/data/applePublication.ts`
- Modify: `desktop/src/app/navigation/useAppNavigation.ts`
- Modify: `desktop/tests/e2e/battle-rhythm.spec.ts`

**Interfaces:**
- Produces: `projectTaskMilestone(task, project) -> CalendarProjection | null`
- Consumes: approved live task/project heads

- [ ] **Step 1: Write projection tests**

Assert:

- an approved incomplete leaf task with a due date projects one all-day item;
- summary, cancelled, undated, and deleted tasks do not project;
- completion changes the visual status without changing identity;
- moving the due date preserves identity;
- the identity is `plan-task:<task-id>`; and
- projection carries `/plans/<project-id>?task=<task-id>`.

- [ ] **Step 2: Implement the pure projection**

Do not persist a duplicate Battle Rhythm event. Merge projections after
calendar-event queries and sort them before timed events.

- [ ] **Step 3: Render linked read-only milestones**

Place them at the top of Month day cells and Week/Day all-day areas. Clicking
opens the plan and selected task. Do not show Edit Event for projections.

- [ ] **Step 4: Include projections in Apple reconciliation**

Send stable external ID `plan-task:<task-id>`. Moving or completing the task
updates the corresponding Apple event; deleting/cancelling it removes the
event.

- [ ] **Step 5: Run projection and E2E tests**

Run:

```bash
cd desktop
pnpm test -- src/features/plans/domain/calendarProjection.test.mjs
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts tests/e2e/plans.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
. ./bin/activate-hermit
git add desktop/src/features/plans desktop/src/features/battle-rhythm desktop/src/app/navigation/useAppNavigation.ts desktop/tests/e2e
git commit -m "feat: project plan milestones into Battle Rhythm"
```

### Task 6: Import a representative deployment Gantt and prove Slice 2

**Files:**
- Create: `desktop/src/features/plans/ui/PlanImportReviewDialog.tsx`
- Create: `desktop/src/features/plans/domain/importProposal.ts`
- Create: `desktop/src/features/plans/domain/importProposal.test.mjs`
- Modify: `desktop/src-tauri/src/commands/battle_rhythm.rs`
- Create: `desktop/tests/e2e/plans-screenshots.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Create: `docs/testing/plans-live-acceptance.md`

**Interfaces:**
- Produces: reviewed deployment-plan import
- Consumes: calendar plan-import extraction/structured-completion seam

- [ ] **Step 1: Define and test the exact plan import contract**

```ts
export type PlanImportProposal = {
  readonly schemaVersion: 1;
  readonly project: ProposedPlanningProject;
  readonly tasks: readonly ProposedPlanningTask[];
  readonly constraints: readonly ProposedMissionConstraint[];
  readonly uncertainties: readonly PlanImportUncertainty[];
};
```

Reject duplicate task IDs/WBS values, missing parents/dependencies, invalid
dates, and source-less rows.

- [ ] **Step 2: Add deployment-plan interpretation**

Pass bounded workbook cells and merged-range metadata to the shared structured
completion path. Require exact WBS, task, owner, start, due, duration, progress,
and dependency output. A missing dependency remains an uncertainty; it is not
invented.

- [ ] **Step 3: Build the reviewed import flow**

Show the source row/cells beside proposed project/tasks. Require resolution of
invalid dependency references before approval, then publish the signed
project/task/constraint heads.

- [ ] **Step 4: Add full E2E and screenshots**

Use a synthetic NT-style WBS fixture to prove import, critical-path result,
constraint panel, and calendar milestone. Capture distinct Project List,
Gantt, Task Editor, and Mission Constraints images.

- [ ] **Step 5: Run the complete Slice 2 gate**

Run:

```bash
. ./bin/activate-hermit
cargo test -p buzz-core planning
cargo test --manifest-path desktop/src-tauri/Cargo.toml plans
cd desktop
pnpm check
pnpm test
pnpm exec playwright test tests/e2e/plans.spec.ts tests/e2e/plans-screenshots.spec.ts tests/e2e/battle-rhythm.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 6: Perform live macOS acceptance**

Using the signed application and the local NT planning workbook:

1. open Plans from the sidebar;
2. import the Navigation Department WBS;
3. resolve any parser uncertainty;
4. approve the plan;
5. verify task hierarchy and critical path;
6. add the seaboat davit defect constraint;
7. move one critical due date;
8. verify the Gantt recalculates;
9. verify the due-date milestone moves in Battle Rhythm and Apple Calendar;
   and
10. verify a mission constraint can remain open outside the calculated path.

Record the exact result and bounded limitations in
`docs/testing/plans-live-acceptance.md`.

- [ ] **Step 7: Run the repository gate and commit**

```bash
. ./bin/activate-hermit
just ci
git add desktop crates/buzz-core docs/testing/plans-live-acceptance.md
git commit -m "test: prove Plans and critical path journey"
```
