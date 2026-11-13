import { expect, type Locator, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type OpenerWindow = Window & {
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: { url?: string };
  }>;
};

const UPGRADE_GUIDE_URL =
  "https://github.com/block/buzz-app/blob/main/docs/upgrading.md";
const WEBSITE_URL = "https://buzz.xyz";

// The main window cannot go below 900px wide, and Cmd+ zoom tops out at 1.5x,
// which leaves the banner the room of a 600px window at default zoom.
const MIN_WINDOW_WIDTH = 900;
const MIN_WIDTH_AT_MAX_ZOOM = 600;
// Enough for the message to read as a sentence instead of a one-word column.
const MIN_MESSAGE_WIDTH = 240;

function openedUrls(page: Page) {
  return page.evaluate(() =>
    (window as OpenerWindow).__BUZZ_E2E_COMMAND_PAYLOADS__
      ?.filter((call) => call.command === "plugin:opener|open_url")
      .map((call) => call.payload.url),
  );
}

async function boxOf(locator: Locator) {
  const box = await locator.boundingBox();
  expect(box).not.toBeNull();
  return box ?? { x: 0, y: 0, width: 0, height: 0 };
}

test("deprecation banner: both links open externally and the row holds up when narrow", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    seedDeprecationBannerDismissed: false,
  });
  await page.goto("/");

  const banner = page.getByTestId("deprecation-banner");
  const message = banner.getByTestId("deprecation-banner-message");
  // "More info" alone says little out of context, so the accessible name adds
  // the topic and that the link leaves the app.
  const upgradeGuide = banner.getByRole("button", {
    exact: true,
    name: "More info about upgrading, opens in your browser",
  });
  const download = banner.getByRole("button", { name: "Get the new app" });
  const dismiss = banner.getByRole("button", { name: "Remind me later" });
  await expect(banner).toBeVisible();
  await expect(upgradeGuide).toHaveText("More info");
  // The two actions carry the same classes, so neither outweighs the other
  // at rest or on hover in any theme. Only the download button has an icon.
  expect(await upgradeGuide.getAttribute("class")).toBe(
    await download.getAttribute("class"),
  );
  await expect(upgradeGuide.locator("svg")).toHaveCount(0);

  // Pointer and keyboard both reach the opener, each with its own link.
  await upgradeGuide.click();
  await download.focus();
  await page.keyboard.press("Enter");
  expect(await openedUrls(page)).toEqual([UPGRADE_GUIDE_URL, WEBSITE_URL]);

  for (const width of [MIN_WINDOW_WIDTH, MIN_WIDTH_AT_MAX_ZOOM]) {
    await page.setViewportSize({ width, height: 650 });
    const bannerBox = await boxOf(banner);
    expect(bannerBox.x + bannerBox.width, `banner at ${width}`).toBe(width);
    expect(
      (await boxOf(message)).width,
      `message at ${width}`,
    ).toBeGreaterThanOrEqual(MIN_MESSAGE_WIDTH);

    // Every control stays whole, inside the banner, and clear of the others.
    const controls = [
      await boxOf(upgradeGuide),
      await boxOf(download),
      await boxOf(dismiss),
    ];
    for (const [index, box] of controls.entries()) {
      expect(box.x, `control ${index} at ${width}`).toBeGreaterThanOrEqual(
        bannerBox.x,
      );
      expect(
        box.x + box.width,
        `control ${index} at ${width}`,
      ).toBeLessThanOrEqual(bannerBox.x + bannerBox.width);
      const previous = controls[index - 1];
      if (previous) {
        expect(box.x, `control ${index} at ${width}`).toBeGreaterThanOrEqual(
          previous.x + previous.width,
        );
      }
    }
  }

  await dismiss.click();
  await expect(banner).toBeHidden();
});
