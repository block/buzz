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

The installed app has intentionally not been opened after the upgrade. This
leaves macOS Keychain and Apple data permissions at the expected first-launch
gate for the user rather than dismissing or pre-empting them.

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
user testing shows a practical need.

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
