import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/global-agent-config";

// Settle any in-flight CSS / Web Animations before capture.
async function settleAnimations(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );
}

/**
 * Navigate to the Agents view (where GlobalAgentConfigSettingsCard lives) and
 * wait for the card to finish loading.
 */
async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  // Wait for the global agent config card to mount and finish its load effect.
  await expect(page.getByTestId("settings-global-agent-config")).toBeVisible({
    timeout: 10_000,
  });
  // The card shows a spinner while loading; wait for it to disappear.
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

/**
 * Navigate to the Agents view and open the Create Agent dialog via the
 * "Custom agent" option, then fill a placeholder name.
 */
async function openCreateDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByText("Custom agent").click();
  await page.getByTestId("agent-name-input").fill("Test Agent");
}

test.describe("global agent config screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
  });

  // Shot 01: GlobalAgentConfigSettingsCard populated with provider + model +
  // env var — shows the "Agent defaults" card in the Agents view as it looks
  // when a user has set global defaults.
  test("01-global-agent-config-card-populated", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-placeholder" },
      },
    });

    await openAgentsView(page);

    const card = page.getByTestId("settings-global-agent-config");
    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/01-global-agent-config-card-populated.png`,
    });
  });

  // Shot 02: Create Agent with global provider = anthropic, no per-agent
  // provider selected, Advanced section auto-expanded, ANTHROPIC_API_KEY
  // shown as a required amber row (Test 2.1 + 2.2 fix).
  test("02-create-global-provider-required-key-advanced-open", async ({
    page,
  }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: null,
        env_vars: {},
      },
    });

    await openCreateDialog(page);

    // With global provider = anthropic and no per-agent provider set, the gate
    // derives the effective provider as anthropic → ANTHROPIC_API_KEY required.
    // The Advanced section auto-expands when required env keys appear.
    await expect(page.getByTestId("env-vars-required-key")).toHaveText(
      "ANTHROPIC_API_KEY",
      { timeout: 10_000 },
    );

    // Scroll the required row into view.
    await page.getByTestId("env-vars-required-key").scrollIntoViewIfNeeded();
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/02-create-global-provider-required-key-advanced-open.png`,
    });
  });

  // Shot 03: Global env satisfies ANTHROPIC_API_KEY — no required amber row,
  // Advanced stays collapsed, and the Create button is enabled (Test 4 nuance fix:
  // globally-satisfied keys are excluded from requiredKeys entirely).
  test("03-global-env-satisfies-required-key", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: null,
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-global-value" },
      },
    });

    await openCreateDialog(page);

    // Global env_vars satisfies ANTHROPIC_API_KEY, so computeLocalModeGate
    // excludes it from requiredEnvKeys — no locked amber row rendered.
    await expect(page.locator("#agent-provider")).toBeVisible({
      timeout: 10_000,
    });
    // No required rows present — globally satisfied keys have no amber row.
    await expect(page.getByTestId("env-vars-required-key")).not.toBeVisible({
      timeout: 5_000,
    });
    // Submit is enabled: effectiveProvider = global "anthropic" is valid.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 5_000,
    });

    // Scroll down to show the env section — empty of amber required rows because
    // the globally-satisfied key is excluded from requiredEnvKeys entirely.
    const dialog = page.getByRole("dialog");
    const envEditor = dialog.getByTestId("env-vars-editor");
    await envEditor.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await dialog.screenshot({
      path: `${SHOTS}/03-global-env-satisfies-required-key.png`,
    });
  });

  // Shot 04: Create gate BLOCKED — no per-agent provider, no global provider
  // set → submit button disabled (provider-default rule, Test 5 / shots 01/08
  // from agent-readiness-screenshots.spec.ts).
  test("04-create-blocked-no-provider-no-global", async ({ page }) => {
    // Default mock bridge has no global provider.
    await installMockBridge(page);

    await openCreateDialog(page);

    // Provider empty + no global provider → submit BLOCKED.
    await expect(page.getByTestId("create-agent-submit")).toBeDisabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/04-create-blocked-no-provider-no-global.png`,
    });
  });

  // Shot 05: Create gate ENABLED — global provider = anthropic provides a
  // default, so the empty per-agent provider is resolved → submit enabled.
  test("05-create-enabled-with-global-provider", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: null,
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-global-value" },
      },
    });

    await openCreateDialog(page);

    // Global provider satisfies the provider-default rule → submit enabled.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/05-create-enabled-with-global-provider.png`,
    });
  });
});
