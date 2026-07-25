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
  scenarios passed 30/30 across ten repetitions.
- The messaging and welcome-modal animation-race corrections each passed 5/5
  repetitions.
- Aggregate `just ci` and a fresh E2E build pass.
- The first uninterrupted full smoke run was not green: 697 passed, 1 skipped,
  and 2 failed in 17.5 minutes. After deterministic diagnosis and narrow
  synchronization corrections, the replacement full smoke run passed with
  700 passed, 1 skipped, and 0 failed in 17.9 minutes.

The inherited video-review scenario had three unrelated responsibilities in
one 543-line test. Its live-message assertion expected an offscreen virtualized
row even though the product correctly exposed `1 new message`; the assertion
now targets that durable user-visible signal. Playback synchronization and
thread comment retention were moved, not deleted, into focused tests. The
review pause assertion now activates the visible current-DOM
`Pause review video` control through Playwright after settling pre-play
animations, then proves both native paused state and the visible
`Play review video` state. A targeted mutation that disabled only the
production pause branch left the control visible and clickable but failed the
native paused-state assertion, proving the test does not bypass the UI
contract.

The messaging test now waits for the avatar editor's animation before filling
and requires the real `Profile saved` toast. The welcome-modal screenshot test
waits for menu animation before clicking the current custom-model option.
`ProfileAvatarEditor.tsx` has no correction diff.

The two late failures in the first corrected aggregate were resolved without
weakening their assertions:

- `supported link previews keep the message link visible` observed a detached
  optimistic card during a one-shot computed-style measurement. A deterministic
  regression replaced the located card during style lookup and reproduced the
  exact 16px-versus-0px failure. The shared helper now polls and re-resolves the
  locator. The real scenario plus regression passed 40/40 under parallel
  repetition, and all 57 helper consumers passed before the aggregate.
- `failed initial relay dial retries automatically` failed 5/10 focused repeats
  because its poll threw before the E2E relay-state seam was installed. A
  narrow wait-for-seam correction passed 20/20 focused repeats.

The first replacement aggregate exposed two further readiness races while the
original link-preview scenario passed: Enter was sent before the channel
browser's deferred result reflected its query, and a prior create dialog's exit
node satisfied the next open helper. The tests now gate on the visible create
row containing the live query and on the old dialog detaching before reopening.
Those scenarios passed 100/100 under parallel repetition. The final full smoke
run passed with 700 passed, 1 skipped, and 0 failed in 17.9 minutes.

The first final `just ci` run then reproduced a rare model-observer lifecycle
test race: 1866 Tauri tests passed and one failed because the test sampled its
poll counter immediately after synchronous cancellation but before the spawned
observer task had exited. Focused repetition reproduced the old failure at run
43. The observer now retains its task handle and exposes a test-only
cancel-and-join boundary; synchronous application shutdown behavior is
unchanged. The corrected test passed 100/100 repetitions, and the replacement
`just ci` run passed, including all 541 mobile tests with one skip.

Phase 4 approval still remains open on the controlled live/offline exercise and
independent-review acceptance.

## Closed baseline debt `PHASE4-SMOKE-001`

Tracking surface: draft PR #4. GitHub issue creation is unavailable, so this
named committed runbook/report entry is authoritative until it can be moved to
an issue.

Title: `Desktop smoke: link-preview style assertion races optimistic row replacement`

Closure evidence:

- Base comparison: the scenario at `messaging.spec.ts:262`, the
  `expectCornerRadiusPx` helper, and `link-preview-attachment.tsx` are unchanged
  from Phase 4 base `7cbab960`.
- Aggregate symptom: the card was visible, but `getComputedStyle` returned an
  empty radius from a detached element that retained the expected
  `rounded-2xl` class; the run recorded 697 passed, 1 skipped, and 2 failed.
- Focused reproduction after the first aggregate run passed, confirming the
  trigger required a node replacement during the measurement window.
- A deterministic regression replaced the located element during
  `getComputedStyle`. It failed before the correction with an expected 16px
  radius and measured 0px, then passed after the helper polled the current
  locator.
- The real link-preview scenario and deterministic regression passed 40/40
  under parallel repetition. All 57 current helper consumers then passed.
- The first replacement aggregate passed the link-preview case but exposed two
  unrelated readiness races. After their narrow readiness gates passed 100/100
  focused repeats, the final aggregate passed: 700 passed, 1 skipped, and 0
  failed in 17.9 minutes.
- The 16px radius and smooth-corner assertions remain unchanged.

The relay seam race does not need the same waiver: it reproduced 5/10, had a
specific test synchronization defect, and passed 20/20 after waiting for the
seam before polling connection state.

Closure criteria and disposition:

1. Deterministic reproduction: complete.
2. Current-locator correction without weakening assertions: complete.
3. Focused RED/GREEN repetition: complete.
4. Fresh green full desktop smoke run: complete.
5. Independent-review acceptance: pending as a Phase 4 gate.

No acceptance waiver was used.

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
