import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "networkidle" });
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", 43004);
});

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, expectedKind }) => {
          return (
            (
              window as Window & {
                __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind: number;
                }) => boolean;
              }
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: currentChannelName,
              kind: expectedKind,
            }) ?? false
          );
        },
        { currentChannelName: channelName, expectedKind: kind },
      );
    })
    .toBe(true);
}

test("opens the signed channel panel and preserves its empty state", async ({
  page,
}) => {
  await page.getByTestId("channel-panel-trigger").click();

  await expect(page.getByTestId("signed-channel-panel")).toBeVisible();
  await expect(page.getByTestId("signed-channel-panel-empty")).toBeVisible();
  await expect(page.getByTestId("signed-channel-panel-empty")).toContainText(
    "No signed job activity has been published in this channel yet.",
  );
  await expect(page).toHaveURL(/panel=%221%22/);

  await page.getByTestId("auxiliary-panel-close").click();
  await expect(page.getByTestId("signed-channel-panel")).toBeHidden();
  await expect(page).not.toHaveURL(/panel=%22/);
});

test("renders signed job activity with source provenance", async ({ page }) => {
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          kind: number;
          id: string;
          createdAt: number;
        }) => unknown;
      }
    ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Validated the signed deliverable and recorded the result.",
      kind: 43004,
      id: "f".repeat(64),
      createdAt: 2_000_000_000,
    });
  });

  await expect(page.getByTestId("channel-panel-trigger")).toBeVisible();
  await page.getByTestId("channel-panel-trigger").click();
  await expect(page.getByTestId("signed-channel-panel-ready")).toBeVisible();
  await expect(
    page.getByTestId("signed-channel-panel-status-complete").first(),
  ).toBeVisible();
  await expect(
    page.getByTestId("signed-channel-panel-provenance"),
  ).toContainText("Job result");

  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/screenshots/signed-channel-panel-ready.png",
  });
});
