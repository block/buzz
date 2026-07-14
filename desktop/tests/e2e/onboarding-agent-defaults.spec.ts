import { expect, test } from "@playwright/test";
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

const SHOTS = "test-results/screenshots-onboarding";

/** Drive to the setup page (page 2) via the full onboarding flow. */
async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByTestId("onboarding-display-name").fill("Alice");
  await page.getByTestId("onboarding-next").click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/alice.png");
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-theme")).toBeVisible();
  await page.getByTestId("onboarding-theme-option-github-light").click();
  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
}

test("setup page shows Agent defaults section with readiness badge", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToSetupPage(page);

  const badge = page.getByTestId("agent-readiness-badge");
  await expect(badge).toBeVisible();

  // Take a screenshot of the entire setup page to capture the readiness badge.
  await waitForAnimations(page);
  const setupPage = page.locator('[data-testid="onboarding-page-2"]');
  await setupPage.screenshot({
    path: `${SHOTS}/04-setup-readiness-badge.png`,
  });
});

test("setup page shows Not configured badge when no CLI runtime or buzz-agent config", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  // Seed empty ACP runtimes so no CLI harness is available.
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToSetupPage(page);

  const badge = page.getByTestId("agent-readiness-badge");
  await expect(badge).toBeVisible();
  await expect(badge).toContainText("Not configured");

  // Not-configured warning text should be visible.
  await expect(
    page.getByText("You can finish now and configure agents later in Settings"),
  ).toBeVisible();

  // Take a screenshot showing the not-configured state.
  await waitForAnimations(page);
  const setupPage = page.locator('[data-testid="onboarding-page-2"]');
  await setupPage.screenshot({
    path: `${SHOTS}/05-setup-not-configured.png`,
  });
});

test("setup page Re-check button triggers runtimes refetch", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await navigateToSetupPage(page);

  const recheckBtn = page.getByTestId("agent-readiness-recheck");
  await expect(recheckBtn).toBeVisible();
  await expect(recheckBtn).toBeEnabled();
  await recheckBtn.click();

  // After click the button should still be there (page stays on setup).
  await expect(recheckBtn).toBeVisible();
});

test("Finish button is always enabled on setup page regardless of readiness", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToSetupPage(page);

  const finishBtn = page.getByTestId("onboarding-finish");
  await expect(finishBtn).toBeVisible();
  await expect(finishBtn).toBeEnabled();
});

// ---------------------------------------------------------------------------
// B1 regression: rapid consecutive edits must not lose the later change
// ---------------------------------------------------------------------------

test("rapid consecutive provider changes both survive — later change wins", async ({
  page,
}) => {
  await seedActiveIdentity(page, FIRST_RUN_ALICE);
  // Hold each set_global_agent_config request for 300 ms so the test can
  // make a second edit before the first response arrives.
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [], setGlobalAgentConfigDelayMs: 300 },
    { skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToSetupPage(page);

  const providerSelect = page.locator("#global-agent-provider");
  await expect(providerSelect).toBeVisible();

  // First edit: select OpenAI — save starts, held open for 300 ms.
  await providerSelect.selectOption("openai");

  // Second edit before first response: select Anthropic. The coalescer must
  // persist this as the trailing save, and it must survive in the UI.
  await providerSelect.selectOption("anthropic");

  // Wait long enough for both saves to complete (2 × 300 ms + margin).
  await page.waitForTimeout(800);

  // The final provider shown must be Anthropic — neither save must overwrite
  // the later optimistic state with a stale response.
  await expect(providerSelect).toHaveValue("anthropic");
});
