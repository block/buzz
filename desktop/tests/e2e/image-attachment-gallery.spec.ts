import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const IMAGE_SHAS = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    uploadDescriptors: [
      {
        url: `http://localhost:3000/media/${IMAGE_SHAS[0]}.png`,
        sha256: IMAGE_SHAS[0],
        size: 1234,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        dim: "160x100",
        filename: "first.png",
      },
      {
        url: `http://localhost:3000/media/${IMAGE_SHAS[1]}.png`,
        sha256: IMAGE_SHAS[1],
        size: 2345,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        dim: "100x160",
        filename: "second.png",
      },
      {
        url: `http://localhost:3000/media/${IMAGE_SHAS[2]}.png`,
        sha256: IMAGE_SHAS[2],
        size: 3456,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        dim: "140x140",
        filename: "third.png",
      },
    ],
  });
});

test("image bundle lightbox navigates as a gallery", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("gallery bundle");
  await page.getByRole("button", { name: "Attach image" }).click();
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "gallery bundle" })
    .last();
  await expect(row).toBeVisible();

  const triggers = row.getByTestId("message-image-lightbox-trigger");
  await expect(triggers).toHaveCount(3);
  await triggers.first().click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src*="${IMAGE_SHAS[0]}"]`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toHaveCount(0);

  await page.getByRole("button", { name: "Next image" }).click();
  await expect(dialog.locator(`img[src*="${IMAGE_SHAS[1]}"]`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toBeVisible();

  await page.keyboard.press("ArrowRight");
  await expect(dialog.locator(`img[src*="${IMAGE_SHAS[2]}"]`)).toBeVisible();
  await expect(page.getByRole("button", { name: "Next image" })).toHaveCount(0);

  const currentThumbnailBox = await triggers
    .nth(2)
    .locator("img")
    .boundingBox();
  if (!currentThumbnailBox) {
    throw new Error("Expected current gallery thumbnail to have a layout box");
  }

  await page.waitForTimeout(500);
  await page.mouse.click(20, 20);
  await page.waitForTimeout(200);

  const closingFrameBox = await page
    .locator("[data-image-lightbox-frame]")
    .boundingBox();
  if (!closingFrameBox) {
    throw new Error("Expected lightbox frame to remain mounted while closing");
  }

  expect(Math.abs(closingFrameBox.x - currentThumbnailBox.x)).toBeLessThan(2);
  expect(Math.abs(closingFrameBox.y - currentThumbnailBox.y)).toBeLessThan(2);
  expect(
    Math.abs(closingFrameBox.width - currentThumbnailBox.width),
  ).toBeLessThan(2);
  expect(
    Math.abs(closingFrameBox.height - currentThumbnailBox.height),
  ).toBeLessThan(2);
});
