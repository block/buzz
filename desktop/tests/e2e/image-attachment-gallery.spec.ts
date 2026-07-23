import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";

const IMAGE_SHAS = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
const SPOILER_VISIBLE_SHA = "d".repeat(64);
const SPOILER_HIDDEN_SHA = "e".repeat(64);
const SPOILER_VISIBLE_URL = `http://localhost:3000/media/${SPOILER_VISIBLE_SHA}.png`;
const SPOILER_HIDDEN_URL = `http://localhost:3000/media/${SPOILER_HIDDEN_SHA}.png`;
const NO_DIM_WIDE_URL = "https://example.com/e2e/gallery-wide.png";
const NO_DIM_PORTRAIT_URL = "https://example.com/e2e/gallery-portrait.png";
const NO_DIM_SECOND_URL = "https://example.com/e2e/gallery-second.png";
const PROGRESSIVE_URL = "https://example.com/e2e/progressive-full.png";
const PROGRESSIVE_THUMB_URL = "https://example.com/e2e/progressive-thumb.jpg";

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
  thumb,
  url,
}: {
  dim: string;
  filename: string;
  sha: string;
  thumb?: string;
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
    ...(thumb ? [`thumb ${thumb}`] : []),
  ];
}

async function installNoDimImageRoutes(page: Page) {
  await page.route("https://example.com/e2e/gallery-*.png", (route) => {
    const requestedUrl = route.request().url();
    const isPortrait = requestedUrl.includes("portrait");
    const isSecond = requestedUrl.includes("second");
    const width = isPortrait ? 120 : 320;
    const height = isPortrait ? 320 : 120;
    const fill = isSecond ? "#a78bfa" : isPortrait ? "#f4b860" : "#4aa3df";
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

  const mosaic = row.locator("[data-image-mosaic]");
  await expect(mosaic).toHaveAttribute("data-image-mosaic-count", "3");
  await expectSmoothCorners(mosaic);
  const mosaicCornerRadius = await mosaic.evaluate(
    (element) => window.getComputedStyle(element).borderTopLeftRadius,
  );
  const mosaicBox = await mosaic.boundingBox();
  const firstBox = await triggers.first().boundingBox();
  const secondBox = await triggers.nth(1).boundingBox();
  const thirdBox = await triggers.nth(2).boundingBox();
  if (!mosaicBox || !firstBox || !secondBox || !thirdBox) {
    throw new Error("Expected image mosaic tiles to have layout boxes");
  }
  expect(mosaicBox.width).toBeCloseTo(512, 0);
  expect(firstBox.height).toBeGreaterThan(secondBox.height * 1.8);
  expect(firstBox.height).toBeCloseTo(
    thirdBox.y + thirdBox.height - secondBox.y,
    0,
  );
  expect(secondBox.x).toBeCloseTo(thirdBox.x, 0);
  expect(secondBox.y).toBeLessThan(thirdBox.y);

  await expectCornerRadiusPx(mosaic, 16);
  await expectCornerRadiusPx(triggers.first(), 0);
  await expect(triggers.first().locator("img")).toHaveCSS(
    "object-fit",
    "cover",
  );
  await expectCornerRadiusPx(triggers.first().locator("img"), 0);
  await triggers.first().click();

  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src*="${IMAGE_SHAS[0]}"]`)).toBeVisible();
  const lightboxSurface = page
    .locator("[data-image-lightbox-frame] > div > div")
    .first();
  await expectCornerRadiusPx(lightboxSurface, 16);
  await expectSmoothCorners(lightboxSurface);
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toHaveCount(0);

  await page.getByRole("button", { name: "Next image" }).click();
  await expect(dialog.locator(`img[src*="${IMAGE_SHAS[1]}"]`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toBeVisible();

  await page.keyboard.press("ArrowRight");
  const currentLightboxImage = dialog.locator(`img[src*="${IMAGE_SHAS[2]}"]`);
  const lightboxFrame = page.locator("[data-image-lightbox-frame]");
  await expect(currentLightboxImage).toBeVisible();
  await expect(currentLightboxImage).toHaveCSS("object-fit", "contain");
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
  await expect(currentLightboxImage).toHaveCSS("object-fit", "cover");
  const closingFrameStyle = await lightboxFrame.evaluate((element) => {
    if (!(element instanceof HTMLElement)) {
      throw new Error("Expected HTML lightbox frame");
    }
    return {
      borderBottomLeftRadius: element.style.borderBottomLeftRadius,
      borderBottomRightRadius: element.style.borderBottomRightRadius,
      borderTopLeftRadius: element.style.borderTopLeftRadius,
      borderTopRightRadius: element.style.borderTopRightRadius,
      height: Number.parseFloat(element.style.height),
      left: Number.parseFloat(element.style.left),
      top: Number.parseFloat(element.style.top),
      transitionProperty: element.style.transitionProperty,
      width: Number.parseFloat(element.style.width),
    };
  });
  expect(closingFrameStyle.borderTopLeftRadius).toBe("0px");
  expect(closingFrameStyle.borderTopRightRadius).toBe("0px");
  expect(closingFrameStyle.borderBottomLeftRadius).toBe("0px");
  expect(closingFrameStyle.borderBottomRightRadius).toBe(mosaicCornerRadius);
  expect(closingFrameStyle.transitionProperty).toContain("border-radius");

  const lightboxSurfaceStyle = await page
    .locator("[data-image-lightbox-frame] > div > div")
    .first()
    .evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        throw new Error("Expected HTML lightbox surface");
      }
      return {
        borderBottomRightRadius: element.style.borderBottomRightRadius,
        borderTopLeftRadius: element.style.borderTopLeftRadius,
        transitionProperty: element.style.transitionProperty,
      };
    });
  expect(lightboxSurfaceStyle.borderTopLeftRadius).toBe("0px");
  expect(lightboxSurfaceStyle.borderBottomRightRadius).toBe(mosaicCornerRadius);
  expect(lightboxSurfaceStyle.transitionProperty).toBe("border-radius");

  expect(Math.abs(closingFrameStyle.left - currentThumbnailBox.x)).toBeLessThan(
    2,
  );
  expect(Math.abs(closingFrameStyle.top - currentThumbnailBox.y)).toBeLessThan(
    2,
  );
  expect(
    Math.abs(closingFrameStyle.width - currentThumbnailBox.width),
  ).toBeLessThan(2);
  expect(
    Math.abs(closingFrameStyle.height - currentThumbnailBox.height),
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

test("message images load a thumbnail before requesting the original", async ({
  page,
}) => {
  let fullRequested = false;
  let releaseThumbnail: (() => void) | undefined;
  let releaseFull: (() => void) | undefined;
  const thumbnailGate = new Promise<void>((resolve) => {
    releaseThumbnail = resolve;
  });
  const fullGate = new Promise<void>((resolve) => {
    releaseFull = resolve;
  });
  const svg = (fill: string) =>
    `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="100%" height="100%" fill="${fill}"/></svg>`;

  await page.route(PROGRESSIVE_THUMB_URL, async (route) => {
    await thumbnailGate;
    await route.fulfill({ body: svg("#f4b860"), contentType: "image/svg+xml" });
  });
  await page.route(PROGRESSIVE_URL, async (route) => {
    fullRequested = true;
    await fullGate;
    await route.fulfill({ body: svg("#4aa3df"), contentType: "image/svg+xml" });
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await waitForMockLiveSubscription(page, "general");
  await page.evaluate(
    ({ content, extraTags }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            extraTags: string[][];
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        extraTags,
      });
    },
    {
      content: `progressive image\n![progressive](${PROGRESSIVE_URL})`,
      extraTags: [
        imageImetaTag({
          dim: "320x200",
          filename: "progressive.png",
          sha: "f".repeat(64),
          thumb: PROGRESSIVE_THUMB_URL,
          url: PROGRESSIVE_URL,
        }),
      ],
    },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "progressive image" })
    .last();
  const trigger = row.getByTestId("message-image-lightbox-trigger");
  await expect(
    trigger.locator(`img[src="${PROGRESSIVE_THUMB_URL}"]`),
  ).toHaveCount(1);
  await expect(trigger.locator(`img[src="${PROGRESSIVE_URL}"]`)).toHaveCount(0);
  expect(fullRequested).toBe(false);
  const before = await trigger.boundingBox();
  const rowBefore = await row.boundingBox();

  releaseThumbnail?.();
  const full = trigger.locator(`img[src="${PROGRESSIVE_URL}"]`);
  await expect(full).toHaveCount(1);
  await expect(full).toHaveClass(/opacity-0/);
  expect(fullRequested).toBe(true);
  releaseFull?.();
  await expect(full).toBeVisible();
  await expect(full).not.toHaveClass(/opacity-0/);
  expect(await trigger.boundingBox()).toEqual(before);
  expect(await row.boundingBox()).toEqual(rowBefore);
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

test("forum markdown images use the markdown root as their gallery scope", async ({
  page,
}) => {
  await installNoDimImageRoutes(page);
  await page.goto("/");
  await expect
    .poll(() => {
      return page.evaluate(() => {
        return (
          typeof (
            window as Window & {
              __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: unknown;
            }
          ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function"
        );
      });
    })
    .toBe(true);

  const postId = await page.evaluate(
    ({ content }) => {
      const event = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            kind: number;
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "watercooler",
        content,
        kind: 45001,
      });
      return event?.id ?? null;
    },
    {
      content: [
        "forum gallery scope",
        `![wide](${NO_DIM_WIDE_URL})`,
        `![portrait](${NO_DIM_PORTRAIT_URL})`,
      ].join("\n"),
    },
  );
  expect(postId).not.toBeNull();

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  await page
    .getByRole("button")
    .filter({ hasText: "forum gallery scope" })
    .first()
    .getByText("forum gallery scope")
    .click();

  const threadPost = page.locator(`[data-forum-event-id="${postId}"]`);
  await expect(threadPost).toBeVisible();
  const triggers = threadPost.getByTestId("message-image-lightbox-trigger");
  await expect(triggers).toHaveCount(2);
  await expect
    .poll(() =>
      triggers.first().evaluate((trigger) => {
        return trigger.closest("[data-testid='message-row']") !== null;
      }),
    )
    .toBe(false);

  await triggers.first().click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(`img[src="${NO_DIM_WIDE_URL}"]`)).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Next image" })).toBeVisible();

  await page.getByRole("button", { name: "Next image" }).click();
  await expect(
    dialog.locator(`img[src="${NO_DIM_PORTRAIT_URL}"]`),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Previous image" }),
  ).toBeVisible();
});

test("multi-image mosaics keep a fixed width and grow by rows", async ({
  page,
}) => {
  await installNoDimImageRoutes(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  const urls = Array.from(
    { length: 5 },
    (_, index) =>
      `https://example.com/e2e/gallery-${index % 2 === 0 ? "wide" : "portrait"}.png?item=${index}`,
  );

  await page.evaluate((imageUrls) => {
    const emit = (
      window as Window & {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
        }) => unknown;
      }
    ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;

    for (const count of [2, 4, 5]) {
      emit?.({
        channelName: "general",
        content: [
          `${count} image mosaic`,
          ...imageUrls.slice(0, count).map((url) => `![image](${url})`),
        ].join("\n"),
      });
    }
  }, urls);

  const mosaics: Array<{ count: number; height: number; width: number }> = [];
  for (const count of [2, 4, 5]) {
    const row = page
      .getByTestId("message-row")
      .filter({ hasText: `${count} image mosaic` })
      .last();
    const mosaic = row.locator("[data-image-mosaic]");
    await expect(mosaic).toHaveAttribute(
      "data-image-mosaic-count",
      String(count),
    );
    const box = await mosaic.boundingBox();
    if (!box) throw new Error(`Expected ${count}-image mosaic layout box`);
    mosaics.push({ count, height: box.height, width: box.width });
  }

  expect(mosaics[0].width).toBeCloseTo(mosaics[1].width, 0);
  expect(mosaics[1].width).toBeCloseTo(mosaics[2].width, 0);
  expect(mosaics[1].height).toBeGreaterThan(mosaics[0].height);
  expect(mosaics[2].height).toBeGreaterThan(mosaics[1].height);
  expect(mosaics[2].height - mosaics[1].height).toBeCloseTo(
    mosaics[0].height + 6,
    0,
  );
});

test("image mosaic screenshot", async ({ page }) => {
  await installNoDimImageRoutes(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.evaluate(
    ({ portraitUrl, secondUrl, wideUrl }) => {
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: [
          "Weekend photo dump",
          `![Coastal overlook](${wideUrl})`,
          `![Boardwalk detail](${portraitUrl})`,
          `![Golden hour](${secondUrl})`,
        ].join("\n"),
      });
    },
    {
      portraitUrl: NO_DIM_PORTRAIT_URL,
      secondUrl: NO_DIM_SECOND_URL,
      wideUrl: NO_DIM_WIDE_URL,
    },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "Weekend photo dump" })
    .last();
  await expect(row.locator("[data-image-mosaic] img")).toHaveCount(3);
  await waitForAnimations(page);
  await row.screenshot({
    path: "test-results/image-mosaic/three-image-mosaic.png",
  });
});

test("mosaic image context menu is portaled outside the clipped gallery", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("mosaic context menu");
  await page.getByRole("button", { name: "Attach image" }).click();
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "mosaic context menu" })
    .last();
  const mosaic = row.locator("[data-image-mosaic]");
  const trigger = row.getByTestId("message-image-lightbox-trigger").last();
  await expect(mosaic).toBeVisible();
  await trigger.click({ button: "right" });

  const menu = page.locator("[data-image-context-menu]");
  await expect(menu).toBeVisible();
  await expect(page.getByRole("button", { name: "Copy image" })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Download image" }),
  ).toBeVisible();
  await expect(mosaic.locator("[data-image-context-menu]")).toHaveCount(0);
  expect(
    await menu.evaluate((element) => element.parentElement === document.body),
  ).toBe(true);

  const mosaicBox = await mosaic.boundingBox();
  const menuBox = await menu.boundingBox();
  if (!mosaicBox || !menuBox) {
    throw new Error("Expected mosaic and image context menu layout boxes");
  }
  expect(menuBox.x + menuBox.width).toBeGreaterThan(
    mosaicBox.x + mosaicBox.width,
  );
});

test("lightbox image context menu stays inside the dialog focus scope", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("lightbox context menu");
  await page.getByRole("button", { name: "Attach image" }).click();
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "lightbox context menu" })
    .last();
  await row.getByTestId("message-image-lightbox-trigger").first().click();

  const dialog = page.getByRole("dialog");
  const lightboxImage = dialog.locator(`img[src*="${IMAGE_SHAS[0]}"]`);
  await expect(lightboxImage).toBeVisible();
  await lightboxImage.click({ button: "right" });

  const menu = dialog.locator("[data-image-context-menu]");
  const copyButton = menu.getByRole("button", { name: "Copy image" });
  const downloadButton = menu.getByRole("button", { name: "Download image" });
  await expect(menu).toBeVisible();
  await expect(page.locator("body > [data-image-context-menu]")).toHaveCount(0);

  await dialog.focus();
  await page.keyboard.press("Shift+Tab");
  await expect(downloadButton).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(copyButton).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(downloadButton).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: "Next image" }),
  ).toBeFocused();
});

test("right-click image shows Copy image and invokes copy command", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("copy me");
  await page.getByRole("button", { name: "Attach image" }).click();
  await page.getByTestId("send-message").click();
  await expect(page.getByText("Sending")).toHaveCount(0);

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "copy me" })
    .last();
  const trigger = row.getByTestId("message-image-lightbox-trigger").first();
  await expect(trigger).toBeVisible();

  await trigger.click({ button: "right" });

  const copyButton = page.getByRole("button", { name: "Copy image" });
  await expect(copyButton).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Download image" }),
  ).toBeVisible();

  // The image menu carries the shared generic marker plus its own image-
  // specific attribute, and never the link/video attributes — so e2e locators
  // can target one surface without aliasing another.
  await expect(page.locator("[data-media-context-menu]")).toBeVisible();
  await expect(page.locator("[data-image-context-menu]")).toBeVisible();
  await expect(page.locator("[data-link-context-menu]")).toHaveCount(0);
  await expect(page.locator("[data-video-context-menu]")).toHaveCount(0);

  await copyButton.click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("copy_image_to_clipboard");
});
