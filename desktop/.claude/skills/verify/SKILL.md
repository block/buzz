---
name: verify
description: Drive the desktop app end-to-end with Playwright and capture screenshot evidence that a change works
---

# Verifying desktop changes

The desktop app only renders with the E2E mock bridge — it cannot run in a
plain browser, and there is no headless Tauri. Drive the built frontend with
Playwright and read the screenshots.

## Recipe

1. Build, killing any stale preview server first (`reuseExistingServer: true`
   would keep serving the previous build's code):

   ```bash
   cd desktop
   lsof -ti :4173 | xargs kill -9 2>/dev/null; pnpm build
   ```

2. Write a throwaway driver spec — screenshots, no assertions — as
   `desktop/tests/e2e/<name>.driver.ts`. Every drive starts with
   `installMockBridge(page, mockOptions, seedOptions)` from
   `tests/helpers/bridge.ts`. Useful options: `skipCommunitySeed` boots into
   the first-run welcome screen, `skipOnboardingSeed` keeps onboarding
   incomplete, `profileReadDelayMs` holds loading gates on screen long enough
   to capture them.

3. CI specs must be listed in `playwright.config.ts` `testMatch`, but a
   driver file should not be — run it via a throwaway config that copies the
   main config's `webServer` block and sets
   `testMatch: ["**/<name>.driver.ts"]`:

   ```bash
   pnpm exec playwright test -c verify.driver.config.ts
   ```

4. Read the PNGs to confirm what the user actually sees. Playwright clears
   `test-results/` at the start of every run, so capture all frames in one
   run.

5. Delete the driver spec and throwaway config before committing.

Mock identities live in `TEST_IDENTITIES` (`tests/helpers/bridge.ts`): alice
has a relay profile so onboarding auto-completes after importing her key;
tyler with `username: ""` stays in onboarding.
