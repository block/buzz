# Phase 4 Task 9 Correction Report — Acceptance and Evidence Boundaries

Date: 2026-07-25

## Outcome

Independent review rejected the original Task 9 acceptance at local commit
`216fe301`. The correction keeps that structural work but closes the review's
three actionable gaps:

- the hermetic runner now selects every authoritative evidence suite and denies
  acceptance when any Rust filter selects zero tests;
- native Daily Command Brief E2E proves the visible advisory limitation,
  pending proposal, and citation-to-source interaction, timestamp, and quoted
  location;
- the full desktop smoke gate is rerun after correcting inherited test
  assertions and animation races without changing the profile editor or mock
  event-routing production code.

The correction remains local and has not been pushed.

## Corrected acceptance surface

- `scripts/check-daily-command-brief.sh` is the production acceptance runner.
- Every filtered Rust gate must report at least one selected test. The runner
  explicitly includes `provenance_tests`, `types_tests`, and the
  `buzz-agent` command-evidence negatives that review found missing.
- `scripts/tests/check-daily-command-brief-test.sh` proves exact child commands,
  non-zero selection, failure propagation, success suppression, Just
  invocation, fail-closed live configuration, model forwarding, and truthful
  live output.
- `desktop/tests/e2e/daily-command-brief.spec.ts` covers the empty OFFICIAL
  state and a complete degraded brief. The complete scenario now exercises the
  citation link and verifies focus on the exact ledger entry plus its retrieval
  timestamp and page location.
- `SourceCitationLink` prevents Buzz's hash router from consuming source
  anchors; it scrolls and focuses the ledger card while preserving a real
  anchor target for accessibility and inspection.
- `docs/command-console/phase-4-daily-command-brief.md` documents architecture,
  hermetic acceptance, and the controlled live/offline evidence boundary.

## Review-driven RED/GREEN evidence

RED evidence:

- The runner contract failed before implementation because the authoritative
  provenance gate was absent. A mocked `running 0 tests` result also
  demonstrated that ordinary `cargo test <filter>` success was not sufficient.
- The new citation interaction failed before implementation because raw hash
  navigation was consumed by the Buzz route parser and left the Command
  Console.
- The first corrected full smoke run reported 697 passed, 1 skipped, and 2
  failed. Focused diagnosis found one aggregate-only optimistic-card detachment
  and one reproducible poll-before-seam synchronization defect.

GREEN evidence:

- The runner contract passes with exact-suite and zero-selection assertions.
- The real hermetic runner passes its authoritative non-empty Rust suites,
  3544 desktop tests, backup/restore fixtures, and 18 Apple-helper tests.
- The corrected Daily Command Brief E2E passes both scenarios and verifies the
  visible proposal, advisory limitation, and source-ledger interaction.
- The original video review scenario plus the two extracted playback/thread
  scenarios passed 15/15 across five repetitions.
- The messaging and welcome-modal animation-race corrections each passed 5/5
  repetitions.
- Aggregate `just ci` and a fresh E2E build pass.
- The uninterrupted full smoke run was not green: 697 passed, 1 skipped, and 2
  failed in 17.5 minutes. The corrected Daily Brief, video, messaging-avatar,
  and welcome-modal scenarios passed in that aggregate run.

The inherited video-review scenario had three unrelated responsibilities in
one 543-line test. Its live-message assertion expected an offscreen virtualized
row even though the product correctly exposed `1 new message`; the assertion
now targets that durable user-visible signal. Playback synchronization and
thread comment retention were moved, not deleted, into focused tests. Atomic
video pause and current-DOM menu clicks avoid animation-detachment races while
retaining the same product behavior.

The messaging test now waits for the avatar editor's animation before filling
and requires the real `Profile saved` toast. The welcome-modal screenshot test
waits for menu animation before clicking the current custom-model option.
`ProfileAvatarEditor.tsx` has no correction diff.

The two late aggregate failures are not presented as Task 9 acceptance:

- `supported link previews keep the message link visible` observed a detached
  optimistic card during a one-shot computed-style measurement. It passed
  10/10 focused repeats; the exact test, CSS helper, and production link-preview
  component are unchanged from base `7cbab960`. It needs a tracked baseline
  issue and independent waiver-or-fix disposition.
- `failed initial relay dial retries automatically` failed 5/10 focused repeats
  because its poll threw before the E2E relay-state seam was installed. The
  test was unchanged from base. A narrow wait-for-seam correction passed 20/20
  focused repeats.

No second 17-minute full run was started after that narrow correction. Phase 4
approval therefore remains open even though all Task 9-specific gates and the
aggregate CI gate are green.

## Baseline debt `PHASE4-SMOKE-001`

Tracking surface: draft PR #4. GitHub issue creation is unavailable, so this
named committed runbook/report entry is authoritative until it can be moved to
an issue.

Title: `Desktop smoke: link-preview style assertion races optimistic row replacement`

Evidence:

- Base comparison: the scenario at `messaging.spec.ts:262`, the
  `expectCornerRadiusPx` helper, and `link-preview-attachment.tsx` are unchanged
  from Phase 4 base `7cbab960`.
- Aggregate symptom: the card was visible, but `getComputedStyle` returned an
  empty radius from a detached element that retained the expected
  `rounded-2xl` class; the run recorded 697 passed, 1 skipped, and 2 failed.
- Focused reproduction after the aggregate run: 10/10 passed, so the failure
  is aggregate-only and not yet deterministically reproduced.
- Safety disposition: do not weaken the 16px or smooth-corner assertions and
  do not call the full smoke gate green. Investigate optimistic message-row
  replacement or make the shared style measurement poll the current locator,
  then require RED/GREEN repetition and a fresh aggregate run.

The relay seam race does not need the same waiver: it reproduced 5/10, had a
specific test synchronization defect, and passed 20/20 after waiting for the
seam before polling connection state.

Closure criteria:

1. deterministically reproduce the optimistic-card detachment or establish its
   exact aggregate trigger;
2. correct the row-replacement or current-locator style-measurement boundary
   without weakening the visual assertions;
3. demonstrate focused RED/GREEN repetition;
4. complete a fresh green full desktop smoke run; and
5. obtain independent-review acceptance.

No acceptance waiver has been granted.

## Controlled live/offline limitation

The live exercise was not run. A fresh prerequisite check found no Memory
listener on port 18006, no RAG listener on port 8005, and no
`BUZZ_DAILY_BRIEF_*` configuration. LM Studio was listening on wildcard
`*:1234`, not loopback-only, which independently prevents recording the
controlled OFFICIAL exercise as passed. No network interface was disabled.
Resource figures remain explicitly `unmeasured`; no values were inferred.

The live runner requires explicit loopback service endpoints, model, and an
absolute non-symlink driver before it will run. This prevents hermetic fixture
proof from being presented as a live/offline acceptance result. Its LM Studio
probe is truthfully labelled tool-free. The reviewed driver, not the shell
runner, owns structured MCP, five-specialist, signed-app, packet-filter,
resource, and signed-history evidence. Even a zero driver exit requires
operator inspection of retained evidence.

## Memory MCP

- `01KYC3P27YP3SPK9RSMQF3KTQP` — authoritative non-zero runner gates,
  truthful live-driver boundary, and current live blockers.
- `01KYC3P9RNZSXPXNCAV7KJF8AC` — visible evidence-boundary E2E and citation
  navigation fix.
- `01KYC3PH6SR9ZX6HVKAMCV7QDB` — exact smoke result, base attribution, relay
  synchronization correction, and no-waiver disposition.
