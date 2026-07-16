/**
 * Screenshot-regression spec for PR #1764: provider/model dropdown fixes.
 *
 * Three states that previously regressed on fresh OSS installs:
 *
 *  01 – Global agent config provider select renders both Databricks v1 and v2
 *       options (they are always shown on OSS builds; the mock bridge returns an
 *       empty baked env, simulating an OSS install with no BUZZ_AGENT_PROVIDER).
 *
 *  02 – Effort dropdown shows "Default (medium)" instead of bare "Inherit" when
 *       no effort is baked and the provider uses a known default effort level.
 *       (provider unset → effortDefault = "medium" → inheritFallbackLabel fires)
 *
 *  03 – Edit dialog for a definition with runtime null auto-seeds the app-default
 *       runtime (buzz-agent) and model discovery runs → model combobox is non-empty.
 *       Previously, the seeding effect bailed in edit mode, leaving the runtime
 *       empty and the model dropdown silently blank.
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/screenshots-dialogs";

/**
 * Open Settings → Agents through the app UI and wait for the defaults card to
 * finish loading. The CI static server does not provide SPA fallbacks for a
 * direct `/settings` request.
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
  // The card shows a spinner during the async load effect; wait for it to clear.
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

test.describe("agent provider dropdown screenshots", () => {
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

  // Shot 01: OSS provider dropdown includes both Databricks v1 and v2.
  //
  // The mock bridge returns get_baked_build_env_keys = [] (OSS), so no
  // BUZZ_AGENT_PROVIDER is baked and hideProviderIds is empty → v1 appears.
  test("01-provider-dropdown-oss", async ({ page }) => {
    await installMockBridge(page);
    await openAiDefaultsSettings(page);

    const providerSelect = page.locator("#global-agent-provider");
    await expect(providerSelect).toBeVisible({ timeout: 5_000 });

    // Regression: both v1 and v2 must be present on OSS.
    const optionTexts = await providerSelect.evaluate((el: HTMLSelectElement) =>
      Array.from(el.options).map((o) => o.text.trim()),
    );
    expect(optionTexts).toContain("Databricks");
    expect(optionTexts).toContain("Databricks v2");

    await waitForAnimations(page);

    // Screenshot the card — provider select is closed but the assertion above
    // proves both Databricks options are present in the option list.
    await page
      .getByTestId("settings-global-agent-config")
      .screenshot({ path: `${SHOTS}/01-provider-dropdown-oss.png` });
  });

  // Shot 02: Effort dropdown shows "Default (medium)" (no baked effort,
  // provider=databricks_v2 with no model → effortDefault = "medium").
  //
  // getProviderEffortConfig("databricks_v2", "") hits the blank-model branch:
  //   → { defaultValue: "medium" }
  // inheritFallbackLabel = "Default (medium)", bakedEffort = null (OSS)
  // → the zero-value option reads "Default (medium)" instead of bare "Inherit".
  //
  // Using databricks_v2 here (rather than no provider) makes this card
  // visually distinct from shot 01 — the provider select shows "Databricks v2"
  // as the selected value — so the two PNG hashes differ.
  test("02-effort-default-label", async ({ page }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        provider: "databricks_v2",
        model: null,
        env_vars: {},
      },
    });
    await openAiDefaultsSettings(page);

    const effortSelect = page.locator("#global-agent-thinking-effort");
    await expect(effortSelect).toBeVisible({ timeout: 5_000 });

    // Regression: the zero-value option must show the provider's default
    // effort rather than a bare "Inherit".
    const zeroOptionText = await effortSelect.evaluate(
      (el: HTMLSelectElement) =>
        Array.from(el.options)
          .find((o) => o.value === "")
          ?.text.trim() ?? "",
    );
    expect(zeroOptionText).toBe("Default (medium)");

    await waitForAnimations(page);

    await page
      .getByTestId("settings-global-agent-config")
      .screenshot({ path: `${SHOTS}/02-effort-default-label.png` });
  });

  // Shot 03: Edit dialog for a definition with null runtime auto-seeds the
  // default runtime (buzz-agent via getDefaultPersonaRuntime) and model
  // discovery runs, producing a non-empty model combobox.
  //
  // Previously the seeding effect bailed in edit mode ("id" in initialValues),
  // leaving runtime = "" → modelFieldVisible = false → discovery never ran →
  // the model dropdown was silently empty.
  test("03-builtin-edit-runtime-seeded", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          displayName: "Null Runtime Agent",
          systemPrompt: "An agent with no runtime configured.",
          // runtime/provider/model not set → all null in the mock, so the
          // edit dialog must auto-seed the app default runtime.
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await expect(page.getByTestId("agents-library-personas")).toBeVisible({
      timeout: 10_000,
    });

    // Open the persona's actions menu (visible for non-builtin personas).
    const actionsBtn = page.getByRole("button", {
      name: "Open actions for Null Runtime Agent",
    });
    await expect(actionsBtn).toBeVisible({ timeout: 8_000 });
    await actionsBtn.click();

    await page.getByRole("menuitem", { name: "Edit" }).click();

    const dialog = page.getByTestId("persona-dialog");
    await expect(dialog).toBeVisible({ timeout: 10_000 });

    // Regression: the runtime trigger must not be empty — the auto-seed effect
    // must have run and selected the app default (buzz-agent in the mock catalog).
    const runtimeTrigger = dialog.locator("#persona-runtime");
    await expect(runtimeTrigger).toBeVisible({ timeout: 8_000 });
    await expect(runtimeTrigger).not.toContainText("No preference", {
      timeout: 8_000,
    });

    await dialog.getByRole("tab", { name: "Customize for this agent" }).click();

    // Regression: the model combobox must appear (modelFieldVisible = true once
    // runtime is non-empty) and model discovery must have run, populating it.
    const modelCombobox = dialog.getByRole("combobox", { name: /model/i });
    await expect(modelCombobox).toBeVisible({ timeout: 8_000 });

    // Open the picker and assert that a known model from the mock discovery
    // response is listed. The mock returns "Claude Opus 4.6" for all providers
    // (see discover_agent_models in e2eBridge.ts). A zero-model discovery
    // regression would leave the list empty and this assertion would fail.
    // The picker is a searchable command popover portaled outside the dialog
    // whose items render as buttons — query at page level.
    await modelCombobox.click();
    await expect(
      page.getByRole("button", { name: /Claude Opus 4\.6/i }),
    ).toBeVisible({ timeout: 5_000 });
    // Close the popover so the screenshot captures the dialog's resting state.
    await page.keyboard.press("Escape");
    await expect(
      page.getByRole("button", { name: /Claude Opus 4\.6/i }),
    ).toBeHidden();

    await waitForAnimations(page);
    await dialog.screenshot({
      path: `${SHOTS}/03-builtin-edit-runtime-seeded.png`,
    });
  });
});
