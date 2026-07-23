# Desktop E2E instructions

Read [`../../AGENTS.md`](../../AGENTS.md) before changing Playwright tests.
For generated and posted PR screenshots, use
[`../../../docs/pr-screenshots.md`](../../../docs/pr-screenshots.md).

- Install the mock Tauri bridge with `installMockBridge(page)`. If a test seeds
  browser state with `page.addInitScript`, add that script before installing the
  bridge so React sees it on mount.
- Wait for a mock live subscription before injecting a live event; navigate to
  the channel first when the subscription is created by the view.
- Before every direct Playwright screenshot, await
  `waitForAnimations(page)` from `../helpers/animations`. The screenshot helper
  already does this.
- Scope screenshots to the intended element or clip. When multiple images are
  meant to show distinct states, hash them and fix identical captures before
  posting.
- Local Playwright configuration may reuse an existing preview server. Rebuild
  or stop a stale preview when a run appears to serve old code.
