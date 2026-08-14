import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

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

test.describe("open thread marker in the main timeline", () => {
  test("marks the thread head while the panel is open and clears on close", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await installMockBridge(page);

    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await emitMockReply(
      page,
      "general",
      "Reply to welcome",
      "mock-general-welcome",
    );

    const markedRows = page.locator('[data-open-thread-head="true"]');
    // Nothing is marked until a thread is actually open.
    await expect(markedRows).toHaveCount(0);

    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();
    await threadSummary.click();
    await expect(page.getByTestId("message-thread-panel")).toBeVisible();

    // Exactly one main-timeline row carries the marker: the open thread head.
    await expect(markedRows).toHaveCount(1);
    await expect(markedRows.first()).toContainText("Welcome");

    // Park the pointer away from the row so the capture shows the resting
    // marker rather than the hover background.
    await page.mouse.move(0, 0);
    await waitForAnimations(page);
    await page.screenshot({
      path: "test-results/open-thread-highlight.png",
    });

    // Escape is deliberately inert for the split panel (see `useEscapeKey` in
    // MessageThreadPanelSkeleton) — close through the header action instead.
    await page.getByTestId("auxiliary-panel-close").click();
    await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);
    await expect(markedRows).toHaveCount(0);
  });
});
