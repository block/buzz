import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

// Ultrawide viewport: 3440px is a common 21:9 monitor width.
const ULTRAWIDE = { width: 3440, height: 1440 };

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      ),
    )
    .toBe(true);
}

async function emitMockReply(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  parentEventId: string,
) {
  await page.evaluate(
    ({ ch, msg, parent, pubkey }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId: parent,
        pubkey,
        createdAt: Math.floor(Date.now() / 1000) - 10,
      }),
    {
      ch: channelName,
      msg: content,
      parent: parentEventId,
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );
}

async function openThread(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  // Seed a reply so a thread summary row appears, then open it.
  await emitMockReply(
    page,
    "general",
    "Reply to welcome",
    "mock-general-welcome",
  );
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

test.describe("thread pane on ultrawide monitors", () => {
  test("expands well past the legacy 720px cap", async ({ page }) => {
    await page.setViewportSize(ULTRAWIDE);
    await installMockBridge(page);
    await openThread(page);

    const pane = page.getByTestId("message-thread-panel");
    const handle = page.getByTestId("right-auxiliary-pane-resize-handle");

    const beforeBox = await pane.boundingBox();
    if (!beforeBox) throw new Error("thread panel not laid out");
    // The panel opens at its default narrow width, far from the viewport edge.
    expect(beforeBox.width).toBeLessThan(720);
    await page.screenshot({
      path: "test-results/threadpane-ultrawide-before.png",
    });

    // Drag the resize handle far to the left to widen the right-hand pane.
    const handleBox = await handle.boundingBox();
    if (!handleBox) throw new Error("resize handle not laid out");
    const startX = handleBox.x + handleBox.width / 2;
    const startY = handleBox.y + handleBox.height / 2;
    await page.mouse.move(startX, startY);
    await page.mouse.down();
    // Move left in steps so pointermove fires repeatedly.
    for (let x = startX; x >= 600; x -= 120) {
      await page.mouse.move(x, startY);
    }
    await page.mouse.up();

    const afterBox = await pane.boundingBox();
    if (!afterBox) throw new Error("thread panel not laid out after resize");
    // The pane is now far wider than the old 720px hard cap.
    expect(afterBox.width).toBeGreaterThan(1200);
    await page.screenshot({
      path: "test-results/threadpane-ultrawide-after.png",
    });
  });
});
