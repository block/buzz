import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// The channel and thread composers float over their conversation scrollers.
// When the conversation is scrolled up, later rows pass underneath the
// composer overlay — the overlay must mask them (gradient fade + solid
// bottom strip, mirroring the inbox treatment from PR #2143) instead of
// letting them bleed out below the composer box.

const CHANNEL = "general";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (ch) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: ch,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

async function emit(
  page: import("@playwright/test").Page,
  content: string,
  parentEventId: string | null = null,
) {
  const event = await page.evaluate(
    (payload) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: payload.channel,
        content: payload.content,
        parentEventId: payload.parentEventId,
      }),
    { channel: CHANNEL, content, parentEventId },
  );
  if (!event) throw new Error("mock message emitter is not installed");
  return event as { id: string };
}

async function expectOverlayBottomMask(
  overlay: import("@playwright/test").Locator,
) {
  const hasMask = await overlay.evaluate((element) => {
    const after = getComputedStyle(element, "::after");
    const before = getComputedStyle(element, "::before");
    const composer = element.querySelector<HTMLElement>(
      '[data-testid="message-composer"]',
    );
    const cornerMaskContainer = element.querySelector<HTMLElement>(
      ".composer-overlay-corner-masks",
    );
    if (!composer || !cornerMaskContainer) return false;

    const overlayRect = element.getBoundingClientRect();
    const composerRect = composer.getBoundingClientRect();
    const leftMask = getComputedStyle(cornerMaskContainer, "::before");
    const rightMask = getComputedStyle(cornerMaskContainer, "::after");
    const tolerance = 0.5;

    return (
      after.content !== "none" &&
      before.content !== "none" &&
      leftMask.maskImage !== "none" &&
      rightMask.maskImage !== "none" &&
      Math.abs(
        composerRect.left - overlayRect.left - Number.parseFloat(leftMask.left),
      ) <= tolerance &&
      Math.abs(
        overlayRect.right -
          composerRect.right -
          Number.parseFloat(rightMask.right),
      ) <= tolerance &&
      Math.abs(
        overlayRect.bottom -
          composerRect.bottom -
          Number.parseFloat(leftMask.bottom),
      ) <= tolerance &&
      leftMask.bottom === rightMask.bottom
    );
  });
  expect(hasMask).toBe(true);
}

test.describe("composer overlays mask scrolled content", () => {
  test("channel timeline rows fade out behind the composer", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();
    await expect(page.getByTestId("message-timeline")).toBeVisible();
    await waitForMockLiveSubscription(page, CHANNEL);

    for (let i = 0; i < 20; i++) {
      await emit(
        page,
        `Channel filler ${i} — enough text to occupy vertical space so the conversation scrolls and rows pass behind the composer overlay.`,
      );
    }
    await page.waitForTimeout(400);

    // Scroll the conversation up so trailing rows sit behind the overlay.
    await page.evaluate(() => {
      const scroller = document.querySelector<HTMLElement>(
        '[data-buzz-conversation-scroll="true"]',
      );
      if (!scroller) throw new Error("Missing conversation scroll container");
      scroller.scrollTop = Math.max(
        0,
        scroller.scrollHeight - scroller.clientHeight - 220,
      );
    });
    await page.waitForTimeout(300);
    await waitForAnimations(page);

    await page.screenshot({
      path: "test-results/channel-overflow/channel-composer.png",
      clip: { x: 300, y: 720 - 280, width: 980, height: 280 },
    });

    await expectOverlayBottomMask(page.getByTestId("channel-composer-overlay"));
  });

  test("thread panel replies fade out behind the reply composer", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();
    await expect(page.getByTestId("message-timeline")).toBeVisible();
    await waitForMockLiveSubscription(page, CHANNEL);

    const root = await emit(page, "Thread root — the discussion begins here.");
    for (let i = 0; i < 20; i++) {
      await emit(
        page,
        `Thread reply ${i} — enough text to occupy vertical space so the thread body scrolls and replies pass behind the reply composer.`,
        root.id,
      );
    }
    await page.waitForTimeout(400);

    const summary = page.locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${root.id}"]`,
    );
    await expect(summary).toBeVisible();
    await summary.click();
    const threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible();
    await waitForAnimations(page);

    // Scroll the thread body up so trailing replies sit behind the overlay.
    await page.evaluate(() => {
      const scroller = document.querySelector<HTMLElement>(
        '[data-testid="message-thread-body"]',
      );
      if (!scroller) throw new Error("Missing thread body scroll container");
      scroller.scrollTop = Math.max(
        0,
        scroller.scrollHeight - scroller.clientHeight - 220,
      );
    });
    await page.waitForTimeout(300);
    await waitForAnimations(page);

    await page.screenshot({
      path: "test-results/channel-overflow/thread-composer.png",
      clip: { x: 1280 - 560, y: 720 - 300, width: 560, height: 300 },
    });

    await expectOverlayBottomMask(page.getByTestId("thread-composer-overlay"));
  });
});
