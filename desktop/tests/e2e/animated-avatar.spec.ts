import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { installFakeCamera } from "../helpers/fakeCamera";
import { openSettings } from "../helpers/settings";

// The review editor (preview + framing + poster strip + backdrop panel) is
// taller than the default 720px viewport — raise it so the whole editor remains visible.
test.use({ viewport: { height: 1280, width: 1280 } });

const UPLOADED_PNG_DESCRIPTOR = {
  sha256: "ab".repeat(32),
  size: 4096,
  type: "image/png",
  uploaded: 1700000000,
  url: `https://relay.example.com/media/${"ab".repeat(32)}.png`,
};

async function openAnimatedTab(
  page: import("@playwright/test").Page,
  options: { cameraDelayMs?: number; holdCamera?: boolean } = {},
) {
  await installFakeCamera(page, options);
  await installMockBridge(page, {
    uploadDescriptors: [UPLOADED_PNG_DESCRIPTOR],
  });
  await page.goto("/");
  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByRole("tab", { name: "Animated" }).click();
  await expect(page.getByTestId("profile-avatar-animated")).toBeVisible();
  // Let the tab indicator + content height animations settle.
  await page.waitForTimeout(400);
}

test.describe("animated avatar", () => {
  test("01 — animated tab idle state", async ({ page }) => {
    await openAnimatedTab(page);
    await expect(
      page.getByTestId("profile-avatar-animated-camera-iphone"),
    ).toBeVisible();
    await expect(
      page.getByTestId("profile-avatar-animated-camera-computer"),
    ).toBeVisible();
    await expect(
      page.getByTestId("profile-avatar-animated-camera-iphone"),
    ).toHaveAttribute("aria-pressed", "false");
    await expect(
      page.getByTestId("profile-avatar-animated-camera-computer"),
    ).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByTestId("profile-avatar-animated-start")).toHaveCount(
      0,
    );
    await expect(
      page.getByTestId("profile-avatar-animated-camera-select"),
    ).toHaveCount(0);
    await expect(page.getByTestId("profile-avatar-done")).toHaveCount(0);
  });

  test("02 — live camera preview", async ({ page }) => {
    await openAnimatedTab(page, { holdCamera: true });
    const animatedTabHeight = () =>
      page
        .getByTestId("profile-avatar-animated")
        .evaluate((element) =>
          Math.round(element.getBoundingClientRect().height),
        );
    const idleHeight = await animatedTabHeight();

    await page.getByTestId("profile-avatar-animated-camera-iphone").click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as Window & {
                __BUZZ_E2E_CAMERA_REQUEST_COUNT__?: number;
              }
            ).__BUZZ_E2E_CAMERA_REQUEST_COUNT__ ?? 0,
        ),
      )
      .toBeGreaterThan(0);
    const previewSlot = page.getByTestId(
      "profile-avatar-animated-preview-slot",
    );
    await expect(
      page.getByRole("status").filter({ hasText: "Starting camera" }),
    ).toBeVisible();
    await expect(
      previewSlot.getByRole("status").filter({ hasText: "Starting camera" }),
    ).toBeVisible();
    await expect(page.getByTestId("profile-avatar-preview")).toHaveCount(0);
    await expect(
      page.getByTestId("profile-avatar-animated-record"),
    ).toHaveCount(0);
    const startingHeight = await animatedTabHeight();
    expect(Math.abs(startingHeight - idleHeight)).toBeLessThanOrEqual(1);

    await page.evaluate(() => {
      (
        window as Window & {
          __BUZZ_E2E_RELEASE_CAMERA__?: () => void;
        }
      ).__BUZZ_E2E_RELEASE_CAMERA__?.();
    });
    await expect(
      page.getByTestId("profile-avatar-animated-record"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      previewSlot.getByTestId("profile-avatar-animated-preview"),
    ).toBeVisible();
    const liveHeight = await animatedTabHeight();
    expect(Math.abs(liveHeight - idleHeight)).toBeLessThanOrEqual(1);
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (
              window as Window & {
                __BUZZ_E2E_CAMERA_CONSTRAINTS__?: MediaStreamConstraints[];
              }
            ).__BUZZ_E2E_CAMERA_CONSTRAINTS__?.at(-1)?.video,
        ),
      )
      .toEqual(
        expect.objectContaining({
          deviceId: { exact: "iphone-continuity" },
          facingMode: { ideal: "user" },
        }),
      );
    await expect(page.getByTestId("profile-avatar-done")).toHaveCount(0);
    await page.waitForTimeout(500);

    await page.getByTestId("profile-avatar-animated-record").click();
    await expect(
      page.getByTestId("profile-avatar-animated-sections"),
    ).toBeVisible({ timeout: 60_000 });
    await expect(
      page.getByTestId("profile-avatar-animated-size"),
    ).toHaveAttribute("aria-valuenow", "116");
  });

  test("03 — recording ring, pop-out review, custom color", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    await openAnimatedTab(page);
    await page.getByTestId("profile-avatar-animated-camera-computer").click();
    await expect(
      page.getByTestId("profile-avatar-animated-record"),
    ).toBeVisible({ timeout: 10_000 });
    await page.getByTestId("profile-avatar-animated-record").click();

    // Mid-recording: the timer stroke sweeps around the capture circle.
    await expect(
      page.getByTestId("profile-avatar-animated-timer-ring"),
    ).toBeVisible();
    await page.waitForTimeout(1500);

    // Review: the section icon row appears once processing finishes.
    await expect(
      page.getByTestId("profile-avatar-animated-sections"),
    ).toBeVisible({ timeout: 60_000 });
    await expect(
      page.getByTestId("profile-avatar-animated-size"),
    ).toHaveAttribute("aria-valuenow", "126");
    await expect(page.getByTestId("profile-avatar-done")).toBeVisible();
    await expect(
      page.getByTestId("profile-avatar-animated-framing-readout"),
    ).toHaveCount(0);

    const previewSnapshot = () =>
      page
        .getByTestId("profile-avatar-animated-review-preview")
        .evaluate((canvas) => (canvas as HTMLCanvasElement).toDataURL());

    // Color section: pick a deterministic backdrop color (the default is random).
    await page.getByTestId("profile-avatar-animated-section-color").click();
    await expect(
      page.getByTestId("profile-avatar-animated-backdrop-none"),
    ).toHaveCount(0);
    await page
      .getByTestId("profile-avatar-animated-backdrop-grid")
      .locator("button")
      .nth(5)
      .click();

    // Poster section: selecting a frame updates the (paused) preview.
    await page.getByTestId("profile-avatar-animated-section-poster").click();
    await expect(
      page.getByTestId("profile-avatar-animated-poster-strip"),
    ).toBeVisible();
    await expect(
      page.getByTestId("profile-avatar-animated-poster-selector"),
    ).toBeVisible();
    await expect(
      page.getByTestId("profile-avatar-animated-review-help"),
    ).toHaveText("Pick the still shown before hover.");
    await expect(
      page.getByTestId("profile-avatar-animated-poster-prev"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("profile-avatar-animated-poster-next"),
    ).toHaveCount(0);
    const scrubber = page.getByTestId(
      "profile-avatar-animated-poster-scrubber",
    );
    const scrubberBox = await scrubber.boundingBox();
    if (!scrubberBox) {
      throw new Error("Animated avatar poster scrubber did not render bounds.");
    }
    await scrubber.click({
      position: {
        x: scrubberBox.width * 0.72,
        y: scrubberBox.height / 2,
      },
    });
    await expect(scrubber).not.toHaveAttribute("aria-valuenow", "0");

    // The Circle tool stays implemented, but it is hidden in the review row for now.
    await expect(
      page.getByTestId("profile-avatar-animated-section-shape"),
    ).toHaveCount(0);

    // You section: the DialKit-style local slider re-composes the preview.
    await page.getByTestId("profile-avatar-animated-section-person").click();
    await expect(
      page.getByTestId("profile-avatar-animated-review-help"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("profile-avatar-animated-outline-toggle"),
    ).toHaveAttribute("aria-pressed", "true");
    await page.getByTestId("profile-avatar-animated-outline-toggle").click();
    await expect(
      page.getByTestId("profile-avatar-animated-outline-toggle"),
    ).toHaveAttribute("aria-pressed", "false");
    await page.getByTestId("profile-avatar-animated-outline-toggle").click();
    const sizeSlider = page.getByTestId("profile-avatar-animated-size");
    const initialSizeValue = await sizeSlider.getAttribute("aria-valuenow");
    const sizeSliderBox = await sizeSlider.boundingBox();
    if (!sizeSliderBox) {
      throw new Error("Animated avatar size slider did not render bounds.");
    }
    await sizeSlider.click({
      position: {
        x: sizeSliderBox.width * 0.25,
        y: sizeSliderBox.height / 2,
      },
    });
    await expect(sizeSlider).not.toHaveAttribute(
      "aria-valuenow",
      initialSizeValue ?? "",
    );
    // Restore defaults so the standard framing remains covered.
    await page.getByTestId("profile-avatar-animated-reset-framing").click();
    await page.waitForTimeout(400);

    // Custom backdrop color panel (shared HSV picker).
    await page.getByTestId("profile-avatar-animated-section-color").click();
    await page.getByTestId("profile-avatar-animated-backdrop-custom").click();
    await expect(
      page.getByTestId("profile-avatar-animated-custom-color-spectrum"),
    ).toBeVisible();
    await expect(page.getByTestId("profile-avatar-done")).toHaveCount(0);
    const beforeCustomColor = await previewSnapshot();
    const customColorSpectrum = page.getByTestId(
      "profile-avatar-animated-custom-color-spectrum",
    );
    const customColorSpectrumBox = await customColorSpectrum.boundingBox();
    if (!customColorSpectrumBox) {
      throw new Error(
        "Animated avatar custom color picker did not render bounds.",
      );
    }
    await customColorSpectrum.click({
      position: {
        x: customColorSpectrumBox.width * 0.88,
        y: customColorSpectrumBox.height * 0.82,
      },
    });
    await page.waitForTimeout(400);
    const customColorPreview = await previewSnapshot();
    expect(customColorPreview).not.toEqual(beforeCustomColor);
    await page.getByTestId("profile-avatar-animated-custom-color-done").click();
    await expect(
      page.getByTestId("profile-avatar-animated-backdrop-custom"),
    ).toHaveAttribute("aria-pressed", "true");
    expect(await previewSnapshot()).toEqual(customColorPreview);

    // Done applies the recording (APNG encode + mocked two-file upload) and
    // then saves the profile in one step.
    const doneButton = page.getByTestId("profile-avatar-done");
    await doneButton.click();
    await expect(doneButton).toContainText("Saving", { timeout: 2_000 });
    await expect(page.getByRole("tab", { name: "Image" })).toBeDisabled();
    await expect(page.getByRole("tab", { name: "Emoji" })).toBeDisabled();
    await expect(page.getByRole("tab", { name: "Animated" })).toBeDisabled();
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (
                window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
              ).__BUZZ_E2E_COMMANDS__?.filter(
                (command) => command === "upload_media_bytes",
              ).length ?? 0,
          ),
        { timeout: 30_000 },
      )
      .toBeGreaterThanOrEqual(2);
    await expect
      .poll(
        () =>
          page.evaluate(
            () =>
              (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
                .__BUZZ_E2E_COMMANDS__ ?? [],
          ),
        { timeout: 30_000 },
      )
      .toEqual(expect.arrayContaining(["update_profile"]));
    await expect(page.getByTestId("profile-avatar-animated-error")).toHaveCount(
      0,
    );
    await expect(page.getByTestId("profile-avatar-preview")).toBeVisible({
      timeout: 10_000,
    });
  });
});
