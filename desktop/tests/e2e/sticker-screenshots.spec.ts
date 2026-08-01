import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { routeStickerAssets } from "../helpers/stickerAssets";

const OUT_DIR = "test-results/sticker-screenshots";

/** Assert an `<img>` decoded real bytes rather than rendering broken. */
async function expectImageLoaded(image: Locator): Promise<void> {
  await expect
    .poll(
      () =>
        image.evaluate((element) => (element as HTMLImageElement).naturalWidth),
      { timeout: 10_000 },
    )
    .toBeGreaterThan(0);
}

/**
 * Screenshot clip covering every locator in `locators`, padded and clamped to
 * the viewport.
 *
 * Element screenshots of a Radix popover crop away the composer it belongs to,
 * which makes the picker shots hard to read in a PR. Framing the popover
 * together with the composer keeps the feature legible.
 */
async function clipAround(
  page: Page,
  locators: Locator[],
  padding = 16,
): Promise<{ x: number; y: number; width: number; height: number }> {
  const boxes = [];
  for (const locator of locators) {
    const box = await locator.boundingBox();
    if (box) boxes.push(box);
  }
  if (boxes.length === 0) throw new Error("no visible element to clip around");
  const viewport = page.viewportSize() ?? { width: 1280, height: 720 };
  const left = Math.max(0, Math.min(...boxes.map((b) => b.x)) - padding);
  const top = Math.max(0, Math.min(...boxes.map((b) => b.y)) - padding);
  const right = Math.min(
    viewport.width,
    Math.max(...boxes.map((b) => b.x + b.width)) + padding,
  );
  const bottom = Math.min(
    viewport.height,
    Math.max(...boxes.map((b) => b.y + b.height)) + padding,
  );
  return { x: left, y: top, width: right - left, height: bottom - top };
}

async function openChannel(page: Page, name: string): Promise<void> {
  await page.goto("/");
  await page.getByTestId(`channel-${name}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(name);
}

/**
 * The sticker picker's portalled popover.
 *
 * Scoping matters: the composer's own "Send message" button also matches
 * `/^Send /`, so an unscoped role query silently picks it up and reports one
 * sticker too many (or, with `.first()`, resolves to a button with no image).
 */
function stickerPopover(page: Page): Locator {
  return page
    .locator("[data-radix-popper-content-wrapper]")
    .filter({ hasText: /No stickers installed|Sticker pack/ });
}

/**
 * Open the picker and wait until its grid images have actually decoded.
 *
 * `rewriteRelayUrl` falls back to `buzz-media://localhost/media/…` while the
 * media-proxy port is still resolving, and the picker reads it during render
 * without subscribing to port changes — so a grid that first renders inside
 * that window keeps a `buzz-media://` src forever. That scheme is a registered
 * custom protocol in the Tauri webview, but not in the e2e browser, where it
 * shows as a broken image. Reopening the popover re-renders it against the
 * now-cached port. Harness artefact, not a product bug.
 */
async function openStickerGrid(page: Page): Promise<void> {
  await page.getByTestId("composer-sticker-button").click();
  await expect(page.getByTestId("composer-sticker-pack-select")).toBeVisible();
  await expect
    .poll(
      async () => {
        const width = await stickerPopover(page)
          .locator("img")
          .first()
          .evaluate((element) => (element as HTMLImageElement).naturalWidth)
          .catch(() => 0);
        if (width > 0) return width;
        await page.keyboard.press("Escape");
        await page.getByTestId("composer-sticker-button").click();
        await expect(
          page.getByTestId("composer-sticker-pack-select"),
        ).toBeVisible();
        return 0;
      },
      { timeout: 15_000 },
    )
    .toBeGreaterThan(0);
}

test.describe("sonar stickers", () => {
  test("composer sticker picker renders the installed pack grid", async ({
    page,
  }) => {
    await installMockBridge(page);
    await routeStickerAssets(page);
    await openChannel(page, "engineering");

    await openStickerGrid(page);
    const packSelect = page.getByTestId("composer-sticker-pack-select");
    // Both catalog-approved packs are installed for the mock viewer.
    await expect(packSelect.locator("option")).toHaveCount(2);

    const grid = stickerPopover(page).getByRole("button", { name: /^Send / });
    await expect(grid).toHaveCount(10);
    await expectImageLoaded(grid.first().locator("img"));
    await expectImageLoaded(grid.last().locator("img"));

    await waitForAnimations(page);
    await page.screenshot({
      path: `${OUT_DIR}/01-composer-picker-grid.png`,
      clip: await clipAround(page, [
        stickerPopover(page),
        page.getByTestId("composer-sticker-button"),
      ]),
    });
  });

  test("a sent sticker renders in the message timeline", async ({ page }) => {
    await installMockBridge(page);
    await routeStickerAssets(page);
    await openChannel(page, "engineering");

    await openStickerGrid(page);
    await stickerPopover(page)
      .getByRole("button", { name: "Send ship_it" })
      .click();

    const sticker = page.getByTestId("sonar-sticker");
    await expect(sticker).toHaveCount(1);
    await expectImageLoaded(sticker);
    // The sticker branch replaces the body entirely — no "Sticker unavailable".
    await expect(page.getByText("Sticker unavailable")).toHaveCount(0);

    const row = page
      .getByTestId("message-row")
      .filter({ has: page.getByTestId("sonar-sticker") });
    await waitForAnimations(page);
    await page.screenshot({
      path: `${OUT_DIR}/02-sticker-in-timeline.png`,
      clip: await clipAround(page, [row], 24),
    });
  });

  test("settings lists the sticker catalog with install state", async ({
    page,
  }) => {
    // NIP-43 membership must be advertised for the client to resolve the
    // viewer's role at all; without it `canCurate` is false and the curator
    // approval queue never renders.
    await installMockBridge(page, { relayRequiresMembership: true });
    await routeStickerAssets(page);
    await page.goto("/");
    await page.getByTestId("open-settings").click();
    await page.getByTestId("profile-popover-settings").click();
    await expect(page.getByTestId("settings-view")).toBeVisible();
    await page.getByTestId("settings-nav-stickers").click();

    const card = page.getByTestId("settings-stickers");
    await expect(card).toContainText("Buzz Classics");
    await expect(card).toContainText("Release Day");
    // Owner role → the pending pack shows in the curator approval queue.
    await expect(card).toContainText("Awaiting catalog approval");
    await expect(card).toContainText("Night Owls");
    await expectImageLoaded(card.locator("img").first());

    await waitForAnimations(page);
    await page.screenshot({
      path: `${OUT_DIR}/03-settings-stickers-catalog.png`,
      clip: await clipAround(page, [card], 8),
    });
  });

  test("picker offers catalog packs when none are installed", async ({
    page,
  }) => {
    await installMockBridge(page, { stickerPacksInstalled: false });
    await routeStickerAssets(page);
    await openChannel(page, "engineering");

    await page.getByTestId("composer-sticker-button").click();
    await expect(page.getByText("No stickers installed")).toBeVisible();
    const installButtons = page.getByRole("button", { name: "Install" });
    await expect(installButtons).toHaveCount(2);

    await waitForAnimations(page);
    await page.screenshot({
      path: `${OUT_DIR}/04-picker-install-state.png`,
      clip: await clipAround(page, [
        stickerPopover(page),
        page.getByTestId("composer-sticker-button"),
      ]),
    });

    // Installing round-trips through the mock relay's kind:10031 store, so the
    // picker flips from the install list to a populated grid.
    await installButtons.first().click();
    await expect(
      page.getByTestId("composer-sticker-pack-select"),
    ).toBeVisible();
    await expect(
      stickerPopover(page).getByRole("button", { name: /^Send / }),
    ).toHaveCount(10);
  });
});
