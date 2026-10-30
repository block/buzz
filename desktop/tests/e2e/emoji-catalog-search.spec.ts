import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Catalog emoji (kind 30180) are never loaded as a list. The picker panel and
// the `:par` autocomplete ask the relay by prefix; the mock bridge serves the
// `search_emoji_entries` command from `src/testing/e2eEmojiCatalog.ts`.

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#22c55e"/></svg>',
    }),
  );
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("picker search caps results at 50, hints to narrow, and inserts with the keyboard", async ({
  page,
}) => {
  await page.getByTestId("composer-emoji-button").click();
  const search = page.getByTestId("emoji-catalog-search");
  await expect(search).toBeVisible();

  await search.fill("wave");
  await expect(page.getByTestId("emoji-catalog-result")).toHaveCount(50);
  await expect(page.getByTestId("emoji-catalog-status")).toContainText(
    "Keep typing to narrow",
  );

  await search.fill("party");
  const options = page.getByRole("option");
  await expect(options).toHaveCount(2);
  await expect(page.getByRole("option", { name: ":bufo_party:" })).toHaveCount(
    1,
  );

  // Shift+Space is a typed space, never a selection.
  await search.press("Shift+Space");
  await expect(search).toHaveValue("party ");
  await expect(
    page.getByTestId("message-input").locator("img[data-custom-emoji]"),
  ).toHaveCount(0);

  await search.press("ArrowDown");
  await search.press("Enter");
  await expect(
    page.getByTestId("message-input").locator("img[data-custom-emoji]"),
  ).toHaveCount(1);
});

test("autocomplete finds a catalog emoji after the debounce and Tab inserts it", async ({
  page,
}) => {
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(":party_pa");

  const autocomplete = page.getByTestId("emoji-autocomplete");
  const parrot = autocomplete.getByRole("option", { name: ":party_parrot:" });
  await expect(parrot).toHaveCount(1);
  await expect(autocomplete.getByRole("option")).not.toHaveCount(0);

  await input.press("Tab");
  await expect(input.locator("img[data-custom-emoji]")).toHaveAttribute(
    "data-shortcode",
    "party_parrot",
  );
});
