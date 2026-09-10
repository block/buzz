import { expect, type Page, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import {
  expectEmojiMartStylesInstalled,
  expectSmoothCorners,
} from "../helpers/css";
import { installFakeCamera } from "../helpers/fakeCamera";
import { seedActiveIdentity } from "../helpers/onboarding";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

const SHOTS = "test-results/screenshots-onboarding";

async function selectFirstEmojiFromPicker(page: Page) {
  const picker = page.locator("em-emoji-picker");
  await expect(picker).toBeVisible();
  await expect
    .poll(() =>
      picker.evaluate((element) =>
        Boolean(element.shadowRoot?.querySelector(".scroll button")),
      ),
    )
    .toBe(true);
  await picker.evaluate((element) => {
    const button = element.shadowRoot?.querySelector(".scroll button");
    if (!(button instanceof HTMLElement)) {
      throw new Error("Emoji picker did not render an emoji button.");
    }
    button.click();
  });
}

test("avatar step always shows Skip for now button without an error", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  // Skip button must be visible before any avatar is chosen (no error path).
  const skipBtn = page.getByTestId("onboarding-skip");
  await expect(skipBtn).toBeVisible();
  await expect(skipBtn).toBeEnabled();
  await expect(skipBtn).toHaveText("Skip for now");
  const nextBtn = page.getByTestId("onboarding-next");
  const [skipRadius, nextRadius] = await Promise.all([
    skipBtn.evaluate(
      (element) => window.getComputedStyle(element).borderRadius,
    ),
    nextBtn.evaluate(
      (element) => window.getComputedStyle(element).borderRadius,
    ),
  ]);
  expect(skipRadius).toBe(nextRadius);

  // Capture the whole viewport: the Skip/Next/Back CTAs are portaled into the
  // docked footer (a sibling of the step subtree), so a section-scoped shot
  // would omit the very buttons this artifact is meant to show.
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/01-avatar-skip-button.png`,
  });
});

test("avatar step uses the compact prototype emoji picker", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await page.getByTestId("onboarding-avatar-mode-emoji").click();

  const picker = page.locator("em-emoji-picker");
  await expect(picker.locator("input[type='search']")).toBeVisible();
  await expectEmojiMartStylesInstalled(picker);
  await expect(page.getByTestId("onboarding-avatar-emoji-picker")).toHaveCSS(
    "height",
    "420px",
  );
  const segmentControl = page.getByTestId("onboarding-avatar-mode-control");
  await expect(segmentControl).toHaveAttribute(
    "data-slot",
    "segmented-control",
  );
  const surfaceColors = await Promise.all([
    segmentControl.evaluate(
      (element) => window.getComputedStyle(element).backgroundColor,
    ),
    page
      .getByTestId("onboarding-avatar-emoji-picker")
      .evaluate((element) => window.getComputedStyle(element).backgroundColor),
  ]);
  expect(surfaceColors[0]).toBe(surfaceColors[1]);
  const activeSegmentColor = await page
    .getByTestId("onboarding-avatar-mode-indicator")
    .evaluate((element) => window.getComputedStyle(element).backgroundColor);
  const cardColor = await page
    .getByTestId("onboarding-content-card")
    .evaluate((element) => window.getComputedStyle(element).backgroundColor);
  expect(activeSegmentColor).toBe(cardColor);
  await expectSmoothCorners(page.getByTestId("onboarding-avatar-emoji-picker"));
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/04-avatar-compact-emoji.png`,
  });

  const pickerGeometry = await picker.evaluate((element) => {
    const input = element.shadowRoot?.querySelector<HTMLInputElement>(
      'input[type="search"]',
    );
    const toneControl =
      element.shadowRoot?.querySelector<HTMLElement>(".search + .flex");
    const firstEmojiButton =
      element.shadowRoot?.querySelector<HTMLElement>("[aria-posinset]");
    const firstEmojiRow =
      element.shadowRoot?.querySelector<HTMLElement>(".row");
    const categoryLabel =
      element.shadowRoot?.querySelector<HTMLElement>(".category .sticky");
    const searchHeader = element.shadowRoot?.querySelector<HTMLElement>(
      "#root > .padding-lr",
    );
    const scroll = element.shadowRoot?.querySelector<HTMLElement>(".scroll");
    const toneButton =
      element.shadowRoot?.querySelector<HTMLElement>(".skin-tone-button");
    const nav = element.shadowRoot?.querySelector<HTMLElement>("#nav");
    if (
      !input ||
      !toneControl ||
      !firstEmojiButton ||
      !firstEmojiRow ||
      !categoryLabel ||
      !searchHeader ||
      !scroll ||
      !toneButton
    ) {
      throw new Error("Onboarding emoji picker controls did not render.");
    }
    return {
      categoryLabelBackground:
        window.getComputedStyle(categoryLabel).backgroundColor,
      categoryLabelDisplay: window.getComputedStyle(categoryLabel).display,
      categoryLabelPosition: window.getComputedStyle(categoryLabel).position,
      categoryLabelZIndex: window.getComputedStyle(categoryLabel).zIndex,
      inputRadius: window.getComputedStyle(input).borderRadius,
      input: input.getBoundingClientRect().height,
      firstEmojiButton: firstEmojiButton.getBoundingClientRect().height,
      firstEmojiRowItems: firstEmojiRow.children.length,
      navDisplay: nav ? window.getComputedStyle(nav).display : "absent",
      searchHeaderBottomPadding:
        window.getComputedStyle(searchHeader).paddingBottom,
      searchHeaderSidePadding:
        window.getComputedStyle(searchHeader).paddingLeft,
      scrollTopPadding: window.getComputedStyle(scroll).paddingTop,
      tone: toneControl.getBoundingClientRect().height,
      toneButtonRadius: window.getComputedStyle(toneButton).borderRadius,
      toneControlRadius: window.getComputedStyle(toneControl).borderRadius,
    };
  });
  expect(pickerGeometry).toEqual({
    categoryLabelBackground: "rgb(245, 245, 245)",
    categoryLabelDisplay: "block",
    categoryLabelPosition: "sticky",
    categoryLabelZIndex: "5",
    firstEmojiButton: 72,
    firstEmojiRowItems: 6,
    input: 40,
    inputRadius: "8px",
    navDisplay: "absent",
    searchHeaderBottomPadding: "8px",
    searchHeaderSidePadding: "8px",
    scrollTopPadding: "0px",
    tone: 40,
    toneButtonRadius: "4px",
    toneControlRadius: "8px",
  });

  await picker.evaluate((element) => {
    const toneButton =
      element.shadowRoot?.querySelector<HTMLButtonElement>(".skin-tone-button");
    if (!toneButton) throw new Error("Skin tone button did not render.");
    toneButton.click();
  });
  await expect
    .poll(() =>
      picker.evaluate((element) => {
        const menu = element.shadowRoot?.querySelector<HTMLElement>(".menu");
        if (!menu) return null;
        const style = window.getComputedStyle(menu);
        const rect = menu.getBoundingClientRect();
        const pickerRect = element.getBoundingClientRect();
        return {
          bottomInsidePicker: rect.bottom <= pickerRect.bottom,
          opacity: style.opacity,
          topInsidePicker: rect.top >= pickerRect.top,
          zIndex: style.zIndex,
        };
      }),
    )
    .toEqual({
      bottomInsidePicker: true,
      opacity: "1",
      topInsidePicker: true,
      zIndex: "7",
    });
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/05-avatar-skin-tone-menu.png`,
  });

  await picker.evaluate((element) => {
    const selectedTone =
      element.shadowRoot?.querySelector<HTMLButtonElement>(".menu .option");
    const scroll = element.shadowRoot?.querySelector<HTMLElement>(".scroll");
    if (!selectedTone || !scroll) {
      throw new Error("Emoji picker scroll state did not render.");
    }
    selectedTone.click();
    scroll.scrollTop = 48;
    scroll.dispatchEvent(new Event("scroll"));
  });
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOTS}/06-avatar-sticky-category.png`,
  });
});

test("avatar step keeps a stable card and compact horizontal navigation", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installFakeCamera(page);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  const card = page.getByTestId("onboarding-content-card");
  await expect(card).toBeVisible();
  const profileCardWidth = await card.evaluate(
    (element) => element.clientWidth,
  );
  expect(profileCardWidth).toBe(610);

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  const modeShell = page.getByTestId("onboarding-avatar-mode-content-shell");
  const cardWidths = [await card.evaluate((element) => element.clientWidth)];
  expect(cardWidths[0]).toBeGreaterThan(profileCardWidth);
  const skipBox = await page.getByTestId("onboarding-skip").boundingBox();
  const nextBox = await page.getByTestId("onboarding-next").boundingBox();
  if (!skipBox || !nextBox) throw new Error("Avatar navigation is missing.");
  expect(skipBox.y).toBeCloseTo(nextBox.y, 0);
  expect(skipBox.x + skipBox.width).toBeLessThanOrEqual(nextBox.x);
  const imageShellBox = await modeShell.boundingBox();
  const uploadBox = await page
    .getByTestId("onboarding-avatar-upload")
    .boundingBox();
  const urlBox = await page
    .getByTestId("onboarding-avatar-url")
    .locator("..")
    .boundingBox();
  if (!imageShellBox || !uploadBox || !urlBox) {
    throw new Error("Image controls are missing.");
  }
  expect(imageShellBox.height).toBeCloseTo(420, 0);
  expect(uploadBox.height + 12 + urlBox.height).toBeCloseTo(
    imageShellBox.height,
    0,
  );
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-avatar-actions.png` });

  await page.getByTestId("onboarding-avatar-mode-emoji").click();
  cardWidths.push(await card.evaluate((element) => element.clientWidth));

  await page.getByTestId("onboarding-avatar-mode-animated").click();
  cardWidths.push(await card.evaluate((element) => element.clientWidth));
  expect(new Set(cardWidths).size).toBe(1);

  const iphoneBox = await page
    .getByTestId("onboarding-avatar-animated-camera-iphone")
    .boundingBox();
  const computerBox = await page
    .getByTestId("onboarding-avatar-animated-camera-computer")
    .boundingBox();
  if (!iphoneBox || !computerBox) {
    throw new Error("Animated camera options are missing.");
  }
  expect(iphoneBox.x).toBeCloseTo(computerBox.x, 0);
  expect(iphoneBox.width).toBeCloseTo(computerBox.width, 0);
  expect(iphoneBox.y + iphoneBox.height).toBeLessThan(computerBox.y);
  expect(iphoneBox.height).toBeCloseTo(computerBox.height, 0);
  expect(iphoneBox.height + 12 + computerBox.height).toBeCloseTo(420, 0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/02-avatar-animated.png` });

  await page.getByTestId("onboarding-avatar-animated-camera-computer").click();
  const recordButton = page.getByTestId("onboarding-avatar-animated-record");
  await expect(recordButton).toBeVisible({ timeout: 10_000 });
  await waitForAnimations(page);
  const liveShellBox = await modeShell.boundingBox();
  const liveIphoneBox = await page
    .getByTestId("onboarding-avatar-animated-camera-iphone")
    .boundingBox();
  const liveComputerBox = await page
    .getByTestId("onboarding-avatar-animated-camera-computer")
    .boundingBox();
  const recordBox = await recordButton.boundingBox();
  if (!liveShellBox || !liveIphoneBox || !liveComputerBox || !recordBox) {
    throw new Error("Live animated-avatar controls are missing.");
  }
  expect(liveShellBox.height).toBeCloseTo(420, 0);
  expect(liveIphoneBox.height).toBeCloseTo(liveComputerBox.height, 0);
  expect(liveIphoneBox.height).toBeLessThan(iphoneBox.height);
  expect(recordBox.y + recordBox.height).toBeLessThanOrEqual(
    liveShellBox.y + liveShellBox.height,
  );
  await page.screenshot({ path: `${SHOTS}/02b-avatar-animated-live.png` });

  await page.getByTestId("onboarding-avatar-mode-emoji").click();
  await selectFirstEmojiFromPicker(page);
  await page.getByTestId("onboarding-avatar-custom-color").click();
  const spectrum = page.getByTestId("onboarding-avatar-custom-color-spectrum");
  await expect(spectrum).toBeVisible();
  await expect(page.getByTestId("onboarding-next")).toHaveCount(0);
  const spectrumBox = await spectrum.boundingBox();
  if (!spectrumBox) throw new Error("Custom color spectrum is missing.");
  expect(spectrumBox.height).toBeGreaterThan(300);

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/03-avatar-compact-color.png` });
});

test("avatar step skip button completes community profile setup", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-skip").click();

  await expect(page.getByTestId("onboarding-gate")).not.toBeVisible();
});

test("avatar Next button still requires an avatar to be chosen", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();

  // Next is disabled until an avatar is set.
  await expect(page.getByTestId("onboarding-next")).toBeDisabled();

  // Once an avatar URL is provided, Next enables.
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/avatar.png");
  await expect(page.getByTestId("onboarding-next")).toBeEnabled();
});

// ---------------------------------------------------------------------------
// B4: Routing tests
// ---------------------------------------------------------------------------

test("normal profile setup keeps the existing identity", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expect(page.getByTestId("onboarding-import-key")).toHaveCount(0);
  await expect(page.getByText("Create an identity key")).toHaveCount(0);

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
});

test("Back from the community avatar step returns to profile", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await page.getByTestId("onboarding-display-name").fill("Morty QA");
  await page.getByTestId("onboarding-next").click();

  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page.getByTestId("onboarding-back").click();

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
});
