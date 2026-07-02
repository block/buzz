import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/local-archive";

// Well-known channel IDs from the mock bridge seed (e2eBridge.ts mockChannels).
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

// Navigate to the Local Archive settings panel.
async function openLocalArchiveSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-local-archive").click();
  const card = page.getByTestId("settings-local-archive");
  await expect(card).toBeVisible({ timeout: 10_000 });
  return card;
}

async function settleAnimations(el: import("@playwright/test").Locator) {
  await el.evaluate((node) =>
    Promise.all(node.getAnimations({ subtree: true }).map((a) => a.finished)),
  );
}

test.describe("local archive screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });
  });

  test("01 — subscriptions list with two active entries", async ({ page }) => {
    await installMockBridge(page, {
      saveSubscriptions: [
        {
          scope_type: "channel_h",
          scope_value: GENERAL_CHANNEL_ID,
          kinds: "[9,40002,40003]",
        },
        {
          scope_type: "owner_p",
          scope_value: "deadbeef".repeat(8),
          kinds: "[24200]",
        },
      ],
    });

    const card = await openLocalArchiveSettings(page);

    // Wait for both subscription rows to appear.
    await expect(
      card.getByTestId(`local-archive-sub-channel_h:${GENERAL_CHANNEL_ID}`),
    ).toBeVisible({ timeout: 5_000 });
    await expect(
      card.getByTestId(`local-archive-sub-owner_p:${"deadbeef".repeat(8)}`),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(card);
    await card.screenshot({ path: `${SHOTS}/01-subscriptions-list.png` });
  });

  test("02 — Step 1 source picker (Channel vs My agents observer feed)", async ({
    page,
  }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    // Open the Add form — the source picker is Step 1.
    await card.getByTestId("local-archive-open-add").click();
    await expect(
      card.getByTestId("local-archive-add-channel"),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(card);
    await card.screenshot({ path: `${SHOTS}/02-step1-source-picker.png` });
  });

  test("03 — Step 2 kind checklist with indeterminate group header", async ({
    page,
  }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    // Navigate to Step 2 via the Channel source path.
    await card.getByTestId("local-archive-open-add").click();
    await card.getByTestId("local-archive-add-channel").click();

    // Step 2 should be visible now. Select a channel so the form becomes valid.
    await card
      .getByTestId("local-archive-channel-select")
      .selectOption({ value: GENERAL_CHANNEL_ID });

    // Check a subset of the first group's items to trigger the indeterminate
    // state on the group header checkbox.
    const firstGroupItems = card
      .locator("[data-testid^='local-archive-kind-']")
      .first();
    await firstGroupItems.click();

    await settleAnimations(card);
    await card.screenshot({
      path: `${SHOTS}/03-step2-kind-checklist-indeterminate.png`,
    });
  });

  test("04 — custom kinds entry with invalid-token error", async ({ page }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    await card.getByTestId("local-archive-open-add").click();
    await card.getByTestId("local-archive-add-channel").click();

    // Type invalid tokens into the custom kinds field.
    await card
      .getByTestId("local-archive-custom-kinds")
      .fill("30023 bad-token 1337 notanumber");

    // Error message should appear.
    await expect(
      card.getByTestId("local-archive-custom-kinds-error"),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(card);
    await card.screenshot({
      path: `${SHOTS}/04-custom-kinds-invalid-error.png`,
    });
  });

  test("05 — observer feed fixed-[24200] step (owner_p source)", async ({
    page,
  }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    await card.getByTestId("local-archive-open-add").click();
    // Click "Add" for the observer feed source.
    await card.getByTestId("local-archive-add-owner").click();

    // Step 2 for owner_p: shows the informational fixed-[24200] message.
    await expect(
      card.getByText(/observer frames/),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(card);
    await card.screenshot({
      path: `${SHOTS}/05-observer-fixed-24200.png`,
    });
  });
});
