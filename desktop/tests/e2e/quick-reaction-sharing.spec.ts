import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const STORAGE_KEY = "buzz.quick-reaction-emojis.v1:e2e-default-community";

// Exercise AppReady's provider wiring and real action bars, not a test-only
// provider tree. A missing provider would silently show the default tray.
test("quick trays share prepared custom items across channel remounts", async ({
  page,
}) => {
  await page.addInitScript((key) => {
    localStorage.setItem(
      key,
      JSON.stringify([
        { emoji: ":buzz:", count: 10, lastUsedAt: 1 },
        { emoji: "🔥", count: 5, lastUsedAt: 1 },
      ]),
    );
  }, STORAGE_KEY);
  await installMockBridge(page);
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"/>',
    }),
  );
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "React to me with a custom emoji" })
    .last();
  await expect(row).toBeVisible();
  await row.hover();
  const tray = row.locator('[data-testid^="message-action-bar-"]');
  const quick = tray.getByRole("button", { name: /^React with / });
  await expect(quick).toHaveCount(3);
  await expect(quick.first().locator("img")).toHaveAttribute("alt", ":buzz:");
  const originalLabels = await quick.evaluateAll((buttons) =>
    buttons.map((button) => button.getAttribute("aria-label")),
  );

  // Keyboard activation goes through the unchanged action handler and persists
  // recents, but must not reshuffle this session's tray.
  await quick.nth(1).focus();
  await page.keyboard.press("Enter");
  await expect(
    row
      .getByTestId("message-reactions")
      .getByRole("button", { name: "Toggle 🔥 reaction" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        (key) =>
          JSON.parse(localStorage.getItem(key) ?? "[]").find(
            (entry: { emoji: string }) => entry.emoji === "🔥",
          )?.count,
        STORAGE_KEY,
      ),
    )
    .toBe(6);
  expect(
    await quick.evaluateAll((buttons) =>
      buttons.map((button) => button.getAttribute("aria-label")),
    ),
  ).toEqual(originalLabels);

  // A palette URL update reaches the actual button without changing rank.
  await page.evaluate(() => {
    const query = window.__BUZZ_E2E_QUERY_CLIENT__;
    const palette = query?.getQueryData<
      Array<{ shortcode: string; url: string }>
    >(["custom-emoji"]);
    if (!query || !palette) throw new Error("palette query not ready");
    query.setQueryData(
      ["custom-emoji"],
      palette.map((item) =>
        item.shortcode === "buzz"
          ? { ...item, url: "https://example.com/e2e/buzz-new.png" }
          : item,
      ),
    );
  });
  await expect(quick.first().locator("img")).toHaveAttribute(
    "src",
    "https://example.com/e2e/buzz-new.png",
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-general").click();
  await expect(row).toBeVisible();
  await row.hover();
  expect(
    await quick.evaluateAll((buttons) =>
      buttons.map((button) => button.getAttribute("aria-label")),
    ),
  ).toEqual(originalLabels);

  // Other-window storage notifications refresh all mounted trays.
  await page.evaluate((key) => {
    const newValue = JSON.stringify([
      { emoji: "🎉", count: 100, lastUsedAt: 2 },
    ]);
    localStorage.setItem(key, newValue);
    window.dispatchEvent(
      new StorageEvent("storage", { key, newValue, storageArea: localStorage }),
    );
  }, STORAGE_KEY);
  await expect(quick.first()).toContainText("🎉");
  const trays = page.locator('[data-testid^="message-action-bar-"]');
  expect(await trays.count()).toBeGreaterThan(1);
  for (const bar of await trays.all()) {
    const buttons = bar.getByRole("button", { name: /^React with / });
    if (await buttons.count())
      await expect(buttons.first()).toContainText("🎉");
  }
});
