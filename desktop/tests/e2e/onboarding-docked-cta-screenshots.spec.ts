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

  await page.getByRole("button", { name: "Use an existing key" }).click();
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();
  const importCard = page.getByTestId("nostr-import-card");
  await expect(importCard).toBeVisible();
  await expect(page.getByLabel("Private key", { exact: true })).toBeVisible();
  // The production card uses a baked nine-slice texture: no runtime SVG
  // filter, measurement, or texture regeneration during resize.
  await expect(importCard).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(importCard).toHaveCSS("border-top-width", "0px");
  await expect(importCard).toHaveCSS("border-image-repeat", "repeat");
  await expect(importCard).toHaveCSS("border-image-outset", "96px");
  // Icon SVGs (e.g. the reveal toggle) are fine; a filter would mean the
  // texture regressed to the runtime SVG pipeline.
  await expect(importCard.locator("svg filter")).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01b-enter-key.png` });

  await page.getByRole("button", { name: "Back" }).click();
  await expect(
    page.getByRole("button", { name: "Create a new identity key" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Your unique identity key has been created",
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

test("machine key import remains usable in a short viewport", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 620 });
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Use an existing key" }).click();

  const heading = page.getByRole("heading", { name: "Enter your private key" });
  const input = page.getByLabel("Private key", { exact: true });
  const footer = page.getByTestId("onboarding-footer-slot");
  await expect(heading).toBeVisible();
  await expect(input).toBeVisible();
  await expect(footer).toBeVisible();

  const layout = await page.evaluate(() => {
    const heading = document.querySelector("h1")?.getBoundingClientRect();
    const input = document
      .querySelector<HTMLInputElement>("#nostr-private-key")
      ?.getBoundingClientRect();
    const footer = document
      .querySelector('[data-testid="onboarding-footer-slot"]')
      ?.getBoundingClientRect();
    return {
      footerTop: footer?.top ?? 0,
      headingBottom: heading?.bottom ?? 0,
      inputBottom: input?.bottom ?? 0,
      inputTop: input?.top ?? 0,
      clientWidth: document.documentElement.clientWidth,
      scrollHeight: document.documentElement.scrollHeight,
      scrollWidth: document.documentElement.scrollWidth,
    };
  });
  expect(layout.inputTop).toBeGreaterThan(layout.headingBottom);
  expect(layout.footerTop).toBeGreaterThan(layout.inputBottom);
  expect(layout.scrollHeight).toBeGreaterThanOrEqual(620);
  expect(layout.scrollWidth).toBe(layout.clientWidth);
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
