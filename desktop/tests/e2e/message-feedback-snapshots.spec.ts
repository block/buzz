import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/message-feedback";

/**
 * Hover `target` and return its background colour once the `transition-colors`
 * hover fade has landed.
 *
 * Reading `getComputedStyle` straight after `hover()` samples an interpolated
 * frame (`rgba(0, 0, 0, 0.027)` instead of the resting `rgba(0, 0, 0, 0.04)`),
 * so a comparison against another element's settled hover colour can never
 * match. Poll until two consecutive reads agree on a value that has moved off
 * the resting colour.
 */
async function settledHoverBackground(
  target: import("@playwright/test").Locator,
): Promise<string> {
  const readBackground = () =>
    target.evaluate((element) => getComputedStyle(element).backgroundColor);

  const resting = await readBackground();
  await target.hover();

  let previous = resting;
  let settled = resting;
  await expect
    .poll(async () => {
      const current = await readBackground();
      const isSettled = current !== resting && current === previous;
      previous = current;
      if (isSettled) {
        settled = current;
      }
      return isSettled;
    })
    .toBe(true);

  return settled;
}

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

test("pending continuation keeps Sending next to its timestamp", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  const sentMessage = `Message before pending state ${Date.now()}`;
  const pendingMessage = `Pending message status ${Date.now()}`;
  const createdAt = Math.floor(Date.now() / 1_000);
  await page.evaluate(
    ({ firstMessage, secondMessage, timestamp }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            createdAt: number;
            pending?: boolean;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      emit?.({
        channelName: "general",
        content: firstMessage,
        createdAt: timestamp - 1,
      });
      emit?.({
        channelName: "general",
        content: secondMessage,
        createdAt: timestamp,
        pending: true,
      });
    },
    {
      firstMessage: sentMessage,
      secondMessage: pendingMessage,
      timestamp: createdAt,
    },
  );

  const pendingRow = page
    .getByTestId("message-row")
    .filter({ hasText: pendingMessage });
  const status = pendingRow.getByTestId("message-send-status");
  await expect(status).toHaveText("Sending…");
  await expect(pendingRow.getByTestId("message-author")).toHaveCount(1);

  const timestamp = status.locator("xpath=../p[1]");
  const [timestampBox, statusBox] = await Promise.all([
    timestamp.boundingBox(),
    status.boundingBox(),
  ]);
  expect(timestampBox).not.toBeNull();
  expect(statusBox).not.toBeNull();
  if (!timestampBox || !statusBox) {
    throw new Error("Pending message metadata is missing its inline layout.");
  }
  expect(statusBox.x).toBeGreaterThan(timestampBox.x);
  expect(Math.abs(statusBox.y - timestampBox.y)).toBeLessThanOrEqual(1);

  await waitForAnimations(page);
  await pendingRow.screenshot({ path: `${SHOTS}/pending-message-inline.png` });
});

test("profile hover uses the channel hover surface", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");

  const profile = page.getByTestId("sidebar-profile-card");
  const channel = page.getByTestId("channel-random");
  const channelHoverColor = await settledHoverBackground(channel);
  await profile.hover();
  await expect(profile).toHaveCSS("background-color", channelHoverColor);

  await waitForAnimations(page);
  await page
    .getByTestId("app-sidebar")
    .screenshot({ path: `${SHOTS}/profile-hover.png` });
});
