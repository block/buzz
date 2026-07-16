import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-readiness";

// An existing goose-runtime managed agent for the Edit-dialog shot.
// Tyler's pubkey maps to gooseSurface in the mock bridge (runtimeId: "goose"),
// which supports LLM provider selection — the edit dialog's provider/model
// pickers render for it just as they do for buzz-agent.
const EDIT_AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;

/**
 * Navigate to the agents view and open the unified agent-create dialog via
 * the "New agent" menu item, then fill a placeholder name.
 */
async function openCreateDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill("Test Agent");
}

/**
 * Pick an option from a PersonaDropdownField (menu-based, not a native
 * select): open the trigger, click the matching menuitemradio.
 */
async function selectDropdownOption(
  page: import("@playwright/test").Page,
  trigger: import("@playwright/test").Locator,
  optionName: string | RegExp,
) {
  await expect(trigger).toBeVisible({ timeout: 10_000 });
  await trigger.press("Enter");
  await page
    .getByRole("menuitemradio", { name: optionName })
    .click({ timeout: 5_000 });
}

/**
 * Wait for the LLM provider field to become visible (buzz-agent
 * auto-selected) then select the given provider option.
 */
async function selectProvider(
  page: import("@playwright/test").Page,
  providerName: string,
) {
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
  await selectDropdownOption(
    page,
    page.locator("#persona-llm-provider"),
    providerName,
  );
}

/**
 * Choose "Custom model..." from the model combobox and fill a custom model id.
 */
async function setCustomModel(
  page: import("@playwright/test").Page,
  modelId: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("discover_agent_models");
  await page.locator("#persona-model").click();
  await page
    .getByRole("button", { name: "Custom model...", exact: true })
    .click();
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

  // Wait for the Edit dialog's LLM provider field (goose runtime supports it).
  // The Edit dialog renders provider selection via PersonaDropdownField, whose
  // trigger button carries this id.
  await expect(page.locator("#edit-agent-llm-provider")).toBeVisible({
    timeout: 10_000,
  });
}

// Settle any in-flight CSS / Web Animations before capture. allSettled, not
// all: a dropdown-menu close cancels its animation, which rejects `finished`.
async function settleAnimations(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.allSettled(document.getAnimations().map((a) => a.finished)),
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

  // Shot 01: inherited AI defaults are an explicit, valid choice. Provider and
  // model controls stay hidden until the user chooses to customize this agent.
  test("01-create-buzzagent-uses-ai-defaults", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
      },
    });
    await openCreateDialog(page);

    await expect(
      page.getByRole("tab", { name: "Use AI defaults" }),
    ).toHaveAttribute("data-state", "active");
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    await expect(page.locator("#persona-model")).not.toBeVisible();
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    await page.getByRole("dialog").screenshot({
      path: `${SHOTS}/01-create-buzzagent-uses-ai-defaults.png`,
    });
  });

  // Shot 02: customized Anthropic configuration uses Automatic model instead
  // of presenting an obsolete model-required error.
  test("02-create-buzzagent-automatic-model", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "Buzz shared compute");

    await expect(page.locator("#persona-model")).toContainText("Automatic");
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled();
    await settleAnimations(page);

    await page.getByRole("dialog").screenshot({
      path: `${SHOTS}/02-create-buzzagent-automatic-model.png`,
    });
  });

  test("03-create-missing-credential-shows-top-level-required-field", async ({
    page,
  }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "Anthropic");
    await setCustomModel(page, "claude-opus-4-5");

    await expect(page.getByLabel("Anthropic API Key")).toBeVisible();
    await expect(page.getByTestId("persona-dialog-submit")).toBeDisabled({
      timeout: 10_000,
    });
    await expect(
      page.getByRole("button", { name: "Advanced", exact: true }),
    ).toHaveAttribute("aria-expanded", "false");

    const createCountBefore = await page.evaluate(
      () =>
        (
          window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
        ).__BUZZ_E2E_COMMANDS__?.filter(
          (command) => command === "create_persona",
        ).length ?? 0,
    );
    await page
      .locator("#persona-dialog-form")
      .evaluate((form) => (form as HTMLFormElement).requestSubmit());
    await expect
      .poll(async () =>
        page.evaluate(
          () =>
            (
              window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
            ).__BUZZ_E2E_COMMANDS__?.filter(
              (command) => command === "create_persona",
            ).length ?? 0,
        ),
      )
      .toBe(createCountBefore);
  });

  test("04-create-top-level-api-key-enables-submit", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);
    await selectProvider(page, "Anthropic");
    await setCustomModel(page, "claude-opus-4-5");
    await page.getByLabel("Anthropic API Key").fill("sk-test-api-key-for-e2e");

    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 5_000,
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

    // Switch the auto-selected buzz-agent runtime to Claude Code.
    await selectDropdownOption(
      page,
      page.locator("#persona-runtime"),
      "Claude Code",
    );

    // Provider/model fields hidden for CLI-login runtimes.
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    // Submit enabled without provider/model.
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 5_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/05-create-cli-login-runtime-no-provider-required.png`,
    });
  });

  // Shot 07: Edit dialog for an existing managed agent (goose runtime) showing
  // the provider/model pickers.
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

  // Shot 08: goose runtime, provider empty + no global → save BLOCKED (same rule as buzz-agent).
  test("08-create-goose-empty-provider-marker", async ({ page }) => {
    await installMockBridge(page);
    await openCreateDialog(page);

    // Opt into per-agent customization, then switch to goose to confirm a
    // genuinely incomplete customized configuration remains blocked.
    await page.getByRole("tab", { name: "Customize for this agent" }).click();
    await expect(page.locator("#persona-llm-provider")).toBeVisible({
      timeout: 10_000,
    });
    await selectDropdownOption(page, page.locator("#persona-runtime"), "Goose");

    // Provider field still visible for goose (also a provider-selection runtime).
    await expect(page.locator("#persona-llm-provider")).toBeVisible({
      timeout: 5_000,
    });
    // Provider empty + no global provider → save is BLOCKED per the
    // provider-default rule. The submit button must be disabled.
    await expect(page.getByTestId("persona-dialog-submit")).toBeDisabled({
      timeout: 10_000,
    });
    await settleAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/08-create-goose-empty-provider-marker.png`,
    });
  });
});
