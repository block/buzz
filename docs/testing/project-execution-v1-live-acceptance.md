# Command Adviser Project Execution V1 Acceptance

**Date:** 29 July 2026  
**Branch:** `codex/project-execution-v1`  
**Feature commit:** `bb56139d`  
**Draft PR:** [NavigatorRAN/buzz#13](https://github.com/NavigatorRAN/buzz/pull/13)

## Delivered user journey

Command Adviser now provides:

- Apple-style Day, Week, Month, and Year Battle Rhythm views with 24-hour Ship
  Time and `Australia/Sydney` as the default;
- explicit local adjustments for imported FAS, Longcast, and Shortcast events,
  so the source revision remains intact;
- a signed Kanban and Gantt planning surface with Department/HOD, position,
  named-individual, and AI-agent assignments;
- MEO, WEEO, SO, XO, and combined HOD Sync Pack PDF generation;
- a text-first Pre-Departure playbook and routine-aware scheduling for
  Alongside, At Sea, and Sunday Sea;
- dependency-aware task movement with preview/cancel/apply and critical-path
  recalculation;
- manual or hybrid AI task execution, with one-hour-prior scheduling,
  claim-before-run duplicate protection, catch-up after wake, RAG/Memory
  evidence, and visible missing-input warnings;
- linked DOCX, PPTX, XLSX, and PDF artefacts, written to iCloud Drive when
  available and a local folder otherwise; and
- release-only start-at-login support using the macOS LaunchAgent backend.

## Automated evidence

- Focused TypeScript tests for calendar, routines, playbooks, task due times,
  local adjustments, and rescheduling: passed.
- Desktop type/lint/format gate (`pnpm check`): passed.
- Desktop production and E2E builds: passed.
- Native project-execution tests: 4 passed.
- Combined Playwright acceptance for Plans, Battle Rhythm, and screenshots:
  14 passed in 31.4 seconds.
- Day, Week, Month, Year, and Import screenshots were visually inspected and
  their SHA-256 hashes were distinct.
- Final desktop tests: 3,644 passed.
- Pre-push native tests: 1,936 passed, 14 ignored; diagnostics: 3 passed.
- Full repository gate (`just ci`): passed, including Rust formatting and
  Clippy, desktop checks/build/tests, web checks/build, mobile analysis/tests,
  native tests, and repository unit gates.

## Build and installation evidence

- Application:
  `/Applications/Command Adviser.app`
- Recoverable previous application:
  `/Applications/Command Adviser.before-project-execution-v1-20260729-203737.app`
- Built application:
  `desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Command Adviser.app`
- DMG:
  `desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Command Adviser_0.4.24_aarch64.dmg`
- DMG SHA-256:
  `e59386bfa92e12566a25b8dca70479eccd3411f03259e913a671ce4d59f6534c`
- Bundle identity: `xyz.block.buzz.app`
- Display name: `Command Adviser`
- Version: `0.4.24`
- Ad-hoc code signature, deep strict verification, and macOS entitlement
  verification: passed for both the build output and installed application.
- Installed relay:
  `~/Library/Application Support/Command Adviser/relay/buzz-relay`
- Relay binary SHA-256:
  `9132008e5869e1e8721eb0c9fe96a08ca29dfff97bf28c00ce6619312526b284`
- Relay service:
  `~/Library/LaunchAgents/xyz.block.command-adviser.relay.plist`
- Relay restart canary: launchd replaced PID `23093` with PID `23199`; the
  readiness endpoint returned `{"status":"ready"}` and relay `/health`
  returned `ok`.

At the initial project-execution checkpoint, the installed app was
intentionally left unopened so the user could pass the expected macOS
permission gates. The correction below records the subsequent live exercise.

## Accepted V1 refinements

The following items are not required for the first usable version:

- playbook duplicate/revise/retire and automatic anchor-move reflow;
- direct pointer-drag of imported Battle Rhythm source events (use the local
  adjustment editor);
- automatic iCloud retry after a local fallback; and
- deterministic virtual-clock/reload browser coverage for the scheduler.

The scheduling rules, stable execution claims, visibility/wake catch-up,
artefact storage decision, and native execution paths are covered by unit,
native, and end-to-end tests. These refinements should only be built after
live use demonstrates that they are needed.

## First user-test correction — 29 July 2026

The first live user test identified and corrected the following Battle Rhythm
issues:

- event date and time controls now use Ship Time with explicit `HH:mm`
  24-hour fields;
- a new event defaults to one hour in the future, and changing its start keeps
  the same duration while ensuring the end remains later;
- clearing or partially typing a time no longer reaches the React error
  boundary;
- title, location, owner, and remarks fields enable Australian English
  spellchecking and autocorrection;
- recurrence controls only appear when recurrence is enabled, and the Until
  date and time fit within the editor;
- multi-day events use overlap semantics in Day, Week, Month, and Year views;
- an all-day range selected in the editor includes every selected calendar day;
  and
- range headings include the complete day, month, and year.

The installed application was exercised against the user's real persisted SMP
event. It appeared on all seven days of both 15–21 and 22–28 February 2027.
The final installed editor opened at the current Australia/Sydney time with an
end one hour later; changing `21:57` to `08:00` automatically changed the end
to `09:00` without an error. The previously hidden project-execution functions
are now advertised on the empty Plans landing page as Board and Gantt
scheduling, Operational playbooks, and HOD Sync Packs and AI outputs.

Verification after the correction:

- focused Battle Rhythm, screenshot, and Plans Playwright suites: 16 passed;
- full repository gate (`just ci`): passed;
- signed application executable SHA-256:
  `aefcab1f1bf1dbdc3c11069d3460fe09dceadd9a46dcf0d95d268bc7a28e8a6d`;
- DMG SHA-256:
  `69066078a8701d41ce984bcc45488b9f4dbaad875d3169597dcf8e1f413ed0e5`;
- recoverable pre-correction application:
  `/Applications/Command Adviser.before-partial-time-fix-20260729-2153.app`;

## Ship program colours and Week all-day lane — 29 July 2026

The ship's broad program is now classified at presentation time without
rewriting its signed events:

- an all-day location containing `Sea` as a word is blue;
- any other non-empty all-day location, including FBE and FBW, is yellow; and
- timed events and blank-location all-day events remain neutral.

Week view now renders each all-day event once in a dedicated seven-column
lane, clipped to the visible week and spanning its overlapping days. The
daily columns retain timed events and plan milestones but no longer repeat
all-day events.

Automated verification:

- Battle Rhythm domain tests: 47 passed;
- Battle Rhythm Playwright journeys: 8 passed;
- desktop format, lint, file-size, text-size, and pubkey checks: passed;
- complete desktop JavaScript test suite: passed; and
- native desktop tests: 1,936 passed, 14 ignored, plus 3 diagnostics passed.

Installed-app verification used the real persisted 2028 program and did not
create or edit an event:

- `Post Maintience Availability · FBE` appeared once as a yellow full-week bar
  for 30 October–5 November 2028; and
- `Unit Readiness Evaluation · SEA` appeared once as a blue full-week bar for
  24–30 April 2028.

Build and recovery evidence:

- application: `/Applications/Command Adviser.app`;
- recoverable prior application:
  `/Applications/Command Adviser.before-program-colours-20260729-2323.app`;
- signed application executable SHA-256:
  `5007a64e38e6522b07de4a5b8ef7f1e662399b474e07417496030e6f9894874b`;
- DMG SHA-256:
  `8c1474ba705c16f2039b2f7e3e38386b57ca82fc68d8e532d9767dca3fc22baf`;
  and
- deep signature and macOS entitlement verification: passed on both the built
  and installed application.
- installed signature, entitlements, bundle identity, executable hash, live
  launch, and relay health: passed.

## First-launch user test

1. Open **Command Adviser** from Applications and approve any expected
   Keychain or Apple Calendar prompts.
2. Open **Battle Rhythm** and verify the current Day, Week, Month, and Year
   headings, 24-hour times, event creation, and one local adjustment.
3. Open **Plans**, create a project and tasks, assign one to an HOD/department
   and one to a named individual, then move them on the Board and Gantt views.
4. Apply the **Pre-Departure** playbook to a Monday sailing and confirm
   preparatory work is scheduled on working days.
5. Generate a combined HOD Sync Pack and one adviser artefact.
6. Run an AI task once with the available dependencies and confirm missing
   inputs are shown without blocking the result.
7. Quit and reopen the app and confirm the signed project and Battle Rhythm
   records remain present.

Record any problems from this journey as user-facing defects. Do not expand
the architecture before confirming the observed workflow needs it.
