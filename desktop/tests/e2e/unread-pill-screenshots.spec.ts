import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/unread-pill";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
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
      );
    })
    .toBe(true);
}

function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
) {
  return page.evaluate(
    ({ ch, msg, pubkey }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        pubkey,
      });
    },
    { ch: channelName, msg: content, pubkey: TEST_IDENTITIES.alice.pubkey },
  );
}

test.describe("unread pill & divider screenshots", () => {
  test("01-unread-pill-visible", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    // Open general, then switch to random so general becomes inactive
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // Emit 3 messages to general while we're away
    await emitMockMessage(page, "general", "First unread message");
    await emitMockMessage(page, "general", "Second unread message");
    await emitMockMessage(page, "general", "Third unread message");

    // Switch back to general — pill should appear
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const pill = page.getByTestId("message-unread-pill");
    await expect(pill).toBeVisible();
    await expect(pill).toContainText("3 new messages");

    await page.screenshot({
      path: `${SHOTS}/01-unread-pill-visible.png`,
    });
  });

  test("02-unread-divider-visible", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    await emitMockMessage(page, "general", "First unread message");
    await emitMockMessage(page, "general", "Second unread message");
    await emitMockMessage(page, "general", "Third unread message");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();

    // Scroll the divider into view for a clear screenshot
    await divider.scrollIntoViewIfNeeded();
    await page.waitForTimeout(300);

    await page.screenshot({
      path: `${SHOTS}/02-unread-divider-visible.png`,
    });
  });

  test("03-pill-dismissed-after-scroll", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");

    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    await emitMockMessage(page, "general", "First unread message");
    await emitMockMessage(page, "general", "Second unread message");
    await emitMockMessage(page, "general", "Third unread message");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const pill = page.getByTestId("message-unread-pill");
    await expect(pill).toBeVisible();

    // Click the pill to jump to oldest unread. The topbar search overlay
    // (fixed, higher z-index) sits over the pill's position and swallows a
    // hit-tested click, so dispatch the event directly to exercise the real
    // jump-and-dismiss handler.
    await pill.dispatchEvent("click");

    // Pill should be dismissed
    await expect(pill).toHaveCount(0);

    // Divider should still be visible
    const divider = page.getByTestId("message-unread-divider");
    await expect(divider).toBeVisible();

    await page.screenshot({
      path: `${SHOTS}/03-pill-dismissed-after-scroll.png`,
    });
  });

  test("04-mark-unread-suppresses-pill", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Mark channel unread via context menu on the sidebar item
    await page.getByTestId("channel-general").click({ button: "right" });
    await page.getByText("Mark unread").click();

    // Switch away and back to re-open the channel
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");

    // The unread indicator only renders on inactive channels, so it appears
    // once general is no longer the active channel.
    await expect(page.getByTestId("channel-unread-general")).toBeVisible();

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Pill and divider should NOT appear (suppressed for forced-unread)
    await expect(page.getByTestId("message-unread-pill")).toHaveCount(0);
    await expect(page.getByTestId("message-unread-divider")).toHaveCount(0);

    await page.screenshot({
      path: `${SHOTS}/04-mark-unread-suppresses-pill.png`,
    });
  });
});
