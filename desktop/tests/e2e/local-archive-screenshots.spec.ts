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

  test("01 — subscriptions list with channel entry and observer toggle on", async ({
    page,
  }) => {
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

    // Channel subscription row appears in the channel list.
    await expect(
      card.getByTestId(`local-archive-sub-channel_h:${GENERAL_CHANNEL_ID}`),
    ).toBeVisible({ timeout: 5_000 });
    // owner_p rows are filtered out of the channel list and surfaced via the
    // dedicated observer section instead — toggle should be visible and checked.
    const observerToggle = card.getByTestId("local-archive-observer-toggle");
    await expect(observerToggle).toBeVisible({ timeout: 5_000 });
    await expect(observerToggle).toBeChecked();
    await settleAnimations(card);
    await card.screenshot({ path: `${SHOTS}/01-subscriptions-list.png` });
  });

  test("02 — Add form opens directly to channel picker", async ({ page }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    // The source-picker step no longer exists — clicking Add opens the channel
    // subscription form directly (channel select is the first visible field).
    await card.getByTestId("local-archive-open-add").click();
    await expect(card.getByTestId("local-archive-channel-select")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(card);
    await card.screenshot({ path: `${SHOTS}/02-add-form-channel-picker.png` });
  });

  test("03 — Step 2 kind checklist with indeterminate group header", async ({
    page,
  }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    // Navigate to the kind checklist — clicking Add opens the form directly.
    await card.getByTestId("local-archive-open-add").click();

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

  test("05 — observer archive section with toggle", async ({ page }) => {
    await installMockBridge(page, { saveSubscriptions: [] });

    const card = await openLocalArchiveSettings(page);

    // Observer archive is now a dedicated first-class section, not an add-flow
    // source. Assert the section, descriptive copy, and toggle are all visible.
    const observerSection = card.getByTestId("local-archive-observer-section");
    await expect(observerSection).toBeVisible({ timeout: 5_000 });
    // The section description mentions observer frames and their ephemeral nature.
    await expect(
      observerSection.getByText(/not stored by the relay/i),
    ).toBeVisible();
    await expect(
      card.getByTestId("local-archive-observer-toggle"),
    ).toBeVisible();
    await settleAnimations(card);
    await card.screenshot({ path: `${SHOTS}/05-observer-section.png` });
  });
});
