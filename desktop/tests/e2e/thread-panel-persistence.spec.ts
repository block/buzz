import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function seedThread(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  return page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Thread persistence root",
      createdAt: 1_700_900_000,
    });
    if (!root) throw new Error("Failed to seed thread root");
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Thread persistence reply",
      parentEventId: root.id,
      createdAt: 1_700_900_001,
    });
    return root.id;
  });
}

/**
 * Per-channel thread-panel memory: leaving a channel and returning restores
 * the panel the way it was left — open on the same thread, or closed
 * (`channelPanelMemory.ts`, seeded into the URL by `goChannel`).
 */
test("thread panel is restored per channel across sidebar switches", async ({
  page,
}) => {
  await page.goto("/");
  const rootId = await seedThread(page);

  // Open the seeded thread in #general.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();

  // Switching to a channel with no memory leaves its panel closed.
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();

  // Returning restores the same thread from memory.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page).toHaveURL(new RegExp(`thread=${rootId}`));

  // Explicitly closing the panel is remembered: it stays closed on return.
  await page.getByRole("button", { name: "Close panel" }).click();
  await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-thread-panel")).not.toBeVisible();
  await expect(page).not.toHaveURL(/thread=/);
});
