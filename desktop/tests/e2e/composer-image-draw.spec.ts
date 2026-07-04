import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const ORIGINAL_SHA = "a".repeat(64);
const EDITED_SHA = "b".repeat(64);
const ORIGINAL_URL = "https://example.com/e2e/draw-original.svg";
const EDITED_URL = "https://example.com/e2e/draw-edited.svg";

const ORIGINAL_DESCRIPTOR = {
  url: ORIGINAL_URL,
  sha256: ORIGINAL_SHA,
  size: 1234,
  type: "image/svg+xml",
  uploaded: Math.floor(Date.now() / 1000),
  dim: "320x200",
  filename: "draw-original.svg",
};

const EDITED_DESCRIPTOR = {
  url: EDITED_URL,
  sha256: EDITED_SHA,
  size: 2345,
  type: "image/png",
  uploaded: Math.floor(Date.now() / 1000),
  dim: "320x200",
  filename: "draw-original.png",
};

/**
 * Serve deterministic same-size SVGs for both attachment URLs. These back
 * the display <img> loads and the mock bridge's `fetch_media_bytes`
 * handler (the editor exports via IPC bytes + blob: URL, so no CORS
 * headers are needed). The CORS header is required only because the mock
 * bridge's in-page `fetch()` of this cross-origin URL is CORS-mode —
 * production fetches the bytes in Rust instead.
 */
async function installImageRoutes(page: Page) {
  await page.route("https://example.com/e2e/draw-*.svg*", (route) => {
    const fill = route.request().url().includes("edited")
      ? "#b3574a"
      : "#4aa3df";
    route.fulfill({
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 320 200"><rect width="100%" height="100%" fill="${fill}"/></svg>`,
      contentType: "image/svg+xml",
      headers: { "access-control-allow-origin": "*" },
    });
  });
}

async function drawStrokeOnCanvas(page: Page) {
  const canvas = page.getByTestId("composer-image-editor-canvas");
  await expect(canvas).toBeVisible();
  const box = await canvas.boundingBox();
  if (!box) throw new Error("Expected drawing canvas to have a layout box");
  const centerY = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * 0.25, centerY);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.75, centerY, { steps: 8 });
  await page.mouse.up();
}

test.beforeEach(async ({ page }) => {
  await installImageRoutes(page);
  await installMockBridge(page, {
    uploadDescriptors: [ORIGINAL_DESCRIPTOR],
  });
});

test("draw on an uploaded image, save replaces it, revert restores in place", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Attach the original image via the mocked paperclip flow.
  await page.getByRole("button", { name: "Attach image" }).click();
  const composer = page.getByTestId("message-composer");
  await expect(composer.getByAltText("Attachment aaaa")).toBeVisible();

  // Open the composer lightbox.
  await composer.getByAltText("Attachment aaaa").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src="${ORIGINAL_URL}"]`)).toBeVisible();

  // No revert affordance before any edit.
  await expect(page.getByTestId("composer-attachment-revert")).toHaveCount(0);

  // Enter canvas mode; Escape leaves canvas mode but keeps the dialog open.
  await page.getByTestId("composer-attachment-edit").click();
  await expect(page.getByTestId("composer-image-editor-canvas")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("composer-image-editor-canvas")).toHaveCount(0);
  await expect(dialog).toBeVisible();

  // Re-enter canvas mode and draw a stroke.
  await page.getByTestId("composer-attachment-edit").click();
  const saveButton = page.getByTestId("composer-image-editor-save");
  await expect(saveButton).toBeDisabled();
  await drawStrokeOnCanvas(page);
  await expect(saveButton).toBeEnabled();

  // The next mocked upload returns the annotated descriptor.
  await page.evaluate((edited) => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: {
        ...window.__BUZZ_E2E__?.mock,
        uploadDescriptors: [edited],
      },
    };
  }, EDITED_DESCRIPTOR);

  await saveButton.click();

  // Saving closes the lightbox; the composer thumbnail now shows the
  // annotated image.
  await expect(dialog).toHaveCount(0);
  await expect(composer.getByAltText("Attachment bbbb")).toBeVisible();

  // The annotated PNG went through the real upload command.
  const uploadCommandCount = await page.evaluate(
    () =>
      (
        window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
      ).__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "upload_media_bytes",
      ).length ?? 0,
  );
  expect(uploadCommandCount).toBe(1);

  // Reopen the lightbox on the annotated attachment to revert.
  await composer.getByAltText("Attachment bbbb").click();
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src="${EDITED_URL}"]`)).toBeVisible();

  // Revert swaps back to the original without closing the dialog.
  await page.getByTestId("composer-attachment-revert").click();
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src="${ORIGINAL_URL}"]`)).toBeVisible();
  await expect(page.getByTestId("composer-attachment-revert")).toHaveCount(0);

  // Closing the dialog shows the (restored) original thumbnail.
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(composer.getByAltText("Attachment aaaa")).toBeVisible();
});

test("spoiler marking survives drawing on the attachment", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByRole("button", { name: "Attach image" }).click();
  const composer = page.getByTestId("message-composer");
  await expect(composer.getByAltText("Attachment aaaa")).toBeVisible();

  // Spoiler the attachment from its lightbox (media spoilers are
  // per-attachment; the text spoiler control no longer affects media),
  // then draw on it.
  await composer.getByAltText("Attachment aaaa").click();
  await page.getByTestId("composer-attachment-spoiler").click();
  await page.keyboard.press("Escape");
  await expect(composer.locator("[data-composer-media-spoiler]")).toBeVisible();

  await composer.getByAltText("Attachment aaaa").click();
  await page.getByTestId("composer-attachment-edit").click();
  await drawStrokeOnCanvas(page);

  await page.evaluate((edited) => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: {
        ...window.__BUZZ_E2E__?.mock,
        uploadDescriptors: [edited],
      },
    };
  }, EDITED_DESCRIPTOR);
  await page.getByTestId("composer-image-editor-save").click();

  // Saving closes the lightbox.
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // The annotated replacement is still marked as a spoiler.
  await expect(composer.getByAltText("Attachment bbbb")).toBeVisible();
  await expect(composer.locator("[data-composer-media-spoiler]")).toBeVisible();
});
