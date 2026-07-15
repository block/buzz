import { test } from "@playwright/test";
import { expect } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";
import { waitForAnimations } from "../helpers/animations";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

/**
 * Screenshot the onboarding landing (step 1) — the marketing-branded welcome
 * surface. A blank first-run identity with no kind:0 profile event lands on
 * onboarding, which now opens on the landing page.
 */
test("onboarding landing", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(
    page.getByTestId("onboarding-landing-get-started"),
  ).toBeVisible();
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/01-landing.png",
  });
});
