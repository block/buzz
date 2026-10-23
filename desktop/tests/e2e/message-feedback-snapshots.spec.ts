import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/message-feedback";

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

test("pending continuation stays grouped through its acknowledgement", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  const sentMessage = `Message before pending state ${Date.now()}`;
  const pendingMessage = `Pending message status ${Date.now()}`;
  const pendingId = `${"a".repeat(63)}1`;
  const createdAt = Math.floor(Date.now() / 1_000);
  const emit = (pending: boolean) =>
    page.evaluate(
      ({ firstMessage, secondMessage, timestamp, id, isPending }) => {
        const emitMessage = (
          window as Window & {
            __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
              channelName: string;
              content: string;
              createdAt: number;
              id?: string;
              pending?: boolean;
            }) => unknown;
          }
        ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (isPending) {
          emitMessage?.({
            channelName: "general",
            content: firstMessage,
            createdAt: timestamp - 1,
          });
        }
        emitMessage?.({
          channelName: "general",
          content: secondMessage,
          createdAt: timestamp,
          id,
          pending: isPending,
        });
      },
      {
        firstMessage: sentMessage,
        secondMessage: pendingMessage,
        timestamp: createdAt,
        id: pendingId,
        isPending: pending,
      },
    );

  await emit(true);
  const pendingRow = page.locator(`[data-message-id="${pendingId}"]`);
  // The status is announced, but the grouped row carries no header for it.
  await expect(pendingRow.getByTestId("message-send-status")).toHaveText(
    "Sending…",
  );
  await expect(pendingRow.getByTestId("message-author")).toHaveCount(0);
  await waitForAnimations(page);
  const pendingBox = await pendingRow.boundingBox();
  await pendingRow.screenshot({ path: `${SHOTS}/pending-message-grouped.png` });

  await emit(false);
  await expect(pendingRow.getByTestId("message-send-status")).toHaveCount(0);
  await expect(pendingRow.getByTestId("message-author")).toHaveCount(0);
  await waitForAnimations(page);
  const sentBox = await pendingRow.boundingBox();

  expect(pendingBox).not.toBeNull();
  expect(sentBox).not.toBeNull();
  // Acknowledgement must not resize the row, or the timeline jumps per send.
  expect(sentBox?.height).toBe(pendingBox?.height);
});

test("profile hover uses the channel hover surface", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");

  const profile = page.getByTestId("sidebar-profile-card");
  const channel = page.getByTestId("channel-random");
  await expect(page.locator("html")).toHaveAttribute("data-buzz-sidebar", "");
  const hoverSurface = await channel.evaluate((element) => {
    const probe = document.createElement("span");
    probe.style.backgroundColor = "var(--buzz-hover-surface)";
    element.append(probe);
    const color = getComputedStyle(probe).backgroundColor;
    probe.remove();
    return color;
  });
  // Equal unfinished (transparent) surfaces must not satisfy hover parity.
  expect(hoverSurface).not.toMatch(/^(transparent|rgba\(.*,\s*0\))$/);
  await channel.hover();
  // Observe the semantic endpoint, not an intermediate transition sample or
  // the shared animation helper's timeout-as-success ceiling.
  await expect(channel).toHaveCSS("background-color", hoverSurface);
  const channelHoverColor = await channel.evaluate(
    (element) => getComputedStyle(element).backgroundColor,
  );
  await profile.hover();
  await expect(profile).toHaveCSS("background-color", channelHoverColor);

  await waitForAnimations(page);
  await page
    .getByTestId("app-sidebar")
    .screenshot({ path: `${SHOTS}/profile-hover.png` });
});
