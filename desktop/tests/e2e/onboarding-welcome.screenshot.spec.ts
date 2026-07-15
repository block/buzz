import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);

async function seedOnboardingCompletion(
  page: Parameters<typeof installMockBridge>[0],
  pubkey: string,
) {
  await page.addInitScript(
    ({ storageKey }: { storageKey: string }) => {
      window.localStorage.setItem(storageKey, "true");
    },
    { storageKey: `buzz-onboarding-complete.v1:${pubkey}` },
  );
}

/**
 * First-run welcome empty state (step 7a): a freshly-onboarded user who has
 * not yet dismissed the welcome lands on Home and sees the welcome composition.
 */
test("home first-run welcome empty state", async ({ page }) => {
  await seedOnboardingCompletion(page, DEFAULT_MOCK_PUBKEY);
  await installMockBridge(page, undefined);
  await page.goto("/");

  await expect(page.getByTestId("home-welcome-empty-state")).toBeVisible();
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/07a-welcome.png",
  });
});
