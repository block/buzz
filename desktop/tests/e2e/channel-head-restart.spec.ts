import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const PERSISTED_ONLY = "persisted restart head";

test("restart paints a persisted head before the single authoritative refresh", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate((content) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content,
    });
  }, PERSISTED_ONLY);

  await page.getByTestId("channel-general").click();
  await expect(page.getByText(PERSISTED_ONLY)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "channel_head_cache_store",
          ).length,
      ),
    )
    .toBeGreaterThan(0);

  const persistedCache = await page.evaluate(() =>
    Object.fromEntries(
      Object.entries(window.localStorage).filter(([key]) =>
        key.startsWith("buzz-e2e-channel-head:"),
      ),
    ),
  );
  expect(Object.keys(persistedCache)).toHaveLength(1);
  await page.addInitScript((cache) => {
    for (const [key, value] of Object.entries(cache)) {
      window.localStorage.setItem(key, value);
    }
    const testWindow = window as Window & {
      __BUZZ_E2E__?: { mock?: Record<string, unknown> };
    };
    testWindow.__BUZZ_E2E__ = {
      ...testWindow.__BUZZ_E2E__,
      mock: {
        ...testWindow.__BUZZ_E2E__?.mock,
        channelHeadDelayMs: 5_000,
      },
    };
  }, persistedCache);
  await page.reload();
  await expect(page.getByTestId("channel-general")).toBeVisible();
  const restartedCallsBeforeOpen = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
        (command) => command === "get_channel_window",
      ).length,
  );
  await page.getByTestId("channel-general").click();

  // The mock relay fetch is held for 5s. Seeing this row inside 2s proves
  // restart hydration painted persisted data rather than waiting for the relay.
  await expect(page.getByText(PERSISTED_ONLY)).toBeVisible({ timeout: 2_000 });
  await expect
    .poll(() =>
      page.evaluate(
        (callsBeforeOpen) =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "get_channel_window",
          ).length - callsBeforeOpen,
        restartedCallsBeforeOpen,
      ),
    )
    .toBe(1);

  // Reload resets the mock relay store, so the authoritative refresh omits the
  // persisted-only row and must replace it wholesale.
  await expect(page.getByText(PERSISTED_ONLY)).toHaveCount(0, {
    timeout: 8_000,
  });
});
