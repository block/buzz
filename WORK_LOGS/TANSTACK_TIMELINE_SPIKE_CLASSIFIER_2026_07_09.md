---
title: "TanStack Timeline Spike Classifier Result"
tags: [buzz-gui, performance, virtualized-timeline, tanstack, classifier]
status: active
created: 2026-07-09
---

# TanStack Timeline Spike Classifier Result

Lane B worktree: `/Users/tlongwell/.buzz-dev/REPOS/buzz-gui-perf-max-tanstack`
Branch: `max/tanstack-timeline-spike`
Base: `e0f76b0e9cbd1c84f1ed064f120bc38ab7006d46`
Fixture source: `sami/portable-classifier` at `5e5bd4a578ec2bca56f113f036776a4cb1b026b9` copied into the worktree as `desktop/tests/e2e/upscroll-portable-classify.perf.ts` plus `desktop/portable-classifier.pwconfig.ts`.

## Spike shape

The spike uses `@tanstack/react-virtual` for windowing/range math only:

- `TimelineMessageList` flattens timeline items including day dividers into a virtual row array.
- `useVirtualizer` estimates each row via the existing row-height estimator (`estimateTimelineItemHeight`).
- Rows are rendered absolute/transform-positioned inside a total-height spacer.
- `scrollToFn` is intentionally a no-op so TanStack cannot write `scrollTop`/`scrollBy`; the lane thesis was “range math only, no new scroll writer.”

## Validation before classifier

Earlier in this worktree:

- `pnpm --filter desktop typecheck` passed.
- `pnpm --filter desktop build` passed.

Classifier runs:

- Fresh baseline worktree: `/Users/tlongwell/.buzz-dev/REPOS/buzz-gui-perf-max-baseline-e0`
- Baseline build: `pnpm --filter buzz build` passed with existing Vite dynamic-import/chunk warnings.
- Candidate build: `pnpm --filter buzz build` passed with existing Vite dynamic-import/chunk warnings.
- Baseline classifier command: `cd desktop && pnpm exec playwright test -c portable-classifier.pwconfig.ts --project=chromium --project=webkit`
- Candidate classifier command: same command in the TanStack worktree.
- Repeat candidate WebKit command: fresh `pnpm --filter buzz build`, then `cd desktop && pnpm exec playwright test -c portable-classifier.pwconfig.ts --project=webkit`.

## Results

| Engine | Build | Reversals | Still | Max px | Reflow | Tracking | Mid-momentum | Post-momentum bite | Bite max px |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Chromium | baseline `e0f76b0e` | 24 | 24 | 60.0 | 0 | 24 | 0 | 24 | 60.0 |
| Chromium | TanStack spike | 21 | 21 | 60.0 | 0 | 21 | 0 | 21 | 60.0 |
| WebKit | baseline `e0f76b0e` | 25 | 25 | 60.0 | 0 | 25 | 0 | 25 | 60.0 |
| WebKit | TanStack spike run 1 | 24 | 21 | 60.0 | 0 | 24 | 3 | 21 | 60.0 |
| WebKit | TanStack spike repeat | 21 | 21 | 60.0 | 0 | 21 | 0 | 21 | 60.0 |

Artifacts on this machine:

- Baseline full output: `/tmp/max-baseline-classifier.log`
- Candidate full output: `/tmp/max-tanstack-classifier.log`
- Candidate WebKit repeat: `/tmp/max-tanstack-webkit-repeat.log`

## Verdict

Not a PR candidate in current form, but the initial mid-momentum kill did **not** reproduce.

Reasoning:

- Eva asked for one WebKit repeat because the first run's mid-momentum count was only `3`. The repeat landed `0`, so the “instant-out” is not stable enough to be the final kill cause.
- Reading the discriminator instead: WebKit post-momentum bite count is stable at `21` vs the same-session baseline `25`, but the bite-set max remains `60.0px`. Chromium also moves only `24 → 21` with bite max still `60.0px`.
- The pre-registered co-gate requires bite count **and** bite max-px both down. The TanStack range-math-only adapter does not pass that gate.
- Eva also flagged a possible fixture bite floor because all lanes keep landing near `21–26` bites with `60.0px` max. If Sami's null-control probe proves a floor, this lane should be read as “does not beat the floor,” not as evidence that TanStack uniquely causes the fixed 60px bite.

## Open caveats / why not continue adapting now

The spike remains intentionally rough and not PR-ready:

- `useAnchoredScroll` is still DOM/querySelector-based and unaware of offscreen unmounted rows.
- Jump-to-message/search/deep-link index semantics are not implemented.
- Prepend anchoring is not virtualizer-owned.
- Day-divider/group semantics are flattened and may differ from the current grouped DOM.
- `useTanStackTimeline = true` is hardcoded and must not ship.

Because the classifier did not pass the co-gated discriminator, I did not spend more time polishing these seams for a judged PR candidate. If the team later wants a deeper TanStack lane, the next variant should defer or freeze measurement/transform reconciliation during active scroll and implement index-model seams before judged acceptance.
