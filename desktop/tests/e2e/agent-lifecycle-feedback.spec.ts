/**
 * E2E screenshots + regression tests for agent-lifecycle feedback (PR #1766):
 *
 * 1. Persona delete confirm dialog shows "Also deletes N agent instance(s)."
 *    when the persona has linked managed-agent instances.
 * 2. Global-config save reports "Saved. Restarted N agents." when running agents
 *    were restarted.
 * 3. Global-config save reports plain "Saved." when no agents were restarted;
 *    the old "Running agents keep their current settings…" text is gone.
 */

import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/screenshots-lifecycle";

// Persona ID used across the cascade-delete test. Must be a custom (non-builtin)
// persona so the actions menu renders the Delete path.
const CASCADE_PERSONA_ID = "custom:test-cascade";

// Stable fake pubkeys for the two seeded managed agents. 64 lowercase hex chars
// that don't collide with TEST_IDENTITIES pubkeys.
const CASCADE_AGENT_A_PUBKEY = "aa".repeat(32);
const CASCADE_AGENT_B_PUBKEY = "bb".repeat(32);

/**
 * Navigate to the Agents view and wait for the global agent config card to
 * finish loading (spinner gone). The card lives at the bottom of the view.
 */
async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("settings-global-agent-config")).toBeVisible({
    timeout: 10_000,
  });
  // Spinner disappears once the load effect resolves.
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

async function openAiDefaultsSettings(page: import("@playwright/test").Page) {
  await page.goto("/settings?section=agents");
  await expect(page.getByTestId("settings-global-agent-config")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

test.describe("agent lifecycle feedback screenshots", () => {
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

  // Shot 01: persona delete confirm dialog — "Also deletes 2 agent instance(s)."
  // Seeds a custom persona with two linked managed agents so instanceCount = 2.
  // Triggers the delete confirm from the persona's "..." actions menu.
  test("01-delete-cascade-copy", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          id: CASCADE_PERSONA_ID,
          displayName: "Cascade Test Agent",
          systemPrompt: "A test persona for cascade delete E2E coverage.",
          isActive: true,
        },
      ],
      managedAgents: [
        {
          pubkey: CASCADE_AGENT_A_PUBKEY,
          name: "Cascade Instance A",
          personaId: CASCADE_PERSONA_ID,
          status: "stopped",
        },
        {
          pubkey: CASCADE_AGENT_B_PUBKEY,
          name: "Cascade Instance B",
          personaId: CASCADE_PERSONA_ID,
          status: "running",
        },
      ],
    });

    await openAgentsView(page);

    // The custom persona card appears in the library.
    await expect(
      page.getByText("Cascade Test Agent", { exact: true }),
    ).toBeVisible({ timeout: 10_000 });

    // Open the actions menu for the custom persona. The trigger button carries
    // an aria-label derived from the persona displayName.
    await page
      .getByRole("button", { name: "Open actions for Cascade Test Agent" })
      .click();

    // For a custom (non-builtin) persona, Delete opens PersonaDeleteDialog.
    await page.getByRole("menuitem", { name: "Delete" }).click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Core assertion: the cascade copy shows the correct instance count (plural).
    await expect(dialog).toContainText("Also deletes 2 agent instances.");

    await waitForAnimations(page);

    await dialog.screenshot({
      path: `${SHOTS}/01-delete-cascade-copy.png`,
    });
  });

  // Shot 02: global config save with restarts — "Saved. Restarted 2 agents."
  // The mock is configured to return restarted_count=2 so the card shows the
  // restart-count feedback.
  test("02-save-restarted", async ({ page }) => {
    await installMockBridge(page, {
      globalConfigRestartedCount: 2,
    });

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");

    // Change the provider to make the card dirty (enables "Save defaults").
    await page.locator("#global-agent-provider").selectOption("anthropic");
    await expect(
      page.getByRole("button", { name: "Save defaults" }),
    ).toBeEnabled({ timeout: 5_000 });

    await page.getByRole("button", { name: "Save defaults" }).click();

    // Core assertion: the restart-count feedback is visible.
    await expect(card.getByText("Saved. Restarted 2 agents.")).toBeVisible({
      timeout: 5_000,
    });

    await waitForAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/02-save-restarted.png`,
    });
  });

  // Shot 03: global config save with no restarts — plain "Saved."
  // The default mock returns restarted_count=0. The old text
  // "Running agents keep their current settings until restarted." must be absent.
  test("03-save-plain", async ({ page }) => {
    await installMockBridge(page);

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");

    // Change the provider to make the card dirty.
    await page.locator("#global-agent-provider").selectOption("anthropic");
    await expect(
      page.getByRole("button", { name: "Save defaults" }),
    ).toBeEnabled({ timeout: 5_000 });

    await page.getByRole("button", { name: "Save defaults" }).click();

    // Core assertion: plain "Saved." with no restart count.
    // exact: true prevents matching "Saved. Restarted N agents." as a substring.
    await expect(card.getByText("Saved.", { exact: true })).toBeVisible({
      timeout: 5_000,
    });

    // Regression guard: the old stale-env message must not appear.
    await expect(
      card.getByText("Running agents keep their current settings"),
    ).not.toBeVisible();

    await waitForAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/03-save-plain.png`,
    });
  });

  // Shot 04: persona delete confirm dialog — singular "Also deletes 1 agent instance."
  // One linked instance → singular copy (no extra "s").
  test("04-delete-cascade-singular", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          id: CASCADE_PERSONA_ID,
          displayName: "Cascade Test Agent",
          systemPrompt: "A test persona.",
          isActive: true,
        },
      ],
      managedAgents: [
        {
          pubkey: CASCADE_AGENT_A_PUBKEY,
          name: "Cascade Instance A",
          personaId: CASCADE_PERSONA_ID,
          status: "stopped",
        },
      ],
    });

    await openAgentsView(page);

    await expect(
      page.getByText("Cascade Test Agent", { exact: true }),
    ).toBeVisible({ timeout: 10_000 });

    await page
      .getByRole("button", { name: "Open actions for Cascade Test Agent" })
      .click();
    await page.getByRole("menuitem", { name: "Delete" }).click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // Singular copy: "Also deletes 1 agent instance." (not "instances").
    await expect(dialog).toContainText("Also deletes 1 agent instance.");

    await waitForAnimations(page);
    await dialog.screenshot({
      path: `${SHOTS}/04-delete-cascade-singular.png`,
    });
  });

  // Shot 05: persona delete confirm dialog — zero linked instances.
  // No managed agents linked to the persona → "Also deletes…" line absent.
  test("05-delete-cascade-zero-instances", async ({ page }) => {
    await installMockBridge(page, {
      personas: [
        {
          id: CASCADE_PERSONA_ID,
          displayName: "Cascade Test Agent",
          systemPrompt: "A test persona.",
          isActive: true,
        },
      ],
      managedAgents: [],
    });

    await openAgentsView(page);

    await expect(
      page.getByText("Cascade Test Agent", { exact: true }),
    ).toBeVisible({ timeout: 10_000 });

    await page
      .getByRole("button", { name: "Open actions for Cascade Test Agent" })
      .click();
    await page.getByRole("menuitem", { name: "Delete" }).click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    // No linked instances → no cascade warning.
    await expect(dialog).not.toContainText("Also deletes");
  });

  // Shot 06: global config save — singular "Saved. Restarted 1 agent."
  test("06-save-restarted-singular", async ({ page }) => {
    await installMockBridge(page, {
      globalConfigRestartedCount: 1,
    });

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");

    await page.locator("#global-agent-provider").selectOption("anthropic");
    await expect(
      page.getByRole("button", { name: "Save defaults" }),
    ).toBeEnabled({ timeout: 5_000 });

    await page.getByRole("button", { name: "Save defaults" }).click();

    // Singular copy: "Restarted 1 agent." (not "agents.").
    await expect(card.getByText("Saved. Restarted 1 agent.")).toBeVisible({
      timeout: 5_000,
    });
  });

  // Shot 07: global config save — partial failure "M failed to restart".
  test("07-save-failed-restart", async ({ page }) => {
    await installMockBridge(page, {
      globalConfigFailedRestartCount: 1,
    });

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");

    await page.locator("#global-agent-provider").selectOption("anthropic");
    await expect(
      page.getByRole("button", { name: "Save defaults" }),
    ).toBeEnabled({ timeout: 5_000 });

    await page.getByRole("button", { name: "Save defaults" }).click();

    // Partial failure copy (zero restarted): singular agent + Agents tab prompt.
    await expect(
      card.getByText(
        "Saved. 1 agent failed to restart — check the Agents tab.",
      ),
    ).toBeVisible({ timeout: 5_000 });

    await waitForAnimations(page);
    await card.screenshot({
      path: `${SHOTS}/07-save-failed-restart.png`,
    });
  });

  // Shot 08: an edit made while a save is in flight must survive the save
  // resolving — the card keeps the newer value and stays dirty instead of
  // clobbering it with the older response.
  test("08-save-race-keeps-newer-edit", async ({ page }) => {
    await installMockBridge(page, {
      globalConfigSaveDelayMs: 2_000,
    });

    await openAiDefaultsSettings(page);

    const card = page.getByTestId("settings-global-agent-config");
    const provider = page.locator("#global-agent-provider");
    const saveButton = page.getByRole("button", { name: "Save defaults" });

    await provider.selectOption("anthropic");
    await expect(saveButton).toBeEnabled({ timeout: 5_000 });
    await saveButton.click();

    // While the save is held open by the mock delay, make a newer edit.
    await provider.selectOption("openai");

    // The save resolves with the OLD submitted config; the newer edit must
    // survive and the card must stay dirty so it can be saved again.
    await expect(card.getByText("Saved.", { exact: true })).toBeVisible({
      timeout: 10_000,
    });
    await expect(provider).toHaveValue("openai");
    await expect(saveButton).toBeEnabled();
  });
});
