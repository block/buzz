import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const MEDIA_PROXY_PORT = 54321;
const IMAGE_SHA = "f".repeat(64);
const RELAY_IMAGE_URL = `http://localhost:3000/media/${IMAGE_SHA}.png`;

test("mounted message image recovers when the media proxy becomes ready", async ({
  page,
}) => {
  await installMockBridge(page, {
    mediaProxyPortDelayMs: 4_000,
    uploadDescriptors: [
      {
        url: RELAY_IMAGE_URL,
        sha256: IMAGE_SHA,
        size: 1234,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        dim: "320x120",
        filename: "cold-start.png",
      },
    ],
  });
  await page.route(
    `http://127.0.0.1:${MEDIA_PROXY_PORT}/media/${IMAGE_SHA}.png`,
    (route) =>
      route.fulfill({
        body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120"><rect width="320" height="120" fill="#22c55e"/></svg>',
        contentType: "image/svg+xml",
      }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("message-input").fill("cold media proxy probe");
  await page.getByRole("button", { name: "Attach file" }).click();
  await page.getByTestId("send-message").click();

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "cold media proxy probe" })
    .last();
  const image = row.locator("img").last();
  await expect(image).toHaveAttribute(
    "src",
    `buzz-media://localhost/media/${IMAGE_SHA}.png`,
  );

  await expect(image).toHaveAttribute(
    "src",
    `http://127.0.0.1:${MEDIA_PROXY_PORT}/media/${IMAGE_SHA}.png`,
    { timeout: 8_000 },
  );
  await expect
    .poll(() =>
      image.evaluate((element) =>
        element instanceof HTMLImageElement ? element.naturalWidth : 0,
      ),
    )
    .toBe(320);
});
