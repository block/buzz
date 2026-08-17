import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// The channel and thread composers float over their conversation scrollers.
// When the conversation is scrolled up, later rows pass underneath the
// composer overlay — the overlay must mask them (gradient fade + solid
// bottom strip, mirroring the inbox treatment from PR #2143) instead of
// letting them bleed out below the composer box.

const CHANNEL = "general";
const TYPING_KIND = 20002;

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ channelName, kind }) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName,
            kind,
          }) ?? false,
        { channelName, kind },
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
  return event as { created_at: number; id: string };
}

type ComposerDockGeometry = {
  composerBottom: number;
  composerLeft: number;
  composerRight: number;
  composerTop: number;
  dockHeight: number;
  dockTop: number;
};

async function composerDockGeometry(
  overlay: import("@playwright/test").Locator,
): Promise<ComposerDockGeometry> {
  return overlay.evaluate((element) => {
    const composer = element.querySelector<HTMLElement>(
      '[data-testid="message-composer"]',
    );
    const dock = element.querySelector<HTMLElement>(".composer-dock");
    if (!composer || !dock) throw new Error("Missing composer dock geometry");
    const composerRect = composer.getBoundingClientRect();
    const dockRect = dock.getBoundingClientRect();
    return {
      composerBottom: composerRect.bottom,
      composerLeft: composerRect.left,
      composerRight: composerRect.right,
      composerTop: composerRect.top,
      dockHeight: dockRect.height,
      dockTop: dockRect.top,
    };
  });
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

  test("activity trades composer height without moving the dock", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();
    await waitForMockLiveSubscription(page, CHANNEL);
    await waitForMockLiveSubscription(page, CHANNEL, TYPING_KIND);

    const overlay = page.getByTestId("channel-composer-overlay");
    const input = overlay.getByTestId("message-input");
    await input.fill(
      "A multiline draft that grows the composer.\nSecond line keeps the natural-height path active.",
    );
    await waitForAnimations(page);
    const quiet = await composerDockGeometry(overlay);

    await page.evaluate((channelName) => {
      window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({ channelName });
    }, CHANNEL);
    await expect(
      overlay.getByTestId("channel-composer-activity-row"),
    ).toBeVisible();
    await waitForAnimations(page);
    const active = await composerDockGeometry(overlay);

    expect(Math.abs(active.dockTop - quiet.dockTop)).toBeLessThanOrEqual(0.5);
    expect(Math.abs(active.dockHeight - quiet.dockHeight)).toBeLessThanOrEqual(
      0.5,
    );
    expect(
      Math.abs(active.composerTop - quiet.composerTop),
    ).toBeLessThanOrEqual(0.5);
    expect(active.composerBottom).toBeLessThan(quiet.composerBottom - 8);
  });

  test("reduced motion removes dock geometry transitions", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();

    const dock = page
      .getByTestId("channel-composer-overlay")
      .locator(".composer-dock");
    const spacer = dock.locator(".composer-dock-quiet-spacer");
    await expect(dock).toHaveCSS("transition-duration", "0s");
    await expect(spacer).toHaveCSS("transition-duration", "0s");
  });

  test("bottom-pinned messages clear a growing composer", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL}`).click();
    await expect(page.getByTestId("message-timeline")).toBeVisible();
    await waitForMockLiveSubscription(page, CHANNEL);

    let newestMessageId = "";
    for (let i = 0; i < 20; i++) {
      newestMessageId = (await emit(page, `Bottom-clearance message ${i}`)).id;
    }
    await page.waitForTimeout(300);
    await page.evaluate(() => {
      const scroller = document.querySelector<HTMLElement>(
        '[data-buzz-conversation-scroll="true"]',
      );
      if (!scroller) throw new Error("Missing conversation scroll container");
      scroller.scrollTop = scroller.scrollHeight;
    });

    await page
      .getByTestId("message-input")
      .fill(
        "A long draft grows the composer and must keep the newest message above it.\nSecond line.\nThird line.",
      );
    await waitForAnimations(page);

    const clearsComposer = await page.evaluate((messageId) => {
      const composer = document.querySelector<HTMLElement>(
        '[data-testid="channel-composer-overlay"] [data-testid="message-composer"]',
      );
      const newestRow = document.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(messageId)}"]`,
      );
      if (!composer || !newestRow) return false;
      return (
        newestRow.getBoundingClientRect().bottom <=
        composer.getBoundingClientRect().top
      );
    }, newestMessageId);
    expect(clearsComposer).toBe(true);

    await page.setViewportSize({ width: 1280, height: 620 });
    await expect
      .poll(() =>
        page.evaluate((messageId) => {
          const composer = document.querySelector<HTMLElement>(
            '[data-testid="channel-composer-overlay"] [data-testid="message-composer"]',
          );
          const newestRow = document.querySelector<HTMLElement>(
            `[data-message-id="${CSS.escape(messageId)}"]`,
          );
          return Boolean(
            composer &&
              newestRow &&
              newestRow.getBoundingClientRect().bottom <=
                composer.getBoundingClientRect().top,
          );
        }, newestMessageId),
      )
      .toBe(true);
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

    await waitForMockLiveSubscription(page, CHANNEL, TYPING_KIND);
    await page.evaluate(
      ({ channelName, createdAt, pubkey, threadHeadId }) => {
        window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
          channelName,
          createdAt,
          pubkey,
          threadHeadId,
        });
      },
      {
        channelName: CHANNEL,
        createdAt: root.created_at + 1,
        pubkey: TEST_IDENTITIES.bob.pubkey,
        threadHeadId: root.id,
      },
    );
    const threadOverlay = page.getByTestId("thread-composer-overlay");
    const activityRow = threadOverlay.getByTestId("message-typing-indicator");
    await expect(activityRow).toBeVisible();
    await waitForAnimations(page);
    const [composerBounds, activityBounds] = await Promise.all([
      threadOverlay.getByTestId("message-composer").boundingBox(),
      activityRow.boundingBox(),
    ]);
    expect(composerBounds).not.toBeNull();
    expect(activityBounds).not.toBeNull();
    expect(activityBounds?.x ?? 0).toBeGreaterThanOrEqual(
      (composerBounds?.x ?? 0) - 0.5,
    );
    expect(
      (activityBounds?.x ?? 0) + (activityBounds?.width ?? 0),
    ).toBeLessThanOrEqual(
      (composerBounds?.x ?? 0) + (composerBounds?.width ?? 0) + 0.5,
    );
  });
});
