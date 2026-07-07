import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-readiness";

// An existing buzz-agent managed agent for the Edit-dialog shot.
// Tyler's pubkey maps to gooseSurface in the mock bridge (runtimeId: "goose"),
// which supports LLM provider selection — the shared AgentProviderField /
// AgentModelField components render for it just as they do for buzz-agent.
const EDIT_AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;

/**
 * Navigate to the agents view and open the Create Agent dialog via the
 * "Custom agent" option, then fill a placeholder name.
 */
async function openCreateDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByText("Custom agent").click();
  await page.getByTestId("agent-name-input").fill("Test Agent");
}

/**
 * Wait for the provider field to become visible (buzz-agent auto-selected)
 * then select the given provider value.
 */
async function selectProvider(
  page: import("@playwright/test").Page,
  provider: string,
) {
  await expect(page.locator("#agent-provider")).toBeVisible({
    timeout: 10_000,
  });
  await page.locator("#agent-provider").selectOption(provider);
}

/**
 * Choose "Custom model..." from the model dropdown and fill a custom model id.
 */
async function setCustomModel(
  page: import("@playwright/test").Page,
  modelId: string,
) {
  await page.locator("#agent-model").selectOption("__custom_model__");
  await page.getByLabel("Custom model ID").fill(modelId);
}

/**
 * Open the Edit Agent dialog for a seeded managed agent.
 * Opens the agents view, clicks the agent card to open the profile panel,
 * then clicks the Edit quick-action button.
 */
async function openEditDialog(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const agentButton = page.getByRole("button", {
    name: `${agentName} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });

  await page.getByTestId("user-profile-edit-agent").click();

  // Wait for the Edit dialog's provider field (goose runtime supports it).
  await expect(page.locator("#agent-provider")).toBeVisible({
    timeout: 10_000,
  });
}

// Settle any in-flight CSS / Web Animations before capture.
async function settleAnimations(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );
}

test.describe("agent readiness gate screenshots", () => {
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

  // Shot 01: buzz-agent selected, provider empty → required marker shown, save allowed.
  test("01-create-buzzagent-empty-provider-marker", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);

    // Wait for buzz-agent to auto-select and the provider field to render.
    await expect(page.locator("#agent-provider")).toBeVisible({
      timeout: 10_000,
    });

    // Provider empty → required marker shown; submit is now ENABLED.
    // Wait up to 10 s for prereqsQuery to resolve (async even in mock env).
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/01-create-buzzagent-empty-provider-marker.png`,
    });
  });

  // Shot 02: buzz-agent + anthropic selected, model empty → required marker shown, save allowed.
  test("02-create-buzzagent-empty-model-marker", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "anthropic");

    // Model still empty → required marker shown; submit is now ENABLED.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/02-create-buzzagent-empty-model-marker.png`,
    });
  });

  // Shot 03: buzz-agent + anthropic + model set, ANTHROPIC_API_KEY missing →
  // amber required row names the key, submit ENABLED (no longer blocked).
  test("03-create-missing-credential-row", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "anthropic");
    await setCustomModel(page, "claude-opus-4-5");

    // Required row should name ANTHROPIC_API_KEY; submit is now ENABLED.
    await expect(page.getByTestId("env-vars-required-key")).toHaveText(
      "ANTHROPIC_API_KEY",
    );
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 10_000,
    });

    // Scroll the required row into view so it is visible in the screenshot.
    await page.getByTestId("env-vars-required-key").scrollIntoViewIfNeeded();
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/03-create-missing-credential-row.png`,
    });
  });

  // Shot 04: all required fields satisfied → Create button enabled.
  test("04-create-all-required-satisfied-enabled", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "anthropic");
    await setCustomModel(page, "claude-opus-4-5");
    await page
      .getByTestId("env-vars-required-value")
      .fill("sk-test-api-key-for-e2e");

    // All required fields satisfied → submit enabled.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 5_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/04-create-all-required-satisfied-enabled.png`,
    });
  });

  // Shot 05: claude runtime (CLI-login) — provider/model not required, submit enabled.
  // Override the catalog to make claude fully available so it appears in the dropdown.
  test("05-create-cli-login-runtime-no-provider-required", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        {
          id: "buzz-agent",
          label: "Buzz Agent",
          avatar_url: "",
          availability: "available",
          command: "buzz-agent",
          binary_path: "/usr/local/bin/buzz-agent",
          default_args: [],
          mcp_command: "buzz-dev-mcp",
          install_hint: "Ships with the Buzz desktop app.",
          install_instructions_url: "https://github.com/block/buzz",
          can_auto_install: false,
          underlying_cli_path: null,
        },
        {
          id: "claude",
          label: "Claude Code",
          avatar_url: "",
          availability: "available",
          command: "/usr/local/bin/claude-agent",
          binary_path: "/usr/local/bin/claude-agent",
          default_args: ["acp"],
          mcp_command: null,
          install_hint: "Install the Claude Code ACP adapter via npm.",
          install_instructions_url:
            "https://www.npmjs.com/package/@anthropic-ai/claude-agent-acp",
          can_auto_install: true,
          underlying_cli_path: "/usr/local/bin/claude",
        },
      ],
    });

    await openCreateDialog(page);

    // Wait for buzz-agent to auto-select (provider field visible), then
    // switch to claude.
    await expect(page.locator("#agent-provider")).toBeVisible({
      timeout: 10_000,
    });
    await page.locator("#agent-runtime").selectOption("claude");

    // Provider/model fields hidden for CLI-login runtimes.
    await expect(page.locator("#agent-provider")).not.toBeVisible();
    // Submit enabled without provider/model.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 5_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/05-create-cli-login-runtime-no-provider-required.png`,
    });
  });

  // Shot 07: Edit dialog for an existing managed agent (goose runtime) showing
  // the shared AgentProviderField / AgentModelField extraction.
  test("07-edit-dialog-extracted-fields", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: EDIT_AGENT_PUBKEY,
          name: "Tyler Agent",
          status: "stopped" as const,
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page, "Tyler Agent");
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/07-edit-dialog-extracted-fields.png`,
    });
  });

  // Shot 08: goose runtime, provider empty → required marker shown, save allowed (same as buzz-agent).
  test("08-create-goose-empty-provider-marker", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);

    // Buzz-agent auto-selects first; wait for its provider field, then
    // switch to goose to confirm its required-marker behavior is identical.
    await expect(page.locator("#agent-provider")).toBeVisible({
      timeout: 10_000,
    });
    await page.locator("#agent-runtime").selectOption("goose");

    // Provider field still visible for goose (also a provider-selection runtime).
    await expect(page.locator("#agent-provider")).toBeVisible({
      timeout: 5_000,
    });
    // Required marker shown; submit is now ENABLED.
    await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/08-create-goose-empty-provider-marker.png`,
    });
  });
});
