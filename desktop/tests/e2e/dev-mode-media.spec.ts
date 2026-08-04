import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Developer mode must support the full attachment flow: paste an image or
// video into either composer (main channel or side chat), see the pending
// chip, send, and get an interactive media block in the transcript — the
// same lightbox/video-player path the standard renderer uses.

const IMAGE_SHA = "a".repeat(64);
const IMAGE_URL = `http://localhost:3000/media/${IMAGE_SHA}.png`;
const IMAGE_DESCRIPTOR = {
  url: IMAGE_URL,
  sha256: IMAGE_SHA,
  size: 4096,
  type: "image/png",
  uploaded: 1_700_000_000,
  dim: "64x64",
  filename: "screenshot.png",
};

const VIDEO_SHA = "b".repeat(64);
const VIDEO_URL = `http://localhost:3000/media/${VIDEO_SHA}.mp4`;
const VIDEO_DESCRIPTOR = {
  url: VIDEO_URL,
  sha256: VIDEO_SHA,
  size: 8192,
  type: "video/mp4",
  uploaded: 1_700_000_000,
  filename: "clip.mp4",
};

/** Dispatch a real ClipboardEvent carrying a File — Playwright's
 * dispatchEvent falls back to a bare Event for "paste", which drops
 * clipboardData, so the event must be constructed in the page. */
async function pasteFile(
  page: Page,
  testId: string,
  filename: string,
  mime: string,
) {
  await page.evaluate(
    ({ testId, filename, mime }) => {
      const target = document.querySelector(`[data-testid="${testId}"]`);
      if (!target) throw new Error(`missing ${testId}`);
      const transfer = new DataTransfer();
      transfer.items.add(
        new File([new Uint8Array([137, 80, 78, 71])], filename, {
          type: mime,
        }),
      );
      target.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData: transfer,
        }),
      );
    },
    { testId, filename, mime },
  );
}

async function openDevModeChannel(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await composer.focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();
  return composer;
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
});

test("pasted image attaches, is removable, and renders interactively", async ({
  page,
}) => {
  await installMockBridge(page, { uploadDescriptors: [IMAGE_DESCRIPTOR] });
  const composer = await openDevModeChannel(page);

  // Paste → pending chip with the filename.
  await pasteFile(page, "dev-mode-composer", "screenshot.png", "image/png");
  const chip = page.getByTestId("dev-mode-attachment-chip");
  await expect(chip).toBeVisible();
  await expect(chip).toContainText("screenshot.png");

  // Remove → chip disappears; nothing is sent.
  await chip.getByRole("button", { name: "remove attachment" }).click();
  await expect(chip).toHaveCount(0);

  // Paste again and send with text.
  await pasteFile(page, "dev-mode-composer", "screenshot.png", "image/png");
  await expect(page.getByTestId("dev-mode-attachment-chip")).toBeVisible();
  await composer.fill("here is a screenshot");
  await page.keyboard.press("Enter");

  // Chip clears and the transcript shows the standard interactive image
  // block (lightbox trigger), not a raw markdown line.
  await expect(page.getByTestId("dev-mode-attachment-chip")).toHaveCount(0);
  const transcript = page.getByTestId("dev-mode-transcript");
  await expect(
    transcript.getByText("here is a screenshot", { exact: false }),
  ).toBeVisible();
  await expect(
    transcript.getByTestId("message-image-lightbox-trigger"),
  ).toBeVisible();
  await expect(transcript).not.toContainText(`![image](${IMAGE_URL})`);

  // Media must not push the pane wider than its box.
  const widths = await transcript.evaluate((el) => ({
    scrollWidth: el.scrollWidth,
    clientWidth: el.clientWidth,
  }));
  expect(widths.scrollWidth).toBeLessThanOrEqual(widths.clientWidth + 1);
});

test("attachment-only send posts the image to the channel", async ({
  page,
}) => {
  await installMockBridge(page, { uploadDescriptors: [IMAGE_DESCRIPTOR] });
  await openDevModeChannel(page);

  await pasteFile(page, "dev-mode-composer", "screenshot.png", "image/png");
  await expect(page.getByTestId("dev-mode-attachment-chip")).toBeVisible();
  await page.keyboard.press("Enter");

  await expect(page.getByTestId("dev-mode-attachment-chip")).toHaveCount(0);
  await expect(
    page
      .getByTestId("dev-mode-transcript")
      .getByTestId("message-image-lightbox-trigger"),
  ).toBeVisible();
});

test("enter during an in-flight upload does not send without the attachment", async ({
  page,
}) => {
  await installMockBridge(page, {
    uploadDescriptors: [IMAGE_DESCRIPTOR],
    uploadDelayMs: 1_500,
  });
  const composer = await openDevModeChannel(page);

  await pasteFile(page, "dev-mode-composer", "screenshot.png", "image/png");
  await expect(page.getByTestId("dev-mode-attachment-uploading")).toBeVisible();
  await composer.fill("wait for the upload");
  await page.keyboard.press("Enter");

  // The send is held: the input keeps its text while the upload runs.
  await expect(composer).toHaveValue("wait for the upload");

  // Once the upload settles into a chip, Enter sends text + image together.
  await expect(page.getByTestId("dev-mode-attachment-chip")).toBeVisible();
  await page.keyboard.press("Enter");
  const transcript = page.getByTestId("dev-mode-transcript");
  await expect(
    transcript.getByText("wait for the upload", { exact: false }),
  ).toBeVisible();
  await expect(
    transcript.getByTestId("message-image-lightbox-trigger"),
  ).toBeVisible();
});

test("pasted video sends from the side chat and renders with controls", async ({
  page,
}) => {
  await installMockBridge(page, { uploadDescriptors: [VIDEO_DESCRIPTOR] });
  const composer = await openDevModeChannel(page);

  // Post a prompt to create a card, then open its side chat.
  await composer.fill("check the video flow");
  await page.keyboard.press("Enter");
  await expect(
    page
      .getByTestId("dev-mode-transcript")
      .getByText("check the video flow", { exact: false }),
  ).toBeVisible();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  const panel = page.getByTestId("dev-mode-thread-panel");
  await panel.waitFor();

  await pasteFile(page, "dev-mode-thread-composer", "clip.mp4", "video/mp4");
  const chip = panel.getByTestId("dev-mode-attachment-chip");
  await expect(chip).toBeVisible();
  await expect(chip).toContainText("clip.mp4");

  await page.getByTestId("dev-mode-thread-composer").fill("video attached");
  await page.keyboard.press("Enter");

  await expect(panel.getByTestId("dev-mode-attachment-chip")).toHaveCount(0);
  await expect(
    panel.getByText("video attached", { exact: false }),
  ).toBeVisible();
  await expect(panel.getByTestId("video-player")).toBeVisible();
});
