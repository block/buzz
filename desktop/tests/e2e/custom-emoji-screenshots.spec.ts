import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Screenshot capture for the user-owned custom-emoji rebuild (PR 816). Not a
// hard assertion suite — it documents the two user-visible surfaces Tyler asked
// to verify: the composer rendering and the settings card's own-vs-workspace
// split. Artifacts land in test-results/.
const SHORTCODE = "buzz";
const SHOTS = "test-results/custom-emoji";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  // The mock emoji sets point at example.com placeholder URLs that don't
  // resolve, so the <img> would render broken in screenshots. Serve a visible
  // square glyph for any example.com emoji image so the captures show the
  // custom-emoji sizing/alignment rather than a broken-image icon.
  const SVG = `
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
      <rect width="32" height="32" rx="7" fill="#22c55e"/>
      <circle cx="16" cy="12" r="5" fill="#fef3c7"/>
      <path d="M8 25c2-7 14-7 16 0" fill="#fef3c7"/>
    </svg>`;
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({ contentType: "image/svg+xml", body: SVG }),
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

  // The mock identity owns :buzz: (removable); :narf: belongs to another
  // member (read-only, no trash button).
  const card = page.getByTestId("settings-custom-emoji");
  await expect(card.getByTestId("custom-emoji-mine")).toContainText(":buzz:");
  const mine = card.getByTestId("custom-emoji-mine");
  await expect(
    mine.getByRole("button", { name: "Remove :buzz:" }),
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

test("message list renders inline and emoji-only messages with Slack-style emoji sizing", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");

  await input.click();
  await input.pressSequentially(`inline :${SHORTCODE}: message`);
  await page.getByTestId("send-message").click();

  await input.click();
  await input.pressSequentially(`:${SHORTCODE}: 😀 ❤️`);
  await page.getByTestId("send-message").click();

  const rows = page.getByTestId("message-row");
  const inlineRow = rows
    .filter({
      has: page.locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`),
      hasText: "inline message",
    })
    .last();
  const emojiOnlyRow = rows
    .filter({
      has: page.locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`),
      hasText: "😀 ❤️",
    })
    .first();

  await expect(inlineRow).toBeVisible();
  await expect(emojiOnlyRow).toBeVisible();

  const inlineEmoji = inlineRow.locator(
    `img[data-custom-emoji][alt=":${SHORTCODE}:"]`,
  );
  const emojiOnlyEmoji = emojiOnlyRow.locator(
    `img[data-custom-emoji][alt=":${SHORTCODE}:"]`,
  );

  await expect
    .poll(async () => (await inlineEmoji.boundingBox())?.height ?? 0)
    .toBeGreaterThan(10);
  await expect
    .poll(async () => {
      const inlineBox = await inlineEmoji.boundingBox();
      const emojiOnlyBox = await emojiOnlyEmoji.boundingBox();

      if (!inlineBox || !emojiOnlyBox || inlineBox.height === 0) {
        return 0;
      }

      return emojiOnlyBox.height / inlineBox.height;
    })
    .toBeGreaterThan(1.8);

  await page.screenshot({
    path: `${SHOTS}/03-message-list-emoji-sizing.png`,
  });
});
