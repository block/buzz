import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Three invariants for the anchored-scroll redesign:
 *
 * 1. **No jump.** When older history prepends while the user is reading,
 *    the message under the user's eye stays at the same viewport y ± 2px.
 * 2. **No blank.** During the in-flight history fetch, the rendered
 *    message count never drops below what was visible before.
 * 3. **Bottom stick.** When at the bottom and a new message arrives, the
 *    timeline follows it.
 */

async function getTimelineMetrics(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const t = element as HTMLDivElement;
    return {
      clientHeight: t.clientHeight,
      scrollHeight: t.scrollHeight,
      scrollTop: t.scrollTop,
    };
  });
}

async function getMessageCount(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-timeline")
    .evaluate(
      (el) =>
        (el as HTMLDivElement).querySelectorAll("[data-message-id]").length,
    );
}

async function pickVisibleAnchorMessage(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const t = element as HTMLDivElement;
    const tRect = t.getBoundingClientRect();
    const messages = Array.from(
      t.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    // Pick the top-most row whose bottom edge has crossed the viewport top —
    // this is the exact row `useAnchoredScroll.recomputeAnchor` selects, so
    // the ±2px assertion below is testing what the algorithm actually pinned.
    for (const m of messages) {
      const r = m.getBoundingClientRect();
      if (r.bottom > tRect.top) {
        return {
          id: m.dataset.messageId ?? "",
          topInContainer: r.top - tRect.top,
        };
      }
    }
    return null;
  });
}

async function getMessageTopInContainer(
  page: import("@playwright/test").Page,
  id: string,
) {
  return page.getByTestId("message-timeline").evaluate((element, mid) => {
    const t = element as HTMLDivElement;
    const m = t.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(mid)}"]`,
    );
    if (!m) return null;
    return m.getBoundingClientRect().top - t.getBoundingClientRect().top;
  }, id);
}

async function waitForBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );
}

test.describe("anchored scroll", () => {
  test("no jump: pinned message stays put while older history loads", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await waitForBridge(page);

    // Seed the channel with enough current + older content that the timeline
    // is scrollable and there are older pages to fetch.
    await page.evaluate(() => {
      for (let i = 0; i < 40; i++) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: `live ${i}\nline two ${i}`,
        });
      }
      window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
        channelName: "general",
        count: 300,
        lineCount: 3,
      });
    });

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline).toContainText("live 39");

    // Wait until the timeline actually has scrollable content.
    await page.waitForFunction(() => {
      const el = document.querySelector(
        '[data-testid="message-timeline"]',
      ) as HTMLDivElement | null;
      return el !== null && el.scrollHeight > el.clientHeight + 800;
    });

    // Delay the next history request so we can observe the in-flight window.
    await page.evaluate(() => {
      window.__BUZZ_E2E__ = {
        ...window.__BUZZ_E2E__,
        mock: { ...window.__BUZZ_E2E__?.mock, historyDelayMs: 1_500 },
      };
    });

    // Scroll up until we're close to the IntersectionObserver root margin
    // (400px) but still *outside* it. We want to capture the anchor before
    // the IO fires and the loading spinner mounts, because the spinner adds
    // ~40px of height that would change the row's apparent topOffset.
    const box = await timeline.boundingBox();
    expect(box).not.toBeNull();
    await page.mouse.move(
      (box?.x ?? 0) + (box?.width ?? 0) / 2,
      (box?.y ?? 0) + (box?.height ?? 0) / 2,
    );
    for (let i = 0; i < 100; i++) {
      await page.mouse.wheel(0, -400);
      await page.waitForTimeout(20);
      const m = await getTimelineMetrics(page);
      if (m.scrollTop <= 800) break;
    }

    const beforeCount = await getMessageCount(page);

    // Now scroll into the IO root margin to trigger the fetch.
    await page.mouse.wheel(0, -400);
    // Wait for the loading spinner to mount and the browser's scroll-anchor
    // to settle the post-mount position. The hook (in the IO callback) and
    // the test both need to read the SAME post-spinner DOM state, otherwise
    // the spinner's ~40px height adds spurious delta.
    await page.waitForTimeout(100);

    // Capture the anchor row's current position. This is the position the
    // hook just pinned to inside its IO callback, so a successful restore
    // after prepend will put the row exactly here.
    const anchorBefore = await pickVisibleAnchorMessage(page);
    expect(anchorBefore).not.toBeNull();

    // During the fetch the message count must never drop below what we had.
    // Poll a handful of times while the fetch is in-flight.
    const samples: number[] = [];
    for (let i = 0; i < 8; i++) {
      samples.push(await getMessageCount(page));
      await page.waitForTimeout(150);
    }
    expect(Math.min(...samples)).toBeGreaterThanOrEqual(beforeCount);

    // After the fetch completes the anchor message must be within ±2px of
    // where it was. This is the "no jump" invariant.
    await page.waitForFunction((expected) => {
      const el = document.querySelector(
        '[data-testid="message-timeline"]',
      ) as HTMLDivElement | null;
      if (!el) return false;
      return (
        el.querySelectorAll("[data-message-id]").length > (expected as number)
      );
    }, beforeCount);

    const anchorTopAfter = await getMessageTopInContainer(
      page,
      anchorBefore?.id ?? "",
    );
    expect(anchorTopAfter).not.toBeNull();
    if (anchorTopAfter === null || anchorBefore === null) {
      throw new Error("anchor invariants violated");
    }
    expect(
      Math.abs(anchorTopAfter - anchorBefore.topInContainer),
    ).toBeLessThanOrEqual(2);
  });

  test("bottom stick: new messages scroll us when we're at the bottom", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await waitForBridge(page);

    await page.evaluate(() => {
      for (let i = 0; i < 20; i++) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: `seed ${i}`,
        });
      }
    });

    await page.getByTestId("channel-general").click();
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline).toContainText("seed 19");

    // Confirm we land at the bottom.
    const before = await getTimelineMetrics(page);
    expect(
      before.scrollHeight - before.scrollTop - before.clientHeight,
    ).toBeLessThanOrEqual(24);

    // Inject a new message and assert we follow it.
    await page.evaluate(() => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "freshly arrived live message",
      });
    });
    await expect(timeline).toContainText("freshly arrived live message");
    await page.waitForTimeout(100);

    const after = await getTimelineMetrics(page);
    expect(
      after.scrollHeight - after.scrollTop - after.clientHeight,
    ).toBeLessThanOrEqual(24);
  });
});
