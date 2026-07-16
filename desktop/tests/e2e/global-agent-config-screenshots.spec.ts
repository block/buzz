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
 * Open Settings → Agents through the app UI and wait for the defaults card to
 * load. CI serves the built SPA with a static file server, so navigating to
 * `/settings` directly returns a 404 before the client router can start.
 */
async function openAiDefaultsSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-agents").click();
  await expect(page.getByTestId("settings-global-agent-config")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

/**
 * Navigate to the Agents view and open the Create Agent dialog via the
 * "New agent" menu item, then fill a placeholder name.
 */
async function openCreateDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill("Test Agent");
}

async function customizeAgentAi(page: import("@playwright/test").Page) {
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
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

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");
    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/01-global-agent-config-card-populated.png`,
    });
  });

  test("02-create-global-provider-shows-top-level-api-key", async ({
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
    await customizeAgentAi(page);

    await expect(page.getByLabel("Anthropic API Key")).toBeVisible({
      timeout: 10_000,
    });
    await expect(
      page.getByRole("button", { name: "Advanced", exact: true }),
    ).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByTestId("env-vars-required-key")).not.toBeVisible();
  });

  test("03-global-env-satisfies-required-key", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-global-value" },
      },
    });

    await openCreateDialog(page);
    await customizeAgentAi(page);

    await expect(page.getByLabel("Anthropic API Key")).toHaveAttribute(
      "placeholder",
      "Inherited from global config",
    );
    await expect(page.getByTestId("env-vars-required-key")).not.toBeVisible({
      timeout: 5_000,
    });
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 5_000,
    });
  });

  test("06-baked-defaults-labels-appear-in-create-dialog", async ({ page }) => {
    await installMockBridge(page, {
      bakedBuildEnv: [
        {
          key: "BUZZ_AGENT_PROVIDER",
          value: "anthropic",
          masked: false,
        },
        {
          key: "BUZZ_AGENT_MODEL",
          value: "claude-opus-4-8",
          masked: false,
        },
        {
          key: "BUZZ_AGENT_THINKING_EFFORT",
          value: "high",
          masked: false,
        },
        {
          key: "ANTHROPIC_API_KEY",
          value: "sk-ant-baked-test",
          masked: true,
        },
      ],
    });

    await openCreateDialog(page);

    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-8", { exact: true }),
    ).toBeVisible();
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    await expect(page.locator("#persona-model")).not.toBeVisible();
  });

  test("07-explicit-global-defaults-override-baked-labels", async ({
    page,
  }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { BUZZ_AGENT_THINKING_EFFORT: "low" },
      },
      bakedBuildEnv: [
        {
          key: "BUZZ_AGENT_PROVIDER",
          value: "databricks_v2",
          masked: false,
        },
        { key: "BUZZ_AGENT_MODEL", value: "build-model", masked: false },
        {
          key: "BUZZ_AGENT_THINKING_EFFORT",
          value: "high",
          masked: false,
        },
      ],
    });

    await openCreateDialog(page);

    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-5", { exact: true }),
    ).toBeVisible();
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    await expect(page.locator("#persona-model")).not.toBeVisible();
  });

  // Shot 04: Create gate BLOCKED — no per-agent provider, no global provider
  // set → submit button disabled (provider-default rule, Test 5 / shots 01/08
  // from agent-readiness-screenshots.spec.ts).
  test("04-create-blocked-no-provider-no-global", async ({ page }) => {
    // Default mock bridge has no global provider.
    await installMockBridge(page);

    await openCreateDialog(page);

    // Provider empty + no global provider → submit BLOCKED.
    await expect(page.getByTestId("persona-dialog-submit")).toBeDisabled({
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
        model: "claude-opus-4-5",
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-global-value" },
      },
    });

    await openCreateDialog(page);

    // Global provider satisfies the provider-default rule → submit enabled.
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/05-create-enabled-with-global-provider.png`,
    });
  });
});
