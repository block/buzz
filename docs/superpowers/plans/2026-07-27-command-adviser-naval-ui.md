# Command Adviser Naval UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the selected Option 1 HMAS Supply naval interface, a decision-first Daily Command Brief, symbolic adviser identities, and a complete user-facing macOS identity change from Buzz to Command Adviser.

**Architecture:** Keep the existing Command Adviser hooks, contracts, routing, Apple/RAG/Memory integrations, and persistence unchanged. Add a small presentation-only naval identity layer, reorganise the existing validated brief fields into a decision-first view, collapse evidence and system diagnostics behind one disclosure, and change only user-facing Tauri/macOS identity metadata while preserving the bundle identifier and internal Buzz protocol/storage names.

**Tech Stack:** Tauri 2, React 19, TypeScript 6, Tailwind CSS, existing shared UI components, Lucide React for standard interface icons, one generated raster sextant insignia, Node test runner, Playwright, macOS `iconutil`/Tauri icon tooling.

## Global Constraints

- The selected visual source is Option 1, the dark-navy Quarterdeck Brief concept displayed on 27 July 2026.
- The user-facing product name is exactly `Command Adviser`.
- Preserve `xyz.block.buzz.app`, the `buzz://` deep-link scheme, Keychain service names, storage keys, crate/binary names, and signed-event contracts.
- Keep Cloud first / Local first routing, active-run locking, Apple/RAG/Memory access, fail-soft behaviour, scheduling, cancellation, and signed publication unchanged.
- Use the official HMAS Supply badge unaltered and store optimised ship assets locally for offline use.
- Use rem-based text tokens only; do not introduce arbitrary text sizes.
- Keep citations, limitations, dissent, freshness, model/provider provenance, and connector health available and truthful.
- Do not add Phase 5 workspace actions, RAG 2.0, replication, security gates, or new backend endpoints.
- Every production behaviour change follows red-green-refactor; generated binary assets are verified by tests immediately after generation.

---

## File Structure

- `desktop/src/features/command-console/ui/CommandAdviserHero.tsx`
  owns the HMAS Supply badge, ship photograph, product identity, motto, and
  model-routing placement.
- `desktop/src/features/command-console/ui/AdviserInsignia.tsx`
  owns the six symbolic adviser identities and their accessible labels.
- `desktop/src/features/command-console/ui/CommandTeamStrip.tsx`
  renders the six compact adviser cards without inventing biographies or rank.
- `desktop/src/features/command-console/ui/BriefSectionCard.tsx`
  renders one decision-first brief section with citations and honest empty
  states.
- `desktop/src/features/command-console/ui/BriefEvidenceDisclosure.tsx`
  owns the collapsed evidence, provenance, specialist detail, lifecycle, and
  system-status disclosure.
- `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
  composes the hero, team, brief, and evidence/system status without changing
  hooks or routing.
- `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
  owns generation controls, useful brief content, limitation summaries, and
  scheduling.
- `desktop/src/app/CommandAdviserLoadingMark.tsx`
  replaces the visible Buzz bee/wordmark during native startup gates.
- `desktop/src/assets/command-adviser/`
  contains local badge, ship, sextant, and attribution assets.
- `desktop/src-tauri/icons/`
  contains the generated Command Adviser app/DMG icon family.
- `desktop/scripts/check-command-adviser-branding.mjs`
  verifies product identity metadata and preservation of stable internals.
- Existing SSR and Playwright tests cover reading order, disclosures,
  accessibility, routing controls, and generated screenshots.

---

### Task 1: Establish Local Naval Assets and macOS Product Identity

**Files:**
- Create: `desktop/src/assets/command-adviser/hmas-supply-badge.png`
- Create: `desktop/src/assets/command-adviser/hmas-supply.jpg`
- Create: `desktop/src/assets/command-adviser/sextant-insignia.png`
- Create: `desktop/src/assets/command-adviser/ATTRIBUTION.md`
- Create: `desktop/src/assets/command-adviser/command-adviser-app-icon.png`
- Create: `desktop/scripts/check-command-adviser-branding.mjs`
- Create: `desktop/src/app/CommandAdviserLoadingMark.tsx`
- Modify: `desktop/src/app/App.tsx`
- Modify: `desktop/src-tauri/tauri.conf.json`
- Modify: `desktop/src-tauri/Info.plist`
- Modify: `desktop/src-tauri/icons/*`
- Modify: `desktop/src-tauri/icons/dmg-background.png`
- Test: `desktop/src/app/CommandAdviserLoadingMark.test.mjs`
- Test: `desktop/scripts/check-command-adviser-branding.test.mjs`

**Interfaces:**
- Consumes: official badge and ship URLs recorded in the approved design.
- Produces: `CommandAdviserLoadingMark(): JSX.Element` and stable asset paths
  under `@/assets/command-adviser/`.

- [ ] **Step 1: Write the failing branding metadata test**

Create `desktop/scripts/check-command-adviser-branding.test.mjs` using the real
files rather than mocks:

```js
import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

test("macOS product identity is Command Adviser without changing stable internals", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url)),
  );
  const plist = await readFile(
    new URL("../src-tauri/Info.plist", import.meta.url),
    "utf8",
  );

  assert.equal(config.productName, "Command Adviser");
  assert.equal(config.identifier, "xyz.block.buzz.app");
  assert.deepEqual(config.plugins["deep-link"].desktop.schemes, ["buzz"]);
  assert.match(plist, /<string>Command Adviser<\/string>/);
  assert.doesNotMatch(plist, />Buzz needs|>Buzz can read/);
});
```

- [ ] **Step 2: Run the branding test and verify RED**

Run:

```bash
. ./bin/activate-hermit
cd desktop
node --test scripts/check-command-adviser-branding.test.mjs
```

Expected: FAIL because `productName` and plist descriptions still say `Buzz`.

- [ ] **Step 3: Write the failing startup-mark test**

Create `desktop/src/app/CommandAdviserLoadingMark.test.mjs`:

```js
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { CommandAdviserLoadingMark } from "./CommandAdviserLoadingMark.tsx";

test("startup mark presents Command Adviser without Buzz branding", () => {
  const html = renderToStaticMarkup(
    React.createElement(CommandAdviserLoadingMark),
  );
  assert.match(html, /Command Adviser/);
  assert.match(html, /Strengthen the Shield/i);
  assert.doesNotMatch(html, /Buzz|bee/i);
});
```

- [ ] **Step 4: Run the startup-mark test and verify RED**

Run:

```bash
cd desktop
pnpm test -- src/app/CommandAdviserLoadingMark.test.mjs
```

Expected: FAIL because `CommandAdviserLoadingMark.tsx` does not exist.

- [ ] **Step 5: Generate and verify the app-owned assets**

Use the official Navy asset URLs from the design and keep local source files.
Use Image Gen for:

1. `command-adviser-app-icon.png`: square macOS icon art, deep navy field,
   restrained brass compass/command mark, replenishment ship silhouette, no
   text, crown, official badge, or Defence/RAN logo.
2. `sextant-insignia.png`: single flat brass line-art sextant on transparent or
   clean deep-navy field, visually compatible with the app icon and legible at
   24–48 px.

Optimise the official ship photograph for a shallow 1600 px-wide hero and the
badge for a 256 px display source. Verify every file decodes and record source,
retrieval date, and usage terms in `ATTRIBUTION.md`.

Generate the Tauri icon family from the 1024 px source:

```bash
cd desktop
pnpm tauri icon src/assets/command-adviser/command-adviser-app-icon.png \
  --output src-tauri/icons
```

- [ ] **Step 6: Implement the metadata and loading identity**

Update `tauri.conf.json` to:

```json
{
  "productName": "Command Adviser",
  "identifier": "xyz.block.buzz.app"
}
```

Keep the existing `buzz` deep-link scheme and sidecar names. Replace each
user-facing `Buzz` subject in `Info.plist` with `Command Adviser`.

Implement `CommandAdviserLoadingMark` as a semantic image-and-copy component:

```tsx
export function CommandAdviserLoadingMark() {
  return (
    <div aria-label="Command Adviser" className="flex flex-col items-center gap-3">
      <img alt="" className="h-20 w-20" src={appIcon} />
      <p className="text-base font-semibold tracking-wide">Command Adviser</p>
      <p className="text-xs uppercase tracking-widest text-muted-foreground">
        Strengthen the Shield
      </p>
    </div>
  );
}
```

Replace visible startup uses of `BuzzMark`, `FlappingBee`, and `FuzzyLogo` in
`App.tsx` with the new component while retaining loading-state behaviour and
reduced-motion handling.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
cd desktop
pnpm test -- \
  src/app/CommandAdviserLoadingMark.test.mjs \
  scripts/check-command-adviser-branding.test.mjs
pnpm typecheck
```

Expected: both focused tests PASS and TypeScript exits 0.

- [ ] **Step 8: Commit Task 1**

```bash
git add desktop/src/assets/command-adviser desktop/src/app \
  desktop/src-tauri/tauri.conf.json desktop/src-tauri/Info.plist \
  desktop/src-tauri/icons desktop/scripts/check-command-adviser-branding*
git commit -m "feat(command-adviser): establish naval product identity"
```

---

### Task 2: Build the HMAS Supply Hero and Symbolic Command Team

**Files:**
- Create: `desktop/src/features/command-console/ui/CommandAdviserHero.tsx`
- Create: `desktop/src/features/command-console/ui/AdviserInsignia.tsx`
- Create: `desktop/src/features/command-console/ui/CommandTeamStrip.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Modify: `desktop/src/features/command-console/ui/ModelRoutingControls.tsx`
- Test: `desktop/src/features/command-console/ui/CommandConsoleScreen.test.mjs`
- Test: `desktop/src/features/command-console/ui/AdviserInsignia.test.mjs`

**Interfaces:**
- Produces:

```ts
export type CommandAdviserId =
  | "chief_of_staff"
  | "operations"
  | "navigation"
  | "daily_routine"
  | "reporting"
  | "plans";

export function AdviserInsignia(props: {
  adviser: CommandAdviserId;
  className?: string;
}): JSX.Element;

export function CommandTeamStrip(): JSX.Element;
```

- `CommandAdviserHero` consumes the existing `ModelRoutingControls` element as
  a child and does not own routing state.

- [ ] **Step 1: Extend the console SSR test for the approved hero and team**

Add assertions to `CommandConsoleScreen.test.mjs`:

```js
assert.match(html, /HMAS SUPPLY · A195/);
assert.match(html, /STRENGTHEN THE SHIELD/);
assert.match(html, /alt="HMAS Supply at sea"/);
for (const adviser of [
  "chief-of-staff",
  "operations",
  "navigation",
  "daily-routine",
  "reporting",
  "plans",
]) {
  assert.match(html, new RegExp(`data-testid="adviser-insignia-${adviser}"`));
}
assert.doesNotMatch(html, />Command Console</);
```

- [ ] **Step 2: Write the failing insignia mapping test**

Create `AdviserInsignia.test.mjs` and render all six IDs. Assert exact accessible
labels and six unique `data-symbol` values:

```js
const expected = {
  chief_of_staff: "Chief of Staff — command anchor",
  operations: "Operations Adviser — radar plot",
  navigation: "Navigation Adviser — sextant",
  daily_routine: "Daily Routine Adviser — ship's bell",
  reporting: "Reporting Adviser — clipboard and returns",
  plans: "Plans Adviser — charted course",
};
```

- [ ] **Step 3: Run the focused UI tests and verify RED**

Run:

```bash
cd desktop
pnpm test -- \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs \
  src/features/command-console/ui/AdviserInsignia.test.mjs
```

Expected: FAIL because the hero/team components and adviser insignia do not
exist.

- [ ] **Step 4: Implement the insignia and team strip**

Use the existing Lucide family for `Anchor`, `Radar`, `Bell`, `ClipboardList`,
and `Route`; use the generated sextant raster only for Navigation. Map IDs to
labels in one exported immutable record. Render each in a consistent circular
navy/brass medallion with accessible hidden text and a `data-symbol` marker.

`CommandTeamStrip` renders six compact cards:

```ts
const COMMAND_TEAM = [
  ["chief_of_staff", "Chief of Staff", "Consolidates the command brief"],
  ["operations", "Operations", "Priorities, readiness and risk"],
  ["navigation", "Navigation", "Navigation evidence and limitations"],
  ["daily_routine", "Daily Routine", "Calendar, reminders and routine"],
  ["reporting", "Reporting", "Reports, returns and missing inputs"],
  ["plans", "Plans", "30, 60 and 90-day outlook"],
] as const;
```

- [ ] **Step 5: Implement the restrained HMAS Supply hero**

Create a dark layered card using the local ship image as a real `<img>` with
`object-cover`, a dark overlay, the unaltered badge, product copy, and the
existing routing controls. The top-level text is:

```text
COMMAND ADVISER
HMAS SUPPLY · A195
STRENGTHEN THE SHIELD
```

Move the existing routing controls into the hero composition without changing
their props or active-run lock.

- [ ] **Step 6: Compose the new console shell**

Replace the former banner/header/advisory stack in `CommandConsoleScreen.tsx`
with `CommandAdviserHero`, a concise advisory note, `CommandTeamStrip`, and
`DailyCommandBrief`. Pass system status into the brief for Task 3 instead of
rendering the large status grid before the useful content.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
cd desktop
pnpm test -- \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs \
  src/features/command-console/ui/AdviserInsignia.test.mjs
pnpm typecheck
```

Expected: focused tests PASS and TypeScript exits 0.

- [ ] **Step 8: Commit Task 2**

```bash
git add desktop/src/features/command-console/ui
git commit -m "feat(command-adviser): add HMAS Supply command shell"
```

---

### Task 3: Reorder the Brief Around Decisions and Collapse Evidence

**Files:**
- Create: `desktop/src/features/command-console/ui/BriefSectionCard.tsx`
- Create: `desktop/src/features/command-console/ui/BriefEvidenceDisclosure.tsx`
- Modify: `desktop/src/features/command-console/ui/DailyCommandBrief.tsx`
- Modify: `desktop/src/features/command-console/ui/AdviserContributionCard.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandSystemStatus.tsx`
- Modify: `desktop/src/features/command-console/ui/CommandConsoleScreen.tsx`
- Modify: `desktop/src/features/command-console/ui/SourceCitationLink.tsx`
- Test: `desktop/src/features/command-console/ui/DailyCommandBrief.test.mjs`
- Test: `desktop/tests/e2e/daily-command-brief.spec.ts`
- Test: `desktop/tests/e2e/command-console.spec.ts`

**Interfaces:**
- `DailyCommandBriefProps` adds:

```ts
readonly systemStatus: CommandConsoleStatusViewModel;
```

- `BriefEvidenceDisclosure` consumes:

```ts
{
  published: PublishedCommandBrief;
  status: BriefRunStatus | null;
  history: readonly BriefRunStatus[];
  systemStatus: CommandConsoleStatusViewModel;
}
```

- [ ] **Step 1: Change the SSR test to the decision-first contract**

In `DailyCommandBrief.test.mjs`, replace the old all-nine-visible-sections
assertion with:

```js
const decisions = html.indexOf(">Decisions and approvals required<");
const today = html.indexOf(">Today at a glance<");
const operations = html.indexOf(">Operational priorities and risks<");
assert.ok(decisions >= 0 && decisions < today && today < operations);
assert.match(html, /<details[^>]*data-testid="brief-evidence-disclosure"/);
assert.doesNotMatch(
  html.slice(0, html.indexOf('data-testid="brief-evidence-disclosure"')),
  />Sources<|>Source ledger<|>Lifecycle history</,
);
assert.match(html, /Evidence and system status/);
assert.match(html, /Specialist adviser contributions/);
assert.match(html, /Source ledger/);
```

Pass a real `systemStatus` fixture containing the existing six service IDs.

- [ ] **Step 2: Update the E2E expectations before implementation**

Make `daily-command-brief.spec.ts` assert:

- Decisions is the first brief section.
- The evidence disclosure is closed by default.
- Source ledger, specialist contributions, snapshot ID, lifecycle history, and
  service cards are not visible while closed.
- Clicking `Evidence and system status` opens the disclosure.
- The existing citation link opens the disclosure if necessary, focuses the
  correct source, and retains the source metadata.
- Routing controls remain usable and no action approval/execute control appears.

- [ ] **Step 3: Run SSR and E2E tests and verify RED**

Run:

```bash
cd desktop
pnpm test -- src/features/command-console/ui/DailyCommandBrief.test.mjs
pnpm build:e2e
pnpm exec playwright test \
  tests/e2e/daily-command-brief.spec.ts \
  tests/e2e/command-console.spec.ts \
  --project=smoke
```

Expected: FAIL because sections use the old order and evidence/system status are
always expanded or rendered ahead of the brief.

- [ ] **Step 4: Implement the explicit main-section order**

Do not mutate `BRIEF_SECTIONS`. Add a presentation-only order:

```ts
const COMMAND_READING_ORDER: readonly BriefSection[] = [
  "decisions",
  "today",
  "operations",
  "navigation",
  "daily_routine",
  "reports",
  "planning_30_60_90",
];
```

Map `decisions` to `Decisions and approvals required`. Render decisions as the
prominent full-width card; render Today full-width below it; use a responsive
two-column grid for remaining sections. Keep inline citation markers on each
finding.

Render `conflicts_and_gaps`, missing information, degraded sections, and dissent
as a compact **Watch items** summary before the planning grid. Keep honest empty
states.

- [ ] **Step 5: Implement the collapsed evidence disclosure**

Use a native `<details>` without the `open` attribute:

```tsx
<details data-testid="brief-evidence-disclosure">
  <summary>Evidence and system status</summary>
  {/* publication metadata, advisory limitation, specialist contributions,
      source ledger, lifecycle history, provider/audit fields, service status */}
</details>
```

Move `SourceLedger`, adviser contributions, generated snapshot/publication
metadata, lifecycle history, advisory limitation, and `CommandSystemStatus`
inside it. Preserve all data; only change visual priority.

When a `SourceCitationLink` is activated, open the closest evidence disclosure
before focusing the source ledger target so citation navigation still works
from the collapsed state.

- [ ] **Step 6: Tighten specialist cards for detail view**

Add `AdviserInsignia` to each specialist contribution and keep confidence,
findings, limitations, dissent, and proposals. Reduce visual prominence because
these cards are now supporting evidence.

Change user-facing `Buzz relay` copy in `useCommandConsoleStatus.ts` and
`CommandSystemStatus.tsx` to `Command workspace` or `Workspace relay`; retain
the internal relay implementation and IDs.

- [ ] **Step 7: Run focused tests and verify GREEN**

Run:

```bash
cd desktop
pnpm test -- \
  src/features/command-console/ui/DailyCommandBrief.test.mjs \
  src/features/command-console/ui/CommandConsoleScreen.test.mjs
pnpm build:e2e
pnpm exec playwright test \
  tests/e2e/daily-command-brief.spec.ts \
  tests/e2e/command-console.spec.ts \
  --project=smoke
```

Expected: all focused unit and E2E tests PASS.

- [ ] **Step 8: Commit Task 3**

```bash
git add desktop/src/features/command-console desktop/tests/e2e
git commit -m "feat(command-adviser): make briefs decision first"
```

---

### Task 4: Match the Selected Visual and Pass Design QA

**Files:**
- Create: `desktop/tests/e2e/command-adviser-naval-ui.spec.ts`
- Modify: `desktop/playwright.config.ts`
- Create: `design-qa.md`
- Create: `test-results/command-adviser-naval-ui/*.png` (untracked evidence)

**Interfaces:**
- Consumes: the Option 1 generated reference image:
  `/Users/matthewwarren/.codex/generated_images/019f91f9-3cb6-7323-bc4f-8bcfcc7e0373/call_Tl4utET31OeAI23lycQaKv22.png`.
- Produces: deterministic desktop screenshots at 1280×720 and a passing
  `design-qa.md`.

- [ ] **Step 1: Write the visual E2E before styling polish**

The spec must:

1. install the mock bridge with a populated brief;
2. navigate to `#/console`;
3. assert the HMAS Supply badge, ship image, six symbolic team identities, model
   routing toggle, and decision-first brief;
4. capture the default briefing state after `waitForAnimations(page)`;
5. open Evidence and system status, assert sources and service status, and
   capture the expanded state;
6. use locator screenshots or distinct clips and verify the PNG hashes differ.

- [ ] **Step 2: Run the visual spec and verify RED**

Run:

```bash
cd desktop
pnpm build:e2e
pnpm exec playwright test \
  tests/e2e/command-adviser-naval-ui.spec.ts \
  --project=smoke
```

Expected: FAIL until the spec is registered and the selected layout is styled
at the target viewport.

- [ ] **Step 3: Apply bounded visual refinement**

Compare the rendered console with the reference at the same 1280×720 viewport.
Adjust only Command Adviser presentation:

- deep navy/charcoal surfaces;
- brass primary accents;
- shallow ship hero;
- compact routing controls;
- calm spacing and section hierarchy;
- responsive two-column content;
- clear degraded/decision states;
- no arbitrary text sizing or decorative instrument-panel effects.

- [ ] **Step 4: Run the Product Design QA gate**

Open the reference and captured app screenshot side by side. Write
`design-qa.md` with P0–P3 findings. Fix all P0/P1/P2 issues, recapture, and
repeat until the final line is:

```text
final result: passed
```

Leave only optional P3 polish notes.

- [ ] **Step 5: Run desktop quality gates**

Run:

```bash
cd desktop
pnpm check
pnpm test
pnpm build:e2e
pnpm exec playwright test \
  tests/e2e/command-console.spec.ts \
  tests/e2e/daily-command-brief.spec.ts \
  tests/e2e/command-adviser-naval-ui.spec.ts \
  --project=smoke
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit Task 4**

```bash
git add desktop/tests/e2e desktop/playwright.config.ts design-qa.md
git commit -m "test(command-adviser): verify naval briefing experience"
```

---

### Task 5: Verify the Native App, Bundle, DMG, and Stable Integrations

**Files:**
- Modify if required: `desktop/scripts/check-command-adviser-branding.mjs`
- Modify if required: `desktop/src-tauri/tauri.conf.json`
- Modify if required: `desktop/src-tauri/Info.plist`
- Modify: `docs/superpowers/specs/2026-07-27-command-adviser-naval-ui-design.md`
  only if verified implementation details require a factual correction.

**Interfaces:**
- Consumes: built `Command Adviser.app`, DMG, existing Keychain credentials,
  existing model routing configuration, and installed macOS privacy grants.
- Produces: a launchable native Command Adviser application without breaking
  stable identity or the current Daily Command Brief flow.

- [ ] **Step 1: Add bundle inspection to the branding checker**

When passed a built `.app`, the checker must run `plutil` and assert:

```text
CFBundleDisplayName = Command Adviser
CFBundleName = Command Adviser
CFBundleIdentifier = xyz.block.buzz.app
```

It must also assert that the bundle contains the generated icon and existing
sidecars, and that the DMG/application path includes `Command Adviser`.

- [ ] **Step 2: Run the checker against the pre-change bundle and verify RED**

If a previous Buzz bundle exists, run the checker against it and confirm it
fails on product identity. Do not treat absence of an old bundle as a test
failure; the source-level test from Task 1 already proved the red state.

- [ ] **Step 3: Build the native release app and DMG**

Run:

```bash
. ./bin/activate-hermit
cd desktop
pnpm tauri build
```

Expected: release `.app` and DMG are produced under the configured Tauri target
directory with Command Adviser naming.

- [ ] **Step 4: Inspect the built artifacts**

Run the branding checker against the built `.app`, then use:

```bash
plutil -p "<app>/Contents/Info.plist"
codesign --verify --deep --strict --verbose=2 "<app>"
spctl --assess --type execute --verbose=4 "<app>"
```

Record the distinction between a valid local/Developer ID signature and
notarisation; do not claim notarisation unless the assessment proves it.

- [ ] **Step 5: Exercise the real user journey**

Launch the built app and verify:

1. Finder, Dock, menu bar, startup mark, and window present `Command Adviser`;
2. the HMAS Supply hero and symbolic team render;
3. the existing cloud/local toggle reflects the persisted preference;
4. one brief can be opened or generated;
5. useful information is visible before evidence;
6. Evidence and system status expands;
7. existing RAG, Memory, Apple, route, citation, and signed-publication status
   remain truthful.

If live source availability prevents generation, verify the most recent
persisted brief and report the live limitation rather than changing the UI.

- [ ] **Step 6: Run the full repository gate**

Run:

```bash
. ./bin/activate-hermit
just ci
```

Expected: exit 0. If relay/auth/database integration code was not touched,
`just test` is not additionally required; if it was touched unexpectedly, run
`just test` with Postgres and Redis available.

- [ ] **Step 7: Record the verified phase in Memory MCP**

Record one high-value event with agent `CODEX` covering:

- branch and commit;
- visible Command Adviser identity;
- preserved bundle identifier and stable internals;
- decision-first brief and evidence disclosure;
- exact verification commands and any remaining live limitations.

- [ ] **Step 8: Commit and push the completed phase**

```bash
git add -A
git commit -m "feat(command-adviser): deliver naval command experience"
git push origin codex/command-adviser-naval-ui
```

Update draft PR #10 with verification evidence and screenshots. Do not mark it
ready or merge without the user requesting that transition.
