/**
 * Playwright regression tests for the numeric tuning fields (max output tokens,
 * context limit, max rounds) on both the global Agent Defaults surface and the
 * per-agent Advanced section.
 *
 * Covers:
 *   1. Global defaults Advanced shows numeric inputs for buzz-agent.
 *   2. Global defaults Advanced hides numeric inputs for non-capable runtimes.
 *   3. Per-agent Goose: saving a max-tokens value globally surfaces as
 *      Inherit (<value>) placeholder in the per-agent edit dialog.
 *   4. Delayed catalog: while loading, saved tuning env vars stay visible
 *      as generic rows (not silently dropped); structured controls appear
 *      once the catalog settles.
 *   5. Failed catalog: when discovery errors, saved tuning env vars remain
 *      visible as generic rows (never the "unsupported" empty state).
 */

import { expect, test } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// ── Helpers ────────────────────────────────────────────────────────────────

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

async function openEditAgentDialog(
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
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
}

// ── Tests ──────────────────────────────────────────────────────────────────

test("global_advanced_buzz_agent_shows_all_numeric_controls", async ({
  page,
}) => {
  // The mock bridge's withMockRuntimeConfigMetadata injects the numeric env var
  // fields for buzz-agent. When buzz-agent is selected and Advanced is opened,
  // all three numeric inputs must be visible.
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
        mcp_command: null,
        install_hint: "Ships with the Buzz desktop app.",
        install_instructions_url: "https://github.com/block/buzz",
        can_auto_install: false,
        underlying_cli_path: null,
        auth_status: { status: "not_applicable" },
      },
    ],
    globalAgentConfig: {
      env_vars: {},
      provider: "anthropic",
      model: null,
      preferred_runtime: "buzz-agent",
    },
  });

  await openAiDefaultsSettings(page);

  // Open the Advanced section. The settings card uses disclosure="full" (no
  // animation wrapper), so we click the toggle and wait for content directly.
  await page.getByTestId("global-agent-advanced-toggle").click();

  // All three numeric inputs must be present for buzz-agent.
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toBeVisible(
    { timeout: 5_000 },
  );
  await expect(page.getByTestId("numeric-context-limit-input")).toBeVisible();
  await expect(page.getByTestId("numeric-max-rounds-input")).toBeVisible();
});

test("global_advanced_non_capable_runtime_hides_numeric_controls", async ({
  page,
}) => {
  // Claude has no numeric tuning env vars (contextLimitEnvVar = null etc.).
  // After selecting Claude, the Advanced section must show no numeric inputs.
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      {
        id: "claude",
        label: "Claude Code",
        avatar_url: "",
        availability: "available",
        command: "/usr/local/bin/claude-agent",
        binary_path: "/usr/local/bin/claude-agent",
        default_args: ["acp"],
        mcp_command: null,
        install_hint: "Install via npm.",
        install_instructions_url: "https://example.com",
        can_auto_install: true,
        underlying_cli_path: "/usr/local/bin/claude",
        auth_status: { status: "logged_in" },
      },
    ],
    globalAgentConfig: {
      env_vars: {},
      provider: null,
      model: null,
      preferred_runtime: "claude",
    },
  });

  await openAiDefaultsSettings(page);

  await page.getByTestId("global-agent-advanced-toggle").click();

  // No numeric inputs must render for a non-capable runtime.
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toHaveCount(
    0,
  );
  await expect(page.getByTestId("numeric-context-limit-input")).toHaveCount(0);
  await expect(page.getByTestId("numeric-max-rounds-input")).toHaveCount(0);
});

test("goose_per_agent_advanced_max_tokens_shows_inherited_global_placeholder", async ({
  page,
}) => {
  // Save GOOSE_MAX_TOKENS = 16384 in the global Agent Defaults settings via
  // the UI, then open a Goose agent's edit dialog. The max-output-tokens input
  // must show "Inherit (16384)" — the globally-saved value surfaced via the
  // inherited placeholder.
  await installMockBridge(page, {
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test-key" },
      provider: "anthropic",
      model: "claude-opus-4-5",
      preferred_runtime: "goose",
    },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.tyler.pubkey,
        name: "Tyler Agent",
        runtime: "goose",
        status: "stopped",
        channelNames: ["agents"],
      },
    ],
  });

  // Step 1: open global defaults, expand Advanced, enter the max-tokens value.
  await openAiDefaultsSettings(page);
  await page.getByTestId("global-agent-advanced-toggle").click();
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toBeVisible(
    { timeout: 5_000 },
  );
  await page.getByTestId("numeric-max-output-tokens-input").click();
  await page
    .getByTestId("numeric-max-output-tokens-input")
    .pressSequentially("16384");
  // Blur to ensure React's change event fires for the number input.
  await page.keyboard.press("Tab");

  // Step 2: save the global defaults.
  await expect(page.getByRole("button", { name: "Save defaults" })).toBeEnabled(
    { timeout: 5_000 },
  );
  await page.getByRole("button", { name: "Save defaults" }).click();
  // Wait for the save to complete: the button returns to disabled (dirty resets).
  await expect(
    page.getByRole("button", { name: "Save defaults" }),
  ).toBeDisabled({ timeout: 5_000 });

  // Step 3: navigate back and open the per-agent edit dialog for the Goose
  // agent. We use the app's Back link rather than page.goto("/") to preserve
  // the in-memory mock state (page.goto causes a full reload that resets it).
  await page.getByRole("button", { name: "Back to app" }).click();
  await page.getByTestId("open-agents-view").click();
  const agentButton = page.getByRole("button", {
    name: "Tyler Agent agent profile",
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });

  // Wait for the provider field — signals the catalog and dialog have settled.
  await expect(page.locator("#edit-agent-llm-provider")).toBeVisible({
    timeout: 10_000,
  });

  // Open the Advanced section.
  await page.getByRole("button", { name: "Advanced", exact: true }).click();
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toBeVisible(
    { timeout: 5_000 },
  );

  // The placeholder must reflect the globally-saved value.
  await expect(
    page.getByTestId("numeric-max-output-tokens-input"),
  ).toHaveAttribute("placeholder", "Inherit (16384)");
});

test("delayed_catalog_per_agent_saved_tuning_values_visible_then_structured_controls_appear", async ({
  page,
}) => {
  // Scenario: catalog takes 5 seconds to respond (simulates slow discovery).
  // The per-agent edit dialog opens. While the catalog is still in flight,
  // saved tuning env vars must not be dropped from view — they appear as
  // generic env rows (no hiddenKeys applied yet). Once the catalog settles,
  // the structured numeric controls replace the generic rows.
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
        mcp_command: null,
        install_hint: "Ships with the Buzz desktop app.",
        install_instructions_url: "https://github.com/block/buzz",
        can_auto_install: false,
        underlying_cli_path: null,
        auth_status: { status: "not_applicable" },
      },
    ],
    // 5-second delay: generous enough that the dialog opens and Advanced is
    // expanded while the catalog query is still in-flight (navigation takes
    // ~1-2 s), but short enough to keep the test under 30 s.
    acpRuntimesDelayMs: 5000,
    globalAgentConfig: {
      env_vars: {},
      provider: "anthropic",
      model: null,
      preferred_runtime: "buzz-agent",
    },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.tyler.pubkey,
        name: "Tyler Agent",
        runtime: "buzz-agent",
        status: "stopped",
        channelNames: ["agents"],
        envVars: {
          BUZZ_AGENT_MAX_OUTPUT_TOKENS: "4096",
          BUZZ_AGENT_MAX_ROUNDS: "25",
        },
      },
    ],
  });

  await openEditAgentDialog(page, "Tyler Agent");

  // Open Advanced before the catalog has settled (the dialog opens quickly;
  // the catalog query fires when the dialog opens and takes ~5 seconds).
  await page.getByRole("button", { name: "Advanced", exact: true }).click();

  // While loading: structured numeric controls must NOT be visible yet —
  // the catalog-settling gate withholds them.
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toHaveCount(
    0,
  );

  // The saved tuning env vars must be visible as generic rows (not hidden)
  // while the catalog hasn't settled: BUZZ_AGENT_MAX_OUTPUT_TOKENS and
  // BUZZ_AGENT_MAX_ROUNDS should appear in the env-vars editor.
  await expect(
    page.locator(
      'input[data-testid="env-vars-key"][value="BUZZ_AGENT_MAX_OUTPUT_TOKENS"]',
    ),
  ).toBeVisible();
  await expect(
    page.locator(
      'input[data-testid="env-vars-key"][value="BUZZ_AGENT_MAX_ROUNDS"]',
    ),
  ).toBeVisible();

  // After the catalog settles (allow up to 8 s — 5 s delay + margin):
  // structured controls appear, replacing the generic rows.
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toBeVisible(
    { timeout: 8_000 },
  );
  await expect(page.getByTestId("numeric-max-rounds-input")).toBeVisible({
    timeout: 8_000,
  });
});

test("failed_catalog_per_agent_saved_tuning_values_remain_visible_as_generic_rows", async ({
  page,
}) => {
  // Scenario: catalog discovery fails (network error / IPC rejection).
  // The per-agent edit dialog opens. The saved tuning env vars must remain
  // visible as generic rows — the error state must never produce the
  // "unsupported" no-controls state that would hide persisted values.
  await installMockBridge(page, {
    acpRuntimesError: true,
    globalAgentConfig: {
      env_vars: {},
      provider: "anthropic",
      model: null,
      preferred_runtime: "buzz-agent",
    },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.tyler.pubkey,
        name: "Tyler Agent",
        runtime: "buzz-agent",
        status: "stopped",
        channelNames: ["agents"],
        envVars: {
          BUZZ_AGENT_MAX_OUTPUT_TOKENS: "8192",
          BUZZ_AGENT_MAX_ROUNDS: "10",
        },
      },
    ],
  });

  await openEditAgentDialog(page, "Tyler Agent");

  // Open Advanced after a brief wait (query has had time to fail).
  await page.waitForTimeout(500);
  await page.getByRole("button", { name: "Advanced", exact: true }).click();

  // Structured numeric controls must NOT render (catalog errored — no runtime).
  await expect(page.getByTestId("numeric-max-output-tokens-input")).toHaveCount(
    0,
  );

  // Saved tuning values must still be visible as generic env rows — the error
  // state must never hide persisted values with no editor to replace them.
  await expect(
    page.locator(
      'input[data-testid="env-vars-key"][value="BUZZ_AGENT_MAX_OUTPUT_TOKENS"]',
    ),
  ).toBeVisible({ timeout: 5_000 });
  await expect(
    page.locator(
      'input[data-testid="env-vars-key"][value="BUZZ_AGENT_MAX_ROUNDS"]',
    ),
  ).toBeVisible();
});
