# Battle Rhythm Acceptance Record

Date: 29 July 2026

## Automated user journey

The desktop E2E bridge verifies the complete supported workflow:

- open Battle Rhythm from the primary sidebar;
- use year, month, week, and day views;
- create and edit a recurring manual event with an excluded occurrence;
- import a Shortcast after reviewing the proposed change set;
- revise one imported FAS source without changing a manual event;
- publish the resulting schedule one way to the dedicated Apple calendar;
- inspect the immutable source-revision history; and
- review and apply a rollback as a new signed revision.

The acceptance test also verifies that a source rollback restores that source's
events, removes the superseded source events, and leaves manually entered
events untouched.

## Visual evidence

`battle-rhythm-screenshots.spec.ts` captures scoped year, month, week, day, and
import-review views under `desktop/test-results/battle-rhythm-acceptance`.
The test hashes all five images and fails if any two captures are identical.

## Native boundary

Swift and Rust tests verify the EventKit request, dedicated-calendar ownership,
stable external identifiers, authoritative-coverage deletion, and fail-soft
permission response. A final signed-app check on the user's macOS profile is
still required to exercise the real Calendar privacy prompt and visually
confirm the dedicated `HMAS Supply Battle Rhythm` calendar.
