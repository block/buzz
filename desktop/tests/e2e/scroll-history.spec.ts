import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function getTimelineMetrics(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;

    return {
      clientHeight: timeline.clientHeight,
      scrollHeight: timeline.scrollHeight,
      scrollTop: timeline.scrollTop,
    };
  });
}

async function getFirstVisibleMessage(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;
    const timelineRect = timeline.getBoundingClientRect();
    const messages = Array.from(
      timeline.querySelectorAll<HTMLElement>("[data-message-id]"),
    );

    for (const message of messages) {
      const rect = message.getBoundingClientRect();
      if (rect.bottom <= timelineRect.top || rect.top >= timelineRect.bottom) {
        continue;
      }

      return {
        id: message.dataset.messageId ?? "",
        text: message.textContent?.replace(/\s+/g, " ").slice(0, 80) ?? "",
        top: rect.top - timelineRect.top,
      };
    }

    return null;
  });
}

async function getMessagePosition(
  page: import("@playwright/test").Page,
  messageId: string,
) {
  return page.getByTestId("message-timeline").evaluate((element, id) => {
    const timeline = element as HTMLDivElement;
    const message = timeline.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(id)}"]`,
    );
    if (!message) {
      return null;
    }

    return {
      id,
      top:
        message.getBoundingClientRect().top -
        timeline.getBoundingClientRect().top,
    };
  }, messageId);
}

test("preserves user scroll while older channel history loads", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );

  await page.evaluate(() => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `visible current ${index}\nsecond line ${index}`,
      });
    }
    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count: 250,
      lineCount: 3,
    });
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText("visible current 39");

  // Initial load should receive enough history to make the page scrollable.
  // Delay only the next history request, so the test isolates pagination while
  // the user is actively scrolling.
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: {
        ...window.__BUZZ_E2E__?.mock,
        historyDelayMs: 1_000,
      },
    };
  });

  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 1000;
  });

  // Move away from the bottom before jumping near the top; otherwise the
  // timeline's sticky-bottom guard can intentionally pin the first upward jump.
  const beforeFetch = await getTimelineMetrics(page);
  await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = timelineElement.scrollHeight;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(50);

  const nearTop = await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = 180;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
    return timelineElement.scrollTop;
  });
  expect(nearTop).toBeLessThan(260);

  await page.waitForTimeout(100);
  const duringFetch = await timeline.evaluate((element) => {
    const timelineElement = element as HTMLDivElement;
    timelineElement.scrollTop = timelineElement.scrollTop + 160;
    timelineElement.dispatchEvent(new Event("scroll", { bubbles: true }));
    return timelineElement.scrollTop;
  });
  expect(duringFetch).toBeGreaterThan(nearTop);
  const anchorDuringFetch = await getFirstVisibleMessage(page);
  expect(anchorDuringFetch).not.toBeNull();

  await expect
    .poll(
      async () => {
        const [anchor, metrics] = await Promise.all([
          getMessagePosition(page, anchorDuringFetch?.id ?? ""),
          getTimelineMetrics(page),
        ]);
        if (metrics.scrollHeight <= beforeFetch.scrollHeight + 1000) {
          return Number.POSITIVE_INFINITY;
        }
        return anchor
          ? Math.abs(anchor.top - (anchorDuringFetch?.top ?? 0))
          : Number.POSITIVE_INFINITY;
      },
      {
        timeout: 3_000,
      },
    )
    .toBeLessThanOrEqual(2);
});

const REAL_BUZZ_BUGS_IMAGE_SHA =
  "ff2862080bac3d009f97cad4bb94e6efec328eaaee058a405e854acd49fc1483";
const REAL_BUZZ_BUGS_IMAGE_URL = `https://sprout-oss.stage.blox.sqprod.co/media/${REAL_BUZZ_BUGS_IMAGE_SHA}.png`;
const REAL_BUZZ_BUGS_IMAGE_TAG = [
  "imeta",
  `url ${REAL_BUZZ_BUGS_IMAGE_URL}`,
  "m image/png",
  `x ${REAL_BUZZ_BUGS_IMAGE_SHA}`,
  "size 26257",
  "dim 951x244",
  "filename image.png",
] as string[];

test("reserves real buzz-bugs imeta image height before image loads", async ({
  page,
}) => {
  await page.route("**/media/**", () => new Promise(() => {}));
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(
    ({ content, extraTags }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags,
      });
    },
    {
      content: `this setting gets reverted on every update\n![image](${REAL_BUZZ_BUGS_IMAGE_URL})`,
      extraTags: [REAL_BUZZ_BUGS_IMAGE_TAG],
    },
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const image = page.getByAltText("image").last();
  const rect = await image.evaluate((element) => {
    const img = element as HTMLImageElement;
    const box = img.getBoundingClientRect();
    return {
      attrHeight: img.getAttribute("height"),
      attrWidth: img.getAttribute("width"),
      height: box.height,
      offsetHeight: img.offsetHeight,
      offsetWidth: img.offsetWidth,
      width: box.width,
    };
  });
  expect(rect.attrWidth).toBe("951");
  expect(rect.attrHeight).toBe("244");
  expect(rect.offsetHeight).toBeGreaterThan(80);
});
