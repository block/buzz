import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/global-agent-config";

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
  await page.getByRole("menuitem", { name: "Create from scratch" }).click();
  await page.locator("#persona-display-name").fill("Test Agent");
}

async function customizeAgentAi(page: import("@playwright/test").Page) {
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
}

/**
 * Pick an option from a PersonaDropdownField (menu-based, not a native
 * <select>): focus the trigger, open it, then click the matching
 * menuitemradio. Mirrors the helper in agent-readiness-screenshots.spec.ts.
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

// A runtime catalog with both a provider-selection runtime (buzz-agent) and a
// CLI-login runtime (Claude Code) marked available, so Claude Code appears and
// is selectable in the harness dropdown. Same shape the readiness spec uses.
const CATALOG_WITH_CLAUDE = [
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
];

// A runtime catalog with Codex marked available (the default catalog ships it
// as `not_installed`). Codex is a CLI-login runtime — it drives its own
// provider, so the definition dialog hides the provider picker for it. Used by
// the Edit/Save-mode test to seed an editable Codex agent.
const CATALOG_WITH_CODEX = [
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
    id: "codex",
    label: "Codex",
    avatar_url: "",
    availability: "available",
    command: "/usr/local/bin/codex-agent",
    binary_path: "/usr/local/bin/codex-agent",
    default_args: ["acp"],
    mcp_command: null,
    install_hint: "The codex-acp adapter must be built from source.",
    install_instructions_url: "https://github.com/openai/codex",
    can_auto_install: false,
    underlying_cli_path: "/usr/local/bin/codex",
  },
];

// A catalog where every runtime is unavailable (not installed). With nothing
// available, getDefaultPersonaRuntime returns null, so the definition dialog's
// runtime auto-seed effect is a no-op and a runtime-less definition keeps its
// empty runtime — the precondition for blankRuntimeModelProviderEditable.
const CATALOG_NONE_AVAILABLE = [
  {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatar_url: "",
    availability: "not_installed",
    command: "buzz-agent",
    binary_path: null,
    default_args: [],
    mcp_command: "buzz-dev-mcp",
    install_hint: "Ships with the Buzz desktop app.",
    install_instructions_url: "https://github.com/block/buzz",
    can_auto_install: false,
    underlying_cli_path: null,
  },
  {
    id: "goose",
    label: "Goose",
    avatar_url: "",
    availability: "not_installed",
    command: "goose",
    binary_path: null,
    default_args: [],
    mcp_command: null,
    install_hint: "Install Goose to use this runtime.",
    install_instructions_url: "https://github.com/block/goose",
    can_auto_install: false,
    underlying_cli_path: null,
  },
];

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

  // Shot 01: AgentDefaultsSettingsCard populated with provider + model +
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
    await waitForAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/01-global-agent-config-card-populated.png`,
    });
  });

  test("settings renders and saves the persisted preferred harness", async ({
    page,
  }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        preferred_runtime: "claude",
        provider: null,
        model: null,
        env_vars: {},
      },
      acpRuntimesCatalog: [...CATALOG_WITH_CLAUDE, CATALOG_WITH_CODEX[1]],
    });

    await openAiDefaultsSettings(page);

    const harness = page.getByTestId("global-agent-default-harness");
    await expect(harness).toHaveText("Claude Code");
    await expect(page.getByText("Provider", { exact: true })).toHaveCount(0);
    await expect(page.locator("#global-agent-model")).toBeVisible();

    // Make the form dirty, then return to Claude with no model override. The
    // harness-native default keeps the now-actionable Save button enabled.
    await harness.press("Enter");
    await page.getByTestId("global-agent-default-harness-option-codex").click();
    await harness.press("Enter");
    await page
      .getByTestId("global-agent-default-harness-option-claude")
      .click();
    await expect(page.getByTestId("global-agent-model")).toHaveText(
      /Default model/,
    );
    await expect(
      page.getByRole("button", { name: "Save defaults" }),
    ).toBeEnabled();

    await harness.press("Enter");
    await page.getByTestId("global-agent-default-harness-option-codex").click();
    const model = page.getByTestId("global-agent-model");
    await model.click();
    await page.getByTestId("global-agent-model-option-gpt-5.5[high]").click();
    await page.getByRole("button", { name: "Save defaults" }).click();

    const saved = await page.evaluate(async () =>
      (
        window as typeof window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload: unknown,
          ) => Promise<unknown>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
    );
    expect(saved).toMatchObject({ preferred_runtime: "codex" });
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

    // The footer must explain WHY it is disabled (regression guard for the
    // submitBlockReason wiring, not just the boolean gate): defaults mode with
    // no resolvable provider names the missing piece and points to Settings.
    const reason = page.getByTestId("persona-dialog-submit-reason");
    await expect(reason).toBeVisible({ timeout: 10_000 });
    await expect(reason).toContainText("provider");
    await expect(reason).toContainText("Settings → AI defaults");

    await waitForAnimations(page);

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
    // The reason is null exactly when the form can submit — no footer reason.
    await expect(page.getByTestId("persona-dialog-submit-reason")).toHaveCount(
      0,
    );

    await waitForAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/05-create-enabled-with-global-provider.png`,
    });
  });

  // Shot 09: CLI-login runtime (Claude Code / Codex) drives its own provider,
  // so the provider picker is intentionally hidden. This is Ian's regression:
  // before the provider-aware gate, the hidden provider left the button
  // permanently disabled with no explanation. Now the provider is not required,
  // the button is enabled, and — critically — no spurious provider reason is
  // shown in the footer. Create and Save share this rendering path.
  test("09-cli-login-runtime-enabled-no-reason", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: CATALOG_WITH_CLAUDE,
    });

    await openCreateDialog(page);

    // Switch the auto-selected buzz-agent runtime to the CLI-login runtime.
    await selectDropdownOption(
      page,
      page.locator("#persona-runtime"),
      "Claude Code",
    );

    // Provider picker hidden — the runtime drives its own provider.
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    // The hidden provider must not block submit, and must not surface a reason.
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await expect(page.getByTestId("persona-dialog-submit-reason")).toHaveCount(
      0,
    );

    await waitForAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/09-cli-login-runtime-enabled-no-reason.png`,
    });
  });

  // Shot 10: the ORIGINAL defect — Ian's "Save button stays disabled after
  // editing an agent." This drives the real EDIT/Save path (not create): a
  // persona-linked Codex agent with an explicit custom model and no provider is
  // opened via the Agents view → profile → Edit affordance, which mounts
  // AgentDefinitionDialog in edit mode (id present in initialValues, "Save
  // changes" label). Before the provider-aware gate, the hidden Codex provider
  // left Save permanently disabled on a value the user could never set. Now:
  // provider picker hidden, Save enabled, and no submit-block reason. Create
  // and Save share this rendering path, but the defect was Save-specific, so
  // this exercises Save directly.
  test("10-edit-codex-custom-model-save-enabled-no-reason", async ({
    page,
  }) => {
    const PERSONA_ID = "persona-codex-edit-e2e";
    await installMockBridge(page, {
      acpRuntimesCatalog: CATALOG_WITH_CODEX,
      managedAgents: [
        {
          pubkey: TEST_IDENTITIES.tyler.pubkey,
          name: "Codex Editor",
          personaId: PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Codex Editor",
          systemPrompt: "You are the Codex edit-mode e2e persona.",
          // CLI-login runtime with an explicit custom model and NO provider —
          // the exact shape that used to pin Save disabled.
          runtime: "codex",
          model: "gpt-5-codex",
          provider: null,
        },
      ],
    });

    // Agents view → persona-grouped agent card → Edit quick action.
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    const agentButton = page.getByRole("button", {
      name: "Codex Editor agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    // The definition dialog opens in EDIT mode ("Save changes"), seeded from
    // the persona — confirm it's the edit path, not create.
    await expect(page.getByTestId("persona-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator("#persona-display-name")).toHaveValue(
      "Codex Editor",
    );
    await expect(page.getByTestId("persona-dialog-submit")).toHaveText(
      /Save changes/,
    );

    // The core assertions: Codex hides the provider picker, so the hidden
    // provider must NOT block Save and must NOT surface a reason.
    await expect(page.locator("#persona-llm-provider")).not.toBeVisible();
    await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
      timeout: 10_000,
    });
    await expect(page.getByTestId("persona-dialog-submit-reason")).toHaveCount(
      0,
    );

    await waitForAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/10-edit-codex-custom-model-save-enabled-no-reason.png`,
    });
  });

  // Shot 11: the inverse of Ian's fix, and wesbillman's blocking review point.
  // A runtime-LESS legacy/builtin definition (no runtime, but a saved model)
  // still EXPOSES the provider picker via blankRuntimeModelProviderEditable, so
  // an empty provider must keep Save DISABLED. The gate must key off the field's
  // visibility (runtimeCanChooseLlmProvider), not the raw runtime capability —
  // otherwise Save persists `provider: undefined` despite the visible picker.
  // A global provider/model default keeps localMode satisfied, so the ONLY thing
  // that can block Save here is the Customize-pair provider gate (step 7), which
  // is exactly what this regression pins.
  test("11-edit-runtime-less-provider-required-save-blocked", async ({
    page,
  }) => {
    const PERSONA_ID = "persona-runtime-less-edit-e2e";
    await installMockBridge(page, {
      // No runtime is available, so getDefaultPersonaRuntime returns null and
      // the dialog does NOT auto-seed a runtime on open — the runtime-less
      // definition stays runtime-less, which is the only state where
      // blankRuntimeModelProviderEditable exposes the provider picker.
      acpRuntimesCatalog: CATALOG_NONE_AVAILABLE,
      // Global defaults satisfy localMode, so any block is the pair gate alone.
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { ANTHROPIC_API_KEY: "sk-ant-global-value" },
      },
      managedAgents: [
        {
          pubkey: TEST_IDENTITIES.tyler.pubkey,
          name: "Legacy Editor",
          personaId: PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Legacy Editor",
          systemPrompt: "You are the runtime-less edit-mode e2e persona.",
          // Runtime-less definition with a saved model and NO provider — the
          // picker is editable-without-runtime, so the provider stays required.
          runtime: null,
          model: "claude-opus-4-5",
          provider: null,
        },
      ],
    });

    // Agents view → persona-grouped agent card → Edit quick action.
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    const agentButton = page.getByRole("button", {
      name: "Legacy Editor agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    // Confirm the real EDIT dialog, seeded from the persona.
    await expect(page.getByTestId("persona-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator("#persona-display-name")).toHaveValue(
      "Legacy Editor",
    );
    await expect(page.getByTestId("persona-dialog-submit")).toHaveText(
      /Save changes/,
    );

    // The provider picker IS visible (runtime-less editable definition) …
    await expect(page.locator("#persona-llm-provider")).toBeVisible({
      timeout: 10_000,
    });
    // … so the empty provider must block Save …
    await expect(page.getByTestId("persona-dialog-submit")).toBeDisabled({
      timeout: 10_000,
    });
    // … and the reason must be the Customize-pair provider gate, not the
    // global-defaults gate (which would say "Settings → AI defaults").
    const reason = page.getByTestId("persona-dialog-submit-reason");
    await expect(reason).toBeVisible({ timeout: 10_000 });
    await expect(reason).toContainText("Select a provider");
    await expect(reason).not.toContainText("Settings → AI defaults");

    await waitForAnimations(page);

    const dialog = page.getByRole("dialog");
    await dialog.screenshot({
      path: `${SHOTS}/11-edit-runtime-less-provider-required-save-blocked.png`,
    });
  });
});
