import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOT_DIR = "test-results/anonymous-shell";

test.use({ viewport: { width: 1280, height: 960 } });

test("anonymous app-shell home on first open", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  // Fresh install → straight into the full-bleed app shell: ghost rail +
  // sidebar on the left, community discovery in the main pane. No identity
  // ceremony and no agent greeting.
  await expect(page.getByTestId("discovery-landing")).toBeVisible();
  await expect(page.getByTestId("anon-shell-rail")).toBeVisible();
  await expect(page.getByTestId("anon-shell-sidebar")).toBeVisible();
  await expect(page.getByTestId("discovery-join-buzz-hq")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01-anonymous-shell-home.png` });

  // Advanced setup remains one click away (the classic corridor).
  await page.getByTestId("discovery-advanced-setup").click();
  await expect(
    page.getByRole("button", { name: "Create a new identity key" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02-advanced-setup-corridor.png` });

  // Back returns to discovery.
  await page.getByTestId("identity-back-to-discover").click();
  await expect(page.getByTestId("discovery-landing")).toBeVisible();

  // One-click join: identity is created silently, the community is added, and
  // (with the mock relay resolving instantly) the user lands straight in the
  // app — no agent corridor, no backup ceremony.
  await page.getByTestId("discovery-join-buzz-hq").click();
  await expect
    .poll(() =>
      page.evaluate(() => window.localStorage.getItem("buzz-communities")),
    )
    .toContain("Buzz HQ");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03-after-join-click.png` });
});
