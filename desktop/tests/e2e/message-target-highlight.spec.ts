import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// A message target (deep link, link click, jump) must flash wherever the row
// starts relative to the viewport. The virtualized timeline scrolls by
// different strategies for each starting position; the flash must not depend
// on which one ran.

const HIGHLIGHT_CLASS = "route-target-highlight-fade";

type TargetCase = {
  name: string;
  /** Navigate from another channel, as a link click from elsewhere does. */
  fromOtherChannel: boolean;
  prependCount: number;
  pick: (ids: { mounted: string[]; prepended: string[] }) => string;
};

const CASES: TargetCase[] = [
  {
    name: "same channel, row on screen",
    fromOtherChannel: false,
    prependCount: 50,
    pick: ({ mounted }) => mounted[mounted.length - 3],
  },
  {
    name: "same channel, row loaded off screen",
    fromOtherChannel: false,
    prependCount: 50,
    pick: ({ mounted }) => mounted[2],
  },
  {
    name: "same channel, row in unloaded older history",
    fromOtherChannel: false,
    prependCount: 600,
    pick: ({ prepended }) => prepended[Math.floor(prepended.length / 8)],
  },
  {
    name: "other channel, row near the bottom",
    fromOtherChannel: true,
    prependCount: 50,
    pick: ({ mounted }) => mounted[mounted.length - 3],
  },
  {
    name: "other channel, row further up",
    fromOtherChannel: true,
    prependCount: 50,
    pick: ({ mounted }) => mounted[2],
  },
];

async function seedGeneral(page: Page, prependCount: number) {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );
  return page.evaluate((count) => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `visible current ${index}\nsecond line ${index}`,
        createdAt: 1_700_000_000 + index,
      });
    }
    const events = window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count,
      lineCount: 3,
    });
    return (events ?? []).map((event) => event.id);
  }, prependCount);
}

for (const targetCase of CASES) {
  test(`message target flashes: ${targetCase.name}`, async ({ page }) => {
    const prepended = await seedGeneral(page, targetCase.prependCount);

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline).toContainText("visible current 39");
    // The second bottom-pin pass runs one frame after mount; let it land so
    // "on screen" and "off screen" are measured against the settled view.
    await page.evaluate(
      () =>
        new Promise((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(resolve)),
        ),
    );
    const mounted = await timeline.evaluate((el) =>
      [...el.querySelectorAll<HTMLElement>("[data-message-id]")].map(
        (row) => row.dataset.messageId ?? "",
      ),
    );
    const targetId = targetCase.pick({ mounted, prepended });
    expect(targetId).toBeTruthy();
    const generalPath = await page.evaluate(
      () => (window.location.hash.replace(/^#/, "") || "/").split("?")[0],
    );

    if (targetCase.fromOtherChannel) {
      await page.getByTestId("channel-random").click();
      await expect(page.getByTestId("chat-title")).toHaveText("random");
    }

    // The app uses hash history, so the target lives in the hash fragment.
    // Watch from the same task that navigates so a short flash is not missed.
    const flashedOnScreen = await page.evaluate(
      ([targetId, generalPath, highlightClass]) =>
        new Promise<boolean>((resolve) => {
          const deadline = performance.now() + 3_000;
          const tick = () => {
            const timelineEl = document.querySelector(
              '[data-testid="message-timeline"]',
            );
            const row = timelineEl?.querySelector<HTMLElement>(
              `[data-message-id="${CSS.escape(targetId)}"]`,
            );
            const flashing =
              row?.closest(`[class*="${highlightClass}"]`) ??
              row?.querySelector(`[class*="${highlightClass}"]`);
            if (timelineEl && row && flashing) {
              const viewport = timelineEl.getBoundingClientRect();
              const rect = row.getBoundingClientRect();
              if (rect.bottom > viewport.top && rect.top < viewport.bottom) {
                resolve(true);
                return;
              }
            }
            if (performance.now() > deadline) {
              resolve(false);
              return;
            }
            requestAnimationFrame(tick);
          };
          const params = new URLSearchParams({ messageId: targetId });
          window.history.pushState(
            {},
            "",
            `${window.location.pathname}#${generalPath}?${params.toString()}`,
          );
          window.dispatchEvent(new HashChangeEvent("hashchange"));
          window.dispatchEvent(new PopStateEvent("popstate"));
          tick();
        }),
      [targetId, generalPath, HIGHLIGHT_CLASS] as const,
    );

    expect(flashedOnScreen).toBe(true);
  });
}
