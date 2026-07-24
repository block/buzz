import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

test.describe("home inbox chrome", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  });

  test("puts the filter on the left and options on the right", async ({
    page,
  }) => {
    const filter = page.getByTestId("inbox-filter-trigger");
    const options = page.getByTestId("inbox-options-trigger");

    await expect(filter).toBeVisible();
    await expect(filter).toContainText("All");
    await expect(options).toBeVisible();

    const [filterBox, optionsBox] = await Promise.all([
      filter.boundingBox(),
      options.boundingBox(),
    ]);
    expect(filterBox).not.toBeNull();
    expect(optionsBox).not.toBeNull();
    expect(filterBox?.x ?? 0).toBeLessThan(optionsBox?.x ?? 0);
  });

  test("shares a blurred header backdrop across list and detail", async ({
    page,
  }) => {
    await expect(page.getByTestId("home-inbox-detail")).toBeVisible();

    const homeInbox = page.getByTestId("home-inbox");
    const listScroller = page.getByTestId("home-inbox-list");
    const detailScroller = page.getByTestId("home-inbox-detail-scroll");
    const sharedBackdrop = page.getByTestId(
      "home-inbox-shared-header-backdrop",
    );

    const [homeBox, backdropBox, listBox, detailBox, backdropFilter] =
      await Promise.all([
        homeInbox.boundingBox(),
        sharedBackdrop.boundingBox(),
        listScroller.boundingBox(),
        detailScroller.boundingBox(),
        sharedBackdrop.evaluate(
          (element) => getComputedStyle(element).backdropFilter,
        ),
      ]);

    expect(homeBox).not.toBeNull();
    expect(backdropBox).not.toBeNull();
    expect(listBox).not.toBeNull();
    expect(detailBox).not.toBeNull();
    expect(Math.round(backdropBox?.x ?? 0)).toBe(Math.round(homeBox?.x ?? 0));
    expect(Math.round(backdropBox?.width ?? 0)).toBe(
      Math.round(homeBox?.width ?? 0),
    );
    expect(Math.round(listBox?.y ?? 0)).toBe(Math.round(backdropBox?.y ?? 0));
    expect(Math.round(detailBox?.y ?? 0)).toBe(Math.round(backdropBox?.y ?? 0));
    expect(backdropFilter).not.toBe("none");

    await waitForAnimations(page);
  });

  test("reserves the measured composer height and masks content behind it", async ({
    page,
  }) => {
    const detailScroller = page.getByTestId("home-inbox-detail-scroll");
    const composerOverlay = page.getByTestId(
      "home-inbox-detail-composer-overlay",
    );

    await expect(composerOverlay).toBeVisible();
    await expect
      .poll(async () => {
        const [paddingBottom, overlayBox] = await Promise.all([
          detailScroller.evaluate((element) =>
            Number.parseFloat(getComputedStyle(element).paddingBottom),
          ),
          composerOverlay.boundingBox(),
        ]);
        return Math.abs(paddingBottom - (overlayBox?.height ?? 0));
      })
      .toBeLessThanOrEqual(1);

    const overlayMaskStyles = await composerOverlay.evaluate((element) => ({
      gradient: getComputedStyle(element, "::before").backgroundImage,
      solidBackground: getComputedStyle(element, "::after").backgroundColor,
    }));
    expect(overlayMaskStyles.gradient).not.toBe("none");
    expect(overlayMaskStyles.solidBackground).not.toBe("rgba(0, 0, 0, 0)");
  });
});
