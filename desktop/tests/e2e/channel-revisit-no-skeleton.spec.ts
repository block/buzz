import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Regression: revisiting a channel that already settled this session must
 * paint its cached rows immediately — never a skeleton over known content.
 *
 * The timeline loading latch used to reset on every channel switch, so a
 * re-entry past the messages query's staleTime refetched on mount with
 * `isFetching: true` and the unsettled branch held the skeleton for the whole
 * relay round-trip — a full flash of skeleton over a fully cached timeline.
 * Session-settled channels now render stale-while-revalidate.
 *
 * Staleness is forced by shifting Date.now() (React Query's staleness clock)
 * past the 5-minute window; the window fetch is delayed so the would-be flash
 * window is wide enough to observe deterministically.
 */

const WINDOW_FETCH_DELAY_MS = 600;
const STALENESS_JUMP_MS = 6 * 60_000;

test("revisiting a settled channel paints cached rows, not a skeleton", async ({
  page,
}) => {
  // Date.now offset shim — must install before the app boots.
  await page.addInitScript(() => {
    const realNow = Date.now.bind(Date);
    (window as unknown as { __TIME_OFFSET_MS__: number }).__TIME_OFFSET_MS__ =
      0;
    Date.now = () =>
      realNow() +
      (window as unknown as { __TIME_OFFSET_MS__: number }).__TIME_OFFSET_MS__;
  });
  await installMockBridge(page, {
    // Head (cursorless) fetches are the revisit refetch path; delaying them
    // creates the observation window this spec asserts over.
    channelWindowHeadDelayMs: WINDOW_FETCH_DELAY_MS,
  });
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // First visit: general settles (cold load may show the skeleton — fine).
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.locator("[data-message-id]").first()).toBeVisible({
    timeout: 15_000,
  });

  // Leave for another channel.
  await page.getByTestId("channel-deep-history").click();
  await expect(page.getByTestId("chat-title")).toHaveText("deep-history");
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible({ timeout: 15_000 });

  // Everything general cached is now stale.
  await page.evaluate((jump) => {
    (window as unknown as { __TIME_OFFSET_MS__: number }).__TIME_OFFSET_MS__ =
      jump;
  }, STALENESS_JUMP_MS);

  // Revisit: sample the surface every frame through the refetch window. The
  // cached rows must be present the whole time and the skeleton must never
  // mount.
  const result = await page.evaluate(async (windowDelay) => {
    const link = document.querySelector<HTMLElement>(
      '[data-testid="channel-general"]',
    );
    if (!link) throw new Error("missing sidebar link");
    const samples: Array<{ skeleton: boolean; rows: number }> = [];
    link.click();
    const deadline = performance.now() + windowDelay + 400;
    await new Promise<void>((resolve) => {
      const tick = () => {
        samples.push({
          skeleton:
            document.querySelector('[data-testid="timeline-skeleton-row"]') !==
            null,
          rows: document.querySelectorAll("[data-message-id]").length,
        });
        if (performance.now() > deadline) {
          resolve();
          return;
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });
    return {
      sampleCount: samples.length,
      skeletonFrames: samples.filter((sample) => sample.skeleton).length,
      rowlessFrames: samples.filter((sample) => sample.rows === 0).length,
    };
  }, WINDOW_FETCH_DELAY_MS);

  expect(result.sampleCount).toBeGreaterThan(20);
  expect(
    result.skeletonFrames,
    "skeleton mounted over cached rows during the stale-revisit refetch",
  ).toBe(0);
  // The deferred-snapshot switch gap may render blank background for a couple
  // of frames; anything longer means cached rows failed to paint.
  expect(
    result.rowlessFrames,
    "cached rows stayed absent past the switch gap during the stale-revisit refetch",
  ).toBeLessThanOrEqual(4);
});
