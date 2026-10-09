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

type MockMessage = {
  channelName: string;
  content: string;
  createdAt: number;
  id?: string;
  pending?: boolean;
};

async function openGeneral(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  message: MockMessage,
) {
  await page.evaluate((input) => {
    (
      window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: MockMessage) => unknown;
      }
    ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.(input);
  }, message);
}

test("an own sent message opens into the timeline once", async ({ page }) => {
  await openGeneral(page);
  // Stretch the entrance so the in-flight state is observable on slow runners.
  await page.addStyleTag({
    content: ":root { --motion-duration-standard: 1500ms; }",
  });
  const id = `${"b".repeat(63)}1`;
  const createdAt = Math.floor(Date.now() / 1_000);
  const content = `Send entrance ${Date.now()}`;

  await emitMockMessage(page, {
    channelName: "general",
    content,
    createdAt,
    id,
    pending: true,
  });
  const row = page.locator(`[data-message-id="${id}"]`);
  const entrance = page
    .getByTestId("message-send-entrance")
    .filter({ has: row });
  await expect(entrance).toHaveClass(/motion-enter-send/);

  // The entrance ends by dropping its clipping class, so the hover action
  // rail is not cut off afterwards.
  await expect(entrance).not.toHaveClass(/motion-enter-send/);
  const settledBox = await row.boundingBox();

  await emitMockMessage(page, {
    channelName: "general",
    content,
    createdAt,
    id,
    pending: false,
  });
  await expect(row.getByTestId("message-send-status")).toHaveCount(0);
  // The acknowledgement keeps the row mounted: no replay, no resize.
  await expect(entrance).not.toHaveClass(/motion-enter-send/);
  expect(await entrance.evaluate((el) => el.getAnimations().length)).toBe(0);
  expect((await row.boundingBox())?.height).toBe(settledBox?.height);
});

test("messages that arrive already sent do not play the send entrance", async ({
  page,
}) => {
  await openGeneral(page);
  const id = `${"c".repeat(63)}1`;
  await emitMockMessage(page, {
    channelName: "general",
    content: `Incoming ${Date.now()}`,
    createdAt: Math.floor(Date.now() / 1_000),
    id,
  });
  await expect(page.locator(`[data-message-id="${id}"]`)).toBeVisible();
  await expect(
    page
      .getByTestId("message-send-entrance")
      .filter({ has: page.locator(`[data-message-id="${id}"]`) }),
  ).toHaveCount(0);
});

test("the send entrance does not animate with reduced motion", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await openGeneral(page);
  const id = `${"d".repeat(63)}1`;
  await emitMockMessage(page, {
    channelName: "general",
    content: `Reduced motion ${Date.now()}`,
    createdAt: Math.floor(Date.now() / 1_000),
    id,
    pending: true,
  });
  const entrance = page
    .getByTestId("message-send-entrance")
    .filter({ has: page.locator(`[data-message-id="${id}"]`) });
  await expect(entrance).toHaveCount(1);
  await expect(entrance).not.toHaveClass(/motion-enter-send/);
  expect(await entrance.evaluate((el) => el.getAnimations().length)).toBe(0);
});
