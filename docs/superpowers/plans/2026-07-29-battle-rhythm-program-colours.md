# Battle Rhythm Program Colours Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Colour the all-day ship program by at-sea or in-port state and render multi-day all-day events once in a dedicated Week lane.

**Architecture:** Add one pure presentation module that classifies events and calculates their clipped Week placement. Calendar components consume those shared semantics while retaining their existing data and editing flows; no contract, persistence, import, or routine-inference changes are required.

**Tech Stack:** TypeScript, React 19, Tailwind CSS, Node test runner, Playwright, Tauri 2.

## Global Constraints

- Colour classification applies only to all-day events.
- A location containing `Sea` as a case-insensitive word is blue.
- Any other non-empty all-day location, including FBE and FBW, is yellow.
- Blank-location all-day events and every timed event retain neutral styling.
- Week view renders an all-day event once, clipped to and spanning its overlapping days.
- Day, Month, Week, and Year consume the same semantic classifier.
- No event data, source revision, FAS routine inference, or Apple Calendar record is modified by this presentation change.

---

### Task 1: Shared program-event presentation semantics

**Files:**
- Create: `desktop/src/features/battle-rhythm/domain/eventPresentation.ts`
- Create: `desktop/src/features/battle-rhythm/domain/eventPresentation.test.mjs`

**Interfaces:**
- Consumes: `BattleRhythmEvent` from `domain/contracts.ts`; `DateRange`, `addDays`, and `overlapsCalendarDay` from `domain/dateRange.ts`.
- Produces: `ProgramEventTone = "sea" | "port" | "neutral"`; `programEventTone(event)`; `weekAllDayPlacement(event, range, timeZone)`.

- [ ] **Step 1: Write failing semantic tests**

```js
test("all-day ship locations classify as sea, port, or neutral", () => {
  assert.equal(programEventTone(event({ allDay: true, location: "Sea" })), "sea");
  assert.equal(programEventTone(event({ allDay: true, location: "At Sea" })), "sea");
  assert.equal(programEventTone(event({ allDay: true, location: "FBE" })), "port");
  assert.equal(programEventTone(event({ allDay: true, location: "FBW" })), "port");
  assert.equal(programEventTone(event({ allDay: true, location: "Sydney" })), "port");
  assert.equal(programEventTone(event({ allDay: true, location: "Fremantle" })), "port");
  assert.equal(programEventTone(event({ allDay: true, location: "Seaside" })), "port");
  assert.equal(programEventTone(event({ allDay: true, location: "  " })), "neutral");
  assert.equal(programEventTone(event({ allDay: false, location: "Sea" })), "neutral");
});

test("a cross-week all-day event is clipped to its visible columns", () => {
  assert.deepEqual(
    weekAllDayPlacement(
      event({
        allDay: true,
        start: "2026-07-29T00:00:00+10:00",
        end: "2026-08-05T00:00:00+10:00",
      }),
      {
        start: "2026-07-27T00:00:00+10:00",
        end: "2026-08-03T00:00:00+10:00",
      },
      "Australia/Sydney",
    ),
    { startColumn: 3, span: 5 },
  );
});
```

- [ ] **Step 2: Run the focused test and confirm RED**

Run: `cd desktop && node --test src/features/battle-rhythm/domain/eventPresentation.test.mjs`

Expected: FAIL because `eventPresentation.ts` does not exist.

- [ ] **Step 3: Implement minimal deterministic semantics**

```ts
export type ProgramEventTone = "sea" | "port" | "neutral";

export function programEventTone(
  event: Pick<BattleRhythmEvent, "allDay" | "location">,
): ProgramEventTone {
  if (!event.allDay) return "neutral";
  const location = event.location?.trim();
  if (!location) return "neutral";
  return /\bsea\b/i.test(location) ? "sea" : "port";
}

export function weekAllDayPlacement(
  event: Pick<BattleRhythmEvent, "allDay" | "start" | "end">,
  range: DateRange,
  timeZone: string,
): Readonly<{ startColumn: number; span: number }> | null {
  if (!event.allDay) return null;
  const weekStart = range.start.slice(0, 10);
  const columns = Array.from({ length: 7 }, (_, offset) => offset).filter(
    (offset) =>
      overlapsCalendarDay(
        event.start,
        event.end,
        addDays(weekStart, offset),
        timeZone,
      ),
  );
  if (columns.length === 0) return null;
  return { startColumn: columns[0] + 1, span: columns.length };
}
```

- [ ] **Step 4: Run the focused test and confirm GREEN**

Run: `cd desktop && node --test src/features/battle-rhythm/domain/eventPresentation.test.mjs`

Expected: PASS.

- [ ] **Step 5: Commit the semantic unit**

```bash
git add desktop/src/features/battle-rhythm/domain/eventPresentation.ts desktop/src/features/battle-rhythm/domain/eventPresentation.test.mjs
git commit -m "feat: classify ship program events"
```

### Task 2: Calendar colours and Week all-day lane

**Files:**
- Create: `desktop/src/features/battle-rhythm/ui/programEventStyles.ts`
- Modify: `desktop/src/features/battle-rhythm/ui/DayShortcast.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/MonthCalendar.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/WeekCalendar.tsx`
- Modify: `desktop/src/features/battle-rhythm/ui/YearTimeline.tsx`
- Modify: `desktop/tests/e2e/battle-rhythm.spec.ts`

**Interfaces:**
- Consumes: `programEventTone(event)` and `weekAllDayPlacement(event, range, timeZone)` from Task 1.
- Produces: `programEventClasses(event, surface)` for shared accessible colour styling; a Week `data-testid="week-all-day-lane"` containing one selectable bar per all-day event.

- [ ] **Step 1: Change the existing all-day Playwright expectation to the approved Week behaviour**

```ts
await expect(
  screen.getByTestId("week-all-day-lane").getByRole("button", {
    name: "All day SMP",
  }),
).toHaveCount(1);
await expect(
  screen.getByTestId("week-timed-columns").getByRole("button", {
    name: "All day SMP",
  }),
).toHaveCount(0);
```

Add two all-day events through the real manual-event editor, one with location `Sea` and one with location `FBE`, and assert their `data-program-tone` values remain `sea` and `port` after switching through Week, Month, Day, and Year.

- [ ] **Step 2: Run the focused E2E test and confirm RED**

Run: `cd desktop && pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts --project=smoke --grep "ship-time controls"`

Expected: FAIL because the Week event is repeated seven times and the all-day lane has no event bars.

- [ ] **Step 3: Add the shared style vocabulary**

```ts
const toneClasses = {
  sea: "border-blue-400/50 bg-blue-500/20 text-blue-900 dark:text-blue-100",
  port: "border-amber-400/50 bg-amber-400/20 text-amber-950 dark:text-amber-100",
  neutral: "border-primary/20 bg-primary/10 text-primary",
} as const;

export function programEventClasses(
  event: Pick<BattleRhythmEvent, "allDay" | "location">,
): string {
  return toneClasses[programEventTone(event)];
}
```

Each calendar surface adds `data-program-tone={programEventTone(event)}` to the event element. Year cells derive a visible tone from that day's events using precedence `sea`, then `port`, then `neutral`, so a day containing an at-sea program cannot be visually mistaken for alongside.

- [ ] **Step 4: Render Week all-day events once**

Split `shown` into `allDayEvents` and `timedEvents`. Render each all-day event in its own seven-column grid row:

```tsx
<div data-testid="week-all-day-lane">
  {allDayEvents.map((event) => {
    const placement = weekAllDayPlacement(event, range, timeZone);
    if (!placement) return null;
    return (
      <div className="grid grid-cols-7 gap-2" key={event.id}>
        <button
          aria-label={`All day ${event.title}`}
          className={`rounded border px-2 py-1 text-left text-xs ${programEventClasses(event)}`}
          data-program-tone={programEventTone(event)}
          onClick={() => onEdit?.(event)}
          style={{
            gridColumn: `${placement.startColumn} / span ${placement.span}`,
          }}
          type="button"
        >
          {event.title}
        </button>
      </div>
    );
  })}
</div>
```

The seven daily columns receive `data-testid="week-timed-columns"` and map only `timedEvents`; plan milestones remain unchanged.

- [ ] **Step 5: Apply the same tones in Day, Month, and Year**

Day adds the shared classes to the event row's title button. Month adds them to each event pill. Year derives the day's highest-precedence tone and applies the corresponding shared classes to the date cell while retaining the combined title tooltip.

- [ ] **Step 6: Run focused unit and E2E tests and confirm GREEN**

Run:

```bash
cd desktop
node --test src/features/battle-rhythm/domain/eventPresentation.test.mjs
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts --project=smoke --grep "ship-time controls"
```

Expected: both commands PASS.

- [ ] **Step 7: Commit the calendar unit**

```bash
git add desktop/src/features/battle-rhythm/ui desktop/tests/e2e/battle-rhythm.spec.ts
git commit -m "feat: colour the Battle Rhythm ship program"
```

### Task 3: Regression, installed-app acceptance, and delivery

**Files:**
- Modify only if a failing check identifies a scoped defect in the files from Tasks 1–2.

**Interfaces:**
- Consumes: the complete Battle Rhythm calendar implementation.
- Produces: a built and installed Command Adviser macOS application plus a pushed phase branch.

- [ ] **Step 1: Run Battle Rhythm domain and component tests**

Run:

```bash
. ./bin/activate-hermit
cd desktop
node --test src/features/battle-rhythm/domain/*.test.mjs
pnpm test -- --run src/features/battle-rhythm
```

Expected: PASS.

- [ ] **Step 2: Run the complete Battle Rhythm Playwright suite**

Run:

```bash
. ./bin/activate-hermit
cd desktop
pnpm run build
pnpm exec playwright test tests/e2e/battle-rhythm.spec.ts --project=smoke
```

Expected: PASS.

- [ ] **Step 3: Run desktop quality gates**

Run:

```bash
. ./bin/activate-hermit
just desktop-check
just desktop-test
cargo test --manifest-path desktop/src-tauri/Cargo.toml
```

Expected: PASS.

- [ ] **Step 4: Build and install the signed macOS app**

Use the repository's existing Command Adviser packaging/install procedure, preserving the current Keychain and persisted relay data. Confirm `/Applications/Command Adviser.app` is replaced only after a successful build.

- [ ] **Step 5: Verify the installed app read-only**

Launch `/Applications/Command Adviser.app`, open Battle Rhythm, navigate to a populated 2027 or 2028 Week, and confirm:

- the all-day lane contains the persisted multi-day program once;
- a `Sea` location is blue;
- FBE, FBW, and other non-empty locations are yellow;
- timed events remain neutral; and
- switching Day, Month, Week, and Year does not create, edit, or remove events.

- [ ] **Step 6: Commit any verification-only changes and push**

```bash
git status --short
git push origin codex/project-execution-v1
```

Expected: the phase branch and draft PR #13 contain the design, plan, implementation, and tests.

- [ ] **Step 7: Record the implementation checkpoint in Memory MCP**

Record one selective event with agent `CODEX` covering the semantic rule, the Week single-bar behaviour, verification result, commit IDs, and any installation gotcha future work must know.
