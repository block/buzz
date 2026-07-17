import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

const SHOT_DIR = "test-results/onboarding-docked-cta";

test.use({ viewport: { width: 1280, height: 800 } });

test("machine onboarding: landing, backup, setup docked CTAs", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  const gate = page.getByTestId("machine-onboarding-gate");
  await expect(gate).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01-landing.png` });

  await page.getByRole("button", { name: "Enter a key" }).click();
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();
  const importCard = page.getByTestId("nostr-import-card");
  await expect(importCard).toBeVisible();
  // Outer card is transparent/borderless — the SVG texture layer and the
  // white surface span own the visuals.
  await expect(importCard).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(importCard).toHaveCSS("border-top-width", "0px");
  await expect(importCard.locator("svg")).toHaveCount(1);
  // Figma texture recipe: dilated+blurred alpha ramp dithered by per-pixel
  // fractal noise (no displaced hard rectangle).
  await expect(importCard.locator("feTurbulence")).toHaveAttribute(
    "baseFrequency",
    "0.999",
  );
  await expect(importCard.locator("feGaussianBlur")).toHaveCount(1);
  await expect(importCard.locator("feDisplacementMap")).toHaveCount(0);
  // Static texture — no animated noise.
  await expect(importCard.locator("feTurbulence animate")).toHaveCount(0);
  await expect(importCard.locator('[data-card-surface="true"]')).toHaveCount(1);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01b-enter-key.png` });

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByRole("button", { name: "Get started" })).toBeVisible();
  await page.getByRole("button", { name: "Get started" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Your unique identity has been created",
    }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02-backup.png` });

  // Reveal the key: box must not reflow (same-length monospace mask).
  await page.getByTestId("nsec-reveal-toggle").click();
  await expect(page.getByTestId("nsec-value")).toHaveClass(/select-text/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02b-backup-revealed.png` });

  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByRole("heading", { name: "Use the models that fit the task" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03-setup.png` });
});

test("relay onboarding: profile and avatar docked CTAs", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await page.getByTestId("onboarding-display-name").fill("Ada Lovelace");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/04-profile.png` });

  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/onboarding-avatar.png");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/05-avatar.png` });
});
