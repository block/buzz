import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import {
  seedActiveIdentity,
  passThroughBackupStep,
} from "../helpers/onboarding";

const FIRST_RUN_ALICE = {
  ...TEST_IDENTITIES.alice,
  username: "",
};

/** Drive the fresh-key flow from landing through to the agent step. */
async function navigateToAgentStep(page: Page) {
  await page.getByTestId("onboarding-landing-get-started").click();
  await page.getByTestId("onboarding-display-name").fill("Alice");
  await page.getByTestId("onboarding-next").click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/alice.png");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-agent-goose")).toBeVisible();
}

test("onboarding agent selection — default", async ({ page }) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToAgentStep(page);
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/03a-agent.png",
  });
});

test("onboarding agent selection — selected", async ({ page }) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToAgentStep(page);
  await page.getByTestId("onboarding-agent-goose").click();
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/03d-agent-selected.png",
  });
});
