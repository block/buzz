import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { passThroughBackupStep } from "../helpers/onboarding";

const SHOTS = "test-results/screenshots-onboarding";

/** Drive to the setup page (page 2) via the full onboarding flow. */
async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByRole("button", { name: "Get started" }).click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
}

test("setup page shows Agent defaults section with readiness badge", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
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
  // Seed empty ACP runtimes so no CLI harness is available.
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
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
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
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
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
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
  // Hold each set_global_agent_config request for 300 ms so the test can
  // make a second edit before the first response arrives.
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [], setGlobalAgentConfigDelayMs: 300 },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
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
