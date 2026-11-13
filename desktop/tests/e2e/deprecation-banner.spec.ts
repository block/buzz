import { expect, type Locator, type Page, test } from "@playwright/test";

import { BUZZ_APP_DOWNLOADS_MANIFEST_URL } from "../../src/app/newAppInstaller";
import { installMockBridge } from "../helpers/bridge";

type InstallerFailureReason = "network" | "storage";

type BridgeWindow = Window & {
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: { url?: string } | null;
  }>;
  __BUZZ_E2E_INSTALLER_DOWNLOAD__?: {
    progress: (receivedBytes: number, totalBytes: number | null) => void;
    finish: () => void;
    fail: (reason: InstallerFailureReason) => void;
    isActive: () => boolean;
  };
};

const UPGRADE_GUIDE_URL =
  "https://github.com/block/buzz-app/blob/main/docs/upgrading.md";
const WEBSITE_URL = "https://buzz.xyz";
const ARM64_DMG_URL =
  "https://github.com/block/buzz-app/releases/download/v1.0.0/Buzz_1.0.0_aarch64.dmg";
// The feed is served as text/plain with open CORS; mirror both.
const MANIFEST_RESPONSE = {
  body: JSON.stringify({
    schemaVersion: 1,
    channel: "stable",
    version: "1.0.0",
    release: "v1.0.0",
    downloads: { "macos-arm64": ARM64_DMG_URL },
  }),
  contentType: "text/plain; charset=utf-8",
  headers: { "access-control-allow-origin": "*" },
};

// The main window cannot go below 900px wide, and Cmd+ zoom tops out at 1.5x,
// which leaves the banner the room of a 600px window at default zoom.
const MIN_WINDOW_WIDTH = 900;
const MIN_WIDTH_AT_MAX_ZOOM = 600;
// Enough for the message to read as a sentence instead of a one-word column.
const MIN_MESSAGE_WIDTH = 240;

const MIB = 1024 * 1024;
const INSTALLER_FILE = "Buzz_1.0.0_aarch64.dmg";

/** Payload URLs of every call to `command`, in order. */
function urlsSentTo(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      ((window as BridgeWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
        .filter((call) => call.command === name)
        .map((call) => call.payload?.url),
    command,
  );
}

function openedUrls(page: Page) {
  return urlsSentTo(page, "plugin:opener|open_url");
}

function downloadRequests(page: Page) {
  return urlsSentTo(page, "download_new_app_installer");
}

async function commandCount(page: Page, command: string) {
  return (await urlsSentTo(page, command)).length;
}

function isMockDownloadActive(page: Page) {
  return page.evaluate(() =>
    (window as BridgeWindow).__BUZZ_E2E_INSTALLER_DOWNLOAD__?.isActive(),
  );
}

function emitProgress(
  page: Page,
  receivedBytes: number,
  totalBytes: number | null,
) {
  return page.evaluate(
    ([received, total]) =>
      (window as BridgeWindow).__BUZZ_E2E_INSTALLER_DOWNLOAD__?.progress(
        received as number,
        total,
      ),
    [receivedBytes, totalBytes] as const,
  );
}

function finishDownload(page: Page) {
  return page.evaluate(() =>
    (window as BridgeWindow).__BUZZ_E2E_INSTALLER_DOWNLOAD__?.finish(),
  );
}

function failDownload(page: Page, reason: InstallerFailureReason) {
  return page.evaluate(
    (why) =>
      (window as BridgeWindow).__BUZZ_E2E_INSTALLER_DOWNLOAD__?.fail(why),
    reason,
  );
}

/** Shows the banner with this machine's installer resolved. */
async function openBannerWithInstaller(page: Page) {
  await page.route(BUZZ_APP_DOWNLOADS_MANIFEST_URL, (route) =>
    route.fulfill(MANIFEST_RESPONSE),
  );
  await installMockBridge(
    page,
    { newAppInstallerTarget: "macos-arm64" },
    { seedDeprecationBannerDismissed: false },
  );
  await page.goto("/");
  const download = page
    .getByTestId("deprecation-banner")
    .getByRole("button", { name: /^Upgrade,/ });
  await expect(download).toHaveAttribute("data-download-source", "installer");
  return download;
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
  const download = banner.getByRole("button", { name: /^Upgrade,/ });
  const dismiss = banner.getByRole("button", { name: "Remind me later" });
  await expect(banner).toBeVisible();
  await expect(upgradeGuide).toHaveText("More info");
  // The default mock reports no installer target, so the banner never asks
  // for the manifest and the download button keeps the website link.
  await expect(download).toHaveAttribute("data-download-source", "fallback");
  // Without an installer the button is a link out, and says so.
  await expect(download).toHaveAccessibleName("Upgrade, opens in your browser");
  await expect(download).toHaveText("Upgrade");
  await expect(download).not.toHaveAttribute("aria-haspopup");
  // The two actions carry the same classes, so neither outweighs the other
  // at rest or on hover in any theme. Neither has an icon.
  expect(await upgradeGuide.getAttribute("class")).toBe(
    await download.getAttribute("class"),
  );
  await expect(upgradeGuide.locator("svg")).toHaveCount(0);
  await expect(download.locator("svg")).toHaveCount(0);

  // Pointer and keyboard both reach the opener, each with its own link.
  await upgradeGuide.click();
  await download.focus();
  await page.keyboard.press("Enter");
  expect(await openedUrls(page)).toEqual([UPGRADE_GUIDE_URL, WEBSITE_URL]);
  // The website link never starts an in-app download.
  expect(await downloadRequests(page)).toEqual([]);
  await expect(page.getByRole("dialog")).toHaveCount(0);

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

test("deprecation banner: the installer downloads in a dialog and the saved file is revealed", async ({
  page,
}) => {
  const download = await openBannerWithInstaller(page);
  // With an installer the button downloads in the app, and says so.
  await expect(download).toHaveAccessibleName(
    "Upgrade, downloads the installer",
  );
  await expect(download).toHaveAttribute("aria-haspopup", "dialog");

  await download.click();
  const dialog = page.getByRole("dialog", {
    name: "Downloading the new Buzz app",
  });
  await expect(dialog).toBeVisible();
  // The download runs in the app: the backend gets the URL, the browser nothing.
  expect(await downloadRequests(page)).toEqual([ARM64_DMG_URL]);
  expect(await openedUrls(page)).toEqual([]);
  await expect(dialog.getByTestId("new-app-installer-description")).toHaveText(
    `Saving ${INSTALLER_FILE} to your Downloads folder.`,
  );
  // Focus moves into the dialog, onto the one action a running download has.
  const cancel = dialog.getByRole("button", { name: "Cancel" });
  await expect(cancel).toBeFocused();

  // No total yet: the bar is indeterminate and only the byte count shows.
  const bar = dialog.getByRole("progressbar", { name: "Download progress" });
  const progressText = dialog.getByTestId("new-app-installer-progress-text");
  await expect(bar).not.toHaveAttribute("aria-valuenow");
  await emitProgress(page, 20 * MIB, null);
  await expect(progressText).toHaveText("20.0 MB downloaded");
  await expect(bar).not.toHaveAttribute("aria-valuenow");

  // With a total: bytes against the total, percent, and a bar with a value.
  await emitProgress(page, 39.5 * MIB, 79 * MIB);
  await expect(progressText).toHaveText("39.5 MB of 79.0 MB (50%)");
  await expect(bar).toHaveAttribute("aria-valuenow", "50");
  await expect(bar).toHaveAttribute(
    "aria-valuetext",
    "39.5 MB of 79.0 MB (50%)",
  );
  // Located on the page: the dialog's name changes when the download ends.
  const announcement = page.getByTestId("new-app-installer-announcement");
  await expect(announcement).toHaveAttribute("aria-live", "polite");
  await expect(announcement).toHaveText(
    `Downloading ${INSTALLER_FILE}, 50% done.`,
  );
  // A tick inside the same quarter changes the bar, not what is announced.
  await emitProgress(page, 45 * MIB, 79 * MIB);
  await expect(bar).toHaveAttribute("aria-valuenow", "56");
  await expect(announcement).toHaveText(
    `Downloading ${INSTALLER_FILE}, 50% done.`,
  );

  // A second activation while the download runs does not start another one.
  // The modal hides the banner from the accessibility tree and from the
  // pointer, so the press is dispatched straight at the element.
  await page
    .getByTestId("deprecation-banner-download")
    .evaluate((button: HTMLButtonElement) => button.click());
  await emitProgress(page, 50 * MIB, 79 * MIB);
  await expect(bar).toHaveAttribute("aria-valuenow", "63");
  expect(await downloadRequests(page)).toEqual([ARM64_DMG_URL]);

  // Finished: the dialog says where the file is and the app reveals it once.
  expect(await commandCount(page, "reveal_new_app_installer")).toBe(0);
  await finishDownload(page);
  const finished = page.getByRole("dialog", {
    name: "The new Buzz app is downloaded",
  });
  await expect(finished).toBeVisible();
  await expect(
    finished.getByTestId("new-app-installer-description"),
  ).toHaveText(
    `${INSTALLER_FILE} is in your Downloads folder. Open it when you're ready to install.`,
  );
  await expect(finished.getByTestId("new-app-installer-path")).toHaveText(
    "/Users/e2e/Downloads",
  );
  await expect(announcement).toHaveText(
    `Download finished. ${INSTALLER_FILE} is in your Downloads folder.`,
  );
  await expect
    .poll(() => commandCount(page, "reveal_new_app_installer"))
    .toBe(1);
  await expect(finished.getByRole("button", { name: "Cancel" })).toHaveCount(0);

  // The reveal button is worded for this platform's file manager and works
  // from the keyboard; it reveals the same file again.
  const expectedRevealLabel = await page.evaluate(() => {
    if (/mac|iphone|ipad|ipod/i.test(navigator.platform)) {
      return "Show in Finder";
    }
    return /^win/i.test(navigator.platform)
      ? "Show in File Explorer"
      : "Show in folder";
  });
  const reveal = finished.getByTestId("new-app-installer-reveal");
  await expect(reveal).toHaveText(expectedRevealLabel);
  await expect(finished.getByRole("button", { name: "Done" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(reveal).toBeFocused();
  await page.keyboard.press("Enter");
  await expect
    .poll(() => commandCount(page, "reveal_new_app_installer"))
    .toBe(2);

  // Closing returns focus to the button that opened the dialog.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(download).toBeFocused();
  expect(await openedUrls(page)).toEqual([]);
  expect(await isMockDownloadActive(page)).toBe(false);
});

test("deprecation banner: a failed download can be retried or taken to the website", async ({
  page,
}) => {
  const download = await openBannerWithInstaller(page);
  await download.click();
  await expect(
    page.getByRole("dialog", { name: "Downloading the new Buzz app" }),
  ).toBeVisible();

  await failDownload(page, "network");
  const failed = page.getByRole("dialog", {
    name: "The download didn't finish",
  });
  await expect(failed).toBeVisible();
  await expect(failed.getByTestId("new-app-installer-description")).toHaveText(
    "The download stopped before it was complete. Check your internet connection and try again. You can also get the new app from buzz.xyz.",
  );
  await expect(failed.getByRole("progressbar")).toHaveCount(0);

  // "Try again" takes focus and starts the same download again by keyboard.
  await expect(failed.getByRole("button", { name: "Try again" })).toBeFocused();
  await page.keyboard.press("Enter");
  const retrying = page.getByRole("dialog", {
    name: "Downloading the new Buzz app",
  });
  await expect(retrying).toBeVisible();
  expect(await downloadRequests(page)).toEqual([ARM64_DMG_URL, ARM64_DMG_URL]);
  expect(await isMockDownloadActive(page)).toBe(true);

  // A second failure names its own cause, and the website is one click away.
  await failDownload(page, "storage");
  await expect(
    failed.getByTestId("new-app-installer-description"),
  ).toContainText("couldn't be saved to your Downloads folder");
  await failed
    .getByRole("button", { name: "Open buzz.xyz in your browser" })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await openedUrls(page)).toEqual([WEBSITE_URL]);
  expect(await downloadRequests(page)).toHaveLength(2);
});

test("deprecation banner: Cancel and Escape stop a running download, a stray click does not", async ({
  page,
}) => {
  const download = await openBannerWithInstaller(page);
  // Mounting the banner clears any download a previous page left running.
  const cancelsAtRest = await commandCount(
    page,
    "cancel_new_app_installer_download",
  );

  // Escape cancels the download; the dialog closes once it has stopped.
  await download.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", {
    name: "Downloading the new Buzz app",
  });
  await expect(dialog).toBeVisible();
  expect(await isMockDownloadActive(page)).toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await isMockDownloadActive(page)).toBe(false);
  expect(await commandCount(page, "cancel_new_app_installer_download")).toBe(
    cancelsAtRest + 1,
  );
  await expect(download).toBeFocused();

  // A click beside the dialog leaves the download running.
  await download.click();
  await expect(dialog).toBeVisible();
  await emitProgress(page, 10 * MIB, 79 * MIB);
  await page.mouse.click(8, 8);
  await expect(dialog).toBeVisible();
  expect(await isMockDownloadActive(page)).toBe(true);
  expect(await commandCount(page, "cancel_new_app_installer_download")).toBe(
    cancelsAtRest + 1,
  );

  // The Cancel button stops it.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(await isMockDownloadActive(page)).toBe(false);
  expect(await commandCount(page, "cancel_new_app_installer_download")).toBe(
    cancelsAtRest + 2,
  );

  // Each press started exactly one download, and nothing went to the browser.
  expect(await downloadRequests(page)).toEqual([ARM64_DMG_URL, ARM64_DMG_URL]);
  expect(await openedUrls(page)).toEqual([]);
});

test("deprecation banner: the download button opens the website while the manifest is still loading", async ({
  page,
}) => {
  let releaseManifest = () => {};
  const manifestReleased = new Promise<void>((resolve) => {
    releaseManifest = resolve;
  });
  await page.route(BUZZ_APP_DOWNLOADS_MANIFEST_URL, async (route) => {
    await manifestReleased;
    await route.fulfill(MANIFEST_RESPONSE);
  });
  await installMockBridge(
    page,
    { newAppInstallerTarget: "macos-arm64" },
    { seedDeprecationBannerDismissed: false },
  );
  const manifestRequested = page.waitForRequest(
    BUZZ_APP_DOWNLOADS_MANIFEST_URL,
  );
  await page.goto("/");

  const download = page
    .getByTestId("deprecation-banner")
    .getByRole("button", { name: /^Upgrade,/ });
  // The request is in flight and unanswered: the click does not wait for it.
  await manifestRequested;
  await download.click();
  expect(await openedUrls(page)).toEqual([WEBSITE_URL]);
  expect(await downloadRequests(page)).toEqual([]);
  await expect(download).toHaveAttribute("data-download-source", "fallback");

  // Once answered, the same button downloads the installer in the app.
  releaseManifest();
  await expect(download).toHaveAttribute("data-download-source", "installer");
  await download.click();
  await expect(
    page.getByRole("dialog", { name: "Downloading the new Buzz app" }),
  ).toBeVisible();
  expect(await downloadRequests(page)).toEqual([ARM64_DMG_URL]);
  expect(await openedUrls(page)).toEqual([WEBSITE_URL]);
});
