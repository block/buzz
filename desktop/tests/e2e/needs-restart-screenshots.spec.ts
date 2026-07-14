/**
 * Screenshot spec for the needsRestart badge and banner (PR #1853).
 *
 * Exercises two surfaces:
 *   - Agent grid card: warning badge ("Restart required") on standalone and
 *     persona-backed cards when `needsRestart: true`.
 *   - Profile panel Runtime tab: amber banner explaining auto-restart behavior.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/pr-1853-screenshots";

const STANDALONE_AGENT = {
  pubkey: TEST_IDENTITIES.alice.pubkey,
  name: "Local Agent",
  status: "running" as const,
  needsRestart: true,
};

const PERSONA_AGENT = {
  pubkey: TEST_IDENTITIES.bob.pubkey,
  name: "Persona Agent",
  personaId: "builtin:fizz",
  status: "running" as const,
  needsRestart: true,
};

async function gotoAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
}

test.describe("needs-restart screenshots", () => {
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

  test("01-grid-standalone-restart-badge", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    const agentCard = page.getByTestId(
      `managed-agent-${STANDALONE_AGENT.pubkey}`,
    );
    await expect(agentCard).toBeVisible({ timeout: 10_000 });
    await waitForAnimations(page);

    await agentCard.screenshot({
      path: `${SHOTS}/01-grid-standalone-restart-badge.png`,
    });
  });

  test("02-grid-persona-restart-badge", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      managedAgents: [PERSONA_AGENT],
    });

    await gotoAgentsView(page);

    const personaCard = page.getByTestId(
      `persona-agent-row-${PERSONA_AGENT.personaId}`,
    );
    await expect(personaCard).toBeVisible({ timeout: 10_000 });
    await waitForAnimations(page);

    await personaCard.screenshot({
      path: `${SHOTS}/02-grid-persona-restart-badge.png`,
    });
  });

  test("03-runtime-tab-restart-banner", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    // Click the agent card to open the profile panel.
    const agentButton = page.getByRole("button", {
      name: `${STANDALONE_AGENT.name} agent profile`,
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });

    // Switch to the Runtime tab.
    await panel.getByRole("tab", { name: "Runtime" }).click();

    // Wait for the restart banner to appear.
    const banner = panel.getByTestId("needs-restart-banner");
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await waitForAnimations(page);

    await banner.screenshot({
      path: `${SHOTS}/03-runtime-tab-restart-banner.png`,
    });
  });
});
