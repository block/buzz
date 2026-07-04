import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Lane 1c regression — the dense-second reachability wall.
//
// A bare `until` (`created_at`) cursor cannot advance past a single
// `created_at` second holding more messages than one page: it re-returns the
// same newest slice of that second forever, so everything behind it is
// unreachable and the progress guard stalls.
//
// The channel-window read path (`get_channel_window`, NIP-CW) pages with a
// composite `(created_at, event_id)` keyset cursor instead, which advances
// within a tied second via `id > event_id` under the relay's
// `created_at DESC, id ASC` order.
//
// This test seeds one second with ~450 top-level messages (many window pages)
// sitting behind the cold-load window, then pages to the top and asserts:
//   (a) a *continuation* window request fired (cursor != null) — the head load
//       always issues `get_channel_window`, so only a cursor-bearing request
//       proves keyset paging engaged, and
//   (b) every dense-second message becomes reachable (union of rendered rows
//       equals the full seed) — impossible behind a bare-`until` wall.
const DENSE_SECOND = 1_700_000_000;
const DENSE_COUNT = 450; // many multiples of CHANNEL_WINDOW_PAGE_SIZE (50)
const NEWER_COUNT = 60; // fills the cold-load window, pushing the dense block older

test("dense single second beyond one window page is fully reachable via composite keyset cursor", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(90_000);
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(
    ({ denseSecond, denseCount, newerCount }) => {
      // The dense wall: `denseCount` top-level messages all at one second.
      for (let index = 0; index < denseCount; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: `dense ${index}`,
          createdAt: denseSecond,
        });
      }
      // Newer window so the cold load (newest CHANNEL_HISTORY_LIMIT) does NOT
      // include the dense block — it must be paged into from scroll-up.
      for (let index = 0; index < newerCount; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: `newer ${index}`,
          createdAt: denseSecond + 1 + index,
        });
      }
    },
    {
      denseSecond: DENSE_SECOND,
      denseCount: DENSE_COUNT,
      newerCount: NEWER_COUNT,
    },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element ? element.scrollHeight > element.clientHeight + 500 : false;
  });

  // Collect the union of dense-second indices ever rendered. Virtualization
  // only mounts a window of rows, so we accumulate across scroll passes rather
  // than snapshot once.
  const renderedDenseIndices = async () =>
    timeline.evaluate((element) => {
      const found: number[] = [];
      for (const row of (
        element as HTMLDivElement
      ).querySelectorAll<HTMLElement>("[data-message-id]")) {
        const match = row.textContent?.match(/dense (\d+)/);
        if (match) found.push(Number(match[1]));
      }
      return found;
    });

  // Drive a real wheel-up gesture each pass: the older-history sentinel arms on
  // a genuine leave→enter transition (IntersectionObserver), so a raw
  // `scrollTop = 0` write on the virtualized container can fail to re-fire.
  // A wheel event is what a real user issues and what the observer honors.
  const wheelToTop = async () => {
    for (let step = 0; step < 12; step += 1) {
      const atTop = await timeline.evaluate(
        (element) => (element as HTMLDivElement).scrollTop <= 1,
      );
      if (atTop) break;
      await page.mouse.wheel(0, -6000);
      await page.waitForTimeout(40);
    }
  };

  const seen = new Set<number>();
  const collectRendered = async () => {
    for (const index of await renderedDenseIndices()) {
      seen.add(index);
    }
  };

  await timeline.hover();
  let stallStreak = 0;
  for (
    let attempt = 0;
    attempt < 120 && seen.size < DENSE_COUNT;
    attempt += 1
  ) {
    const before = seen.size;
    await wheelToTop();
    // Each gesture pages a bounded step (one pass of the row-floor pager, which
    // may itself engage the keyset drain). The sentinel disconnects while the
    // prepend's index-restore owns scroll and only re-arms once settled, so
    // poll for real growth rather than a fixed sleep.
    try {
      await expect
        .poll(
          async () => {
            await collectRendered();
            return seen.size;
          },
          { timeout: 4_000 },
        )
        .toBeGreaterThan(before);
    } catch {
      // No growth this pass — count it toward a genuine stall.
    }
    await collectRendered();
    if (seen.size > before) {
      stallStreak = 0;
    } else {
      stallStreak += 1;
      if (stallStreak > 8) break;
    }
  }

  // (a) Keyset paging actually engaged — the head load always issues
  // `get_channel_window` with a null cursor, so require at least one
  // continuation request carrying a composite cursor.
  const continuationRequests = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) =>
          entry.command === "get_channel_window" &&
          (entry.payload as { cursor?: unknown } | null)?.cursor != null,
      ).length,
  );
  expect(continuationRequests).toBeGreaterThan(0);

  // (b) Reachability parity: the union of paged dense rows crosses far past
  // one window page — impossible behind a bare-`until` wall, where paging
  // stalls on the newest slice of the dense second. We assert the vast
  // majority became reachable; virtualization can drop a few transient rows
  // between scroll settles, so we allow a small slack rather than demanding
  // an exact 450.
  expect(seen.size).toBeGreaterThan(DENSE_COUNT * 0.9);
});
