# Design-system validation

Use the repository Hermit environment. The catalog runs independently of Tauri,
so it needs no mock native bridge or live relay.

- `pnpm check` checks formatting, both source layers' typography, both modes'
  APCA text pairings, essential-control boundary contrast, generated CSS freshness,
  compiled Tailwind role mappings, and generated-response validation/rendering.
- `pnpm test:e2e` builds fresh production output, starts a dedicated preview server
  on 5189, and runs Chromium. It refuses to reuse an existing server.
- `pnpm exec playwright test` reuses an already-built `dist` for a focused rerun.
  Rebuild after any source change. `pnpm exec playwright test --grep <name>` narrows
  a regression without replacing the full final pass.

Browser coverage binds the actual production catalog: all 55 examples, axe ARIA
checks in both modes, no external runtime requests, preserved loading labels,
checkbox and switch keyboard activation, dialog focus trap and return, selection,
command dispatch, form validation, multiline input, tabs, manual carousel,
keyboard resizing, filtering recovery, generated-response actions/streaming/error
recovery, reduced motion, and overflow at 390/768/1440px.

Axe's WCAG text-color rule is disabled because DESIGN.md deliberately uses APCA;
`check:contrast` is the required independent contrast gate. Automated accessibility
checks supplement manual keyboard and visual inspection; they do not certify every
screen-reader/browser combination.

Screenshots are generated under `test-results/`. Every explicit capture uses the
repository's `waitForAnimations` helper. Overview light/dark and composed examples
are different states; compare hashes before including images in a PR. Use the root
`scripts/post-screenshots.sh` workflow for PR image hosting. Do not commit reports,
traces, or screenshots to the source branch.

Spacing and motion regressions exercise command empty-state recovery, dialog heading
gaps, accordion density, toast entrance/exit, interrupted tab selection, and reduced
motion in the production catalog. Animation checks stretch duration for frame
inspection without adding transition properties that could mask missing motion.

Layout coverage verifies table alignment, native/custom scrollbar treatment, actual
scrolling, pointer and keyboard resizing, grip sizing under zoom, and shared Glass
tab motion, keyboard selection, and reduced motion.

Composer coverage exercises empty drafts, multiline and modifier handling, IME
composition, pointer/keyboard send and stop, retained failed drafts and retry,
attachment removal and limits, model controls, transcript preview, and 390/884/1440
layouts. Card header spacing is checked against the production component.

Density coverage checks pointer/keyboard switching, root and portal geometry,
unchanged text size, retained drafts, reload and cross-tab persistence, invalid
stored values, save failure and retry, live foundation values, compositions,
Glass tab selection, and all 55 examples at 390/884/1440 in both themes.
