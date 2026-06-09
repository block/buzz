import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Screenshot capture for the user-owned custom-emoji rebuild (PR 816). Not a
// hard assertion suite — it documents the two user-visible surfaces Tyler asked
// to verify: the composer rendering and the settings card's own-vs-workspace
// split. Artifacts land in test-results/.
const SHORTCODE = "sprout";
const SHOTS = "test-results/custom-emoji";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  // The mock emoji sets point at example.com placeholder URLs that don't
  // resolve, so the <img> would render broken in screenshots. Serve a real
  // 1x1-scaled magenta PNG for any example.com emoji image so the captures
  // actually show a rendered glyph. (Screenshot-only; the bridge fixtures stay
  // honest for the union/collapse unit + e2e assertions.)
  const PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({ contentType: "image/png", body: PNG }),
  );
});

test("composer renders a custom emoji inline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`shipping it :${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);

  await page.screenshot({ path: `${SHOTS}/01-composer-inline-emoji.png` });
});

test("settings card splits My emoji from read-only Workspace emoji", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-custom-emoji").click();

  // The mock identity owns :sprout: (removable); :narf: belongs to another
  // member (read-only, no trash button).
  const card = page.getByTestId("settings-custom-emoji");
  await expect(card.getByTestId("custom-emoji-mine")).toContainText(":sprout:");
  const mine = card.getByTestId("custom-emoji-mine");
  await expect(
    mine.getByRole("button", { name: "Remove :sprout:" }),
  ).toBeVisible();

  const workspace = card.getByTestId("custom-emoji-workspace");
  await expect(workspace).toContainText(":narf:");
  // No remove button for someone else's emoji.
  await expect(
    workspace.getByRole("button", { name: /^Remove :/ }),
  ).toHaveCount(0);

  await page.screenshot({
    path: `${SHOTS}/02-settings-own-vs-workspace.png`,
    fullPage: true,
  });
});
