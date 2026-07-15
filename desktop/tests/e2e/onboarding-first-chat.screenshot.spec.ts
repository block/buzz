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

/** Drive the fresh-key flow from landing through to the first-chat step. */
async function navigateToFirstChatStep(page: Page) {
  await page.getByTestId("onboarding-landing-get-started").click();
  await page.getByTestId("onboarding-display-name").fill("Alice");
  await page.getByTestId("onboarding-next").click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/alice.png");
  await page.getByTestId("onboarding-next").click();
  // Agent step
  await expect(page.getByTestId("onboarding-agent-goose")).toBeVisible();
  await page.getByTestId("onboarding-agent-goose").click();
  await page.getByTestId("onboarding-next").click();
  // Theme step
  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await page.getByTestId("onboarding-theme-option-github-light").click();
  await page.getByTestId("onboarding-next").click();
  // Setup (harness) step
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await page.getByTestId("onboarding-finish").click();
  // First-chat step
  await expect(page.getByTestId("onboarding-first-chat-input")).toBeVisible();
}

test("onboarding first chat — empty", async ({ page }) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToFirstChatStep(page);
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/04a-first-chat.png",
  });
});

test("onboarding first chat — reply + continue", async ({ page }) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToFirstChatStep(page);
  await page.getByTestId("onboarding-first-chat-send").click();

  // Wait for the scripted reply to land (Continue appears).
  await expect(page.getByTestId("onboarding-next")).toBeVisible({
    timeout: 5000,
  });
  await waitForAnimations(page);

  await page.screenshot({
    path: "test-results/screenshots/04d-first-chat-reply.png",
  });
});
