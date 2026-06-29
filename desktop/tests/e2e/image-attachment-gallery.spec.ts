import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const IMAGE_SHAS = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
const SPOILER_VISIBLE_SHA = "d".repeat(64);
const SPOILER_HIDDEN_SHA = "e".repeat(64);
const SPOILER_VISIBLE_URL = `http://localhost:3000/media/${SPOILER_VISIBLE_SHA}.png`;
const SPOILER_HIDDEN_URL = `http://localhost:3000/media/${SPOILER_HIDDEN_SHA}.png`;
const NO_DIM_WIDE_URL = "https://example.com/e2e/gallery-wide.png";
const NO_DIM_PORTRAIT_URL = "https://example.com/e2e/gallery-portrait.png";

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () => {
      return page.evaluate((name) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
          }) ?? false
        );
      }, channelName);
    })
    .toBe(true);
}

function imageImetaTag({
  dim,
  filename,
  sha,
  url,
}: {
  dim: string;
  filename: string;
  sha: string;
  url: string;
}) {
  return [
    "imeta",
    `url ${url}`,
    "m image/png",
    `x ${sha}`,
    "size 1234",
    `dim ${dim}`,
    `filename ${filename}`,
  ];
}

async function installNoDimImageRoutes(page: Page) {
  await page.route("https://example.com/e2e/gallery-*.png", (route) => {
    const isPortrait = route.request().url().includes("portrait");
    const width = isPortrait ? 120 : 320;
    const height = isPortrait ? 320 : 120;
    const fill = isPortrait ? "#f4b860" : "#4aa3df";
    route.fulfill({
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="100%" height="100%" fill="${fill}"/></svg>`,
      contentType: "image/svg+xml",
    });
  });
}

async function getLightboxFrameBox(page: Page) {
  const box = await page.locator("[data-image-lightbox-frame]").boundingBox();
  if (!box) {
    throw new Error("Expected lightbox frame to have a layout box");
  }
  return box;
}

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

test("hidden spoiler images are excluded from gallery navigation until revealed", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.evaluate(
    ({ content, extraTags }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            extraTags?: string[][];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags,
      });
    },
    {
      content: [
        "spoiler gallery",
        `![visible](${SPOILER_VISIBLE_URL})`,
        `||![hidden](${SPOILER_HIDDEN_URL})||`,
      ].join("\n"),
      extraTags: [
        imageImetaTag({
          dim: "160x100",
          filename: "visible.png",
          sha: SPOILER_VISIBLE_SHA,
          url: SPOILER_VISIBLE_URL,
        }),
        imageImetaTag({
          dim: "100x160",
          filename: "hidden.png",
          sha: SPOILER_HIDDEN_SHA,
          url: SPOILER_HIDDEN_URL,
        }),
      ],
    },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "spoiler gallery" })
    .last();
  await expect(row).toBeVisible();

  await row.locator(`img[src*="${SPOILER_VISIBLE_SHA}"]`).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("button", { name: "Next image" })).toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);

  const spoiler = row.locator(".buzz-spoiler[data-spoiler]").first();
  await expect(spoiler).toHaveAttribute("data-revealed", "false");
  await spoiler.click();
  await expect(spoiler).toHaveAttribute("data-revealed", "true");

  await row.locator(`img[src*="${SPOILER_VISIBLE_SHA}"]`).click();
  await expect(dialog).toBeVisible();
  await expect(page.getByRole("button", { name: "Next image" })).toBeVisible();
});

test("gallery items without imeta dimensions keep their thumbnail aspect ratio", async ({
  page,
}) => {
  await installNoDimImageRoutes(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.evaluate(
    ({ content }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
      });
    },
    {
      content: [
        "no dim gallery",
        `![wide](${NO_DIM_WIDE_URL})`,
        `![portrait](${NO_DIM_PORTRAIT_URL})`,
      ].join("\n"),
    },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "no dim gallery" })
    .last();
  await expect(row).toBeVisible();
  await expect(row.locator(`img[src="${NO_DIM_WIDE_URL}"]`)).toBeVisible();
  await expect(row.locator(`img[src="${NO_DIM_PORTRAIT_URL}"]`)).toBeVisible();

  await row.locator(`img[src="${NO_DIM_WIDE_URL}"]`).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src="${NO_DIM_WIDE_URL}"]`)).toBeVisible();
  await page.waitForTimeout(350);
  const wideFrameBox = await getLightboxFrameBox(page);
  expect(wideFrameBox.width / wideFrameBox.height).toBeGreaterThan(2);

  await page.getByRole("button", { name: "Next image" }).click();
  await expect(
    dialog.locator(`img[src="${NO_DIM_PORTRAIT_URL}"]`),
  ).toBeVisible();
  await page.waitForTimeout(350);
  const portraitFrameBox = await getLightboxFrameBox(page);
  expect(portraitFrameBox.width / portraitFrameBox.height).toBeLessThan(0.6);
});
