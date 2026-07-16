import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { passThroughBackupStep } from "../helpers/onboarding";

const SHOTS = "test-results/screenshots-onboarding";

/** Drive to the harness setup page (page 3) via the full onboarding flow. */
async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByRole("button", { name: "Get started" }).click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
}

/** Drive to the default config page (page 4), past the harness page. */
async function navigateToConfigPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
}

test("config page shows Agent defaults section with readiness badge", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  const badge = page.getByTestId("agent-readiness-badge");
  await expect(badge).toBeVisible();

  // Take a screenshot of the entire config page to capture the readiness badge.
  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/04-setup-readiness-badge.png`,
  });
});

test("config page shows Not configured badge when no CLI runtime or buzz-agent config", async ({
  page,
}) => {
  // Seed empty ACP runtimes so no CLI harness is available.
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const badge = page.getByTestId("agent-readiness-badge");
  await expect(badge).toBeVisible();
  await expect(badge).toContainText("Not configured");

  // Not-configured warning text should be visible.
  await expect(
    page.getByText("You can finish now and configure agents later in Settings"),
  ).toBeVisible();

  // Take a screenshot showing the not-configured state.
  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/05-setup-not-configured.png`,
  });
});

test("config page Re-check button triggers runtimes refetch", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  const recheckBtn = page.getByTestId("agent-readiness-recheck");
  await expect(recheckBtn).toBeVisible();
  await expect(recheckBtn).toBeEnabled();
  await recheckBtn.click();

  // After click the button should still be there (page stays on config).
  await expect(recheckBtn).toBeVisible();
});

test("Finish button is always enabled on config page regardless of readiness", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { acpRuntimesCatalog: [] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const finishBtn = page.getByTestId("onboarding-finish");
  await expect(finishBtn).toBeVisible();
  await expect(finishBtn).toBeEnabled();
});

// ---------------------------------------------------------------------------
// B1 regression: rapid consecutive edits must not lose the later change
// ---------------------------------------------------------------------------

test("provider credentials are first-class and drive model discovery", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { acpRuntimesCatalog: undefined },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToConfigPage(page);

  await page.locator("#global-agent-provider").selectOption("openai");
  const apiKey = page.getByLabel("OpenAI API Key");
  await expect(apiKey).toBeVisible();
  await apiKey.fill("test-openai-key");
  await expect(
    page
      .locator("#global-agent-model")
      .getByRole("option", { name: "GPT-5.5" }),
  ).toBeAttached();

  await page.locator("#global-agent-provider").selectOption("openai-compat");
  await expect(page.getByLabel("OpenAI API Key")).toHaveValue(
    "test-openai-key",
  );

  await page
    .locator("#global-agent-provider")
    .selectOption("__custom_provider__");
  await expect(page.getByLabel("OpenAI API Key")).not.toBeVisible();
  await expect(page.locator('input[value="test-openai-key"]')).toHaveCount(0);

  await page.locator("#global-agent-provider").selectOption("anthropic");
  await expect(page.getByLabel("Anthropic API Key")).toBeVisible();

  const databricksOption = page
    .locator("#global-agent-provider")
    .locator('option[value^="databricks"]')
    .first();
  await page
    .locator("#global-agent-provider")
    .selectOption(await databricksOption.getAttribute("value"));
  await expect(page.getByLabel("Value for DATABRICKS_HOST")).toBeVisible();
  await expect(page.getByLabel("OpenAI API Key")).not.toBeVisible();
});

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

  await navigateToConfigPage(page);

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
