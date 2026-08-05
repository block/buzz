/**
 * Screenshot spec for the needsRestart badge and config diff (PR #1853 + diff
 * overlay work).
 *
 * Exercises:
 *   - Agent grid card and list-row badges at all three badge sites.
 *   - Hover tooltip with itemised before→after diff (capped at 6 + "and N more").
 *   - Runtime-tab banner with full uncapped diff list.
 *   - Side-panel badge visible on the default (Info) tab — not only Runtime.
 *   - DOM validity: tooltip trigger has no <button> ancestor.
 *   - Generic rendering: unknown field ids, number/array values, masked values.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/restart-diff-screenshots";

// ── Sample diff entries ───────────────────────────────────────────────────────

const DIFF_ENTRIES = [
  {
    field: "model",
    change: {
      kind: "value" as const,
      before: "gpt-4",
      after: "claude-3-5-sonnet",
    },
  },
  {
    field: "system_prompt",
    change: { kind: "text" as const, before_chars: 512, after_chars: 1024 },
  },
  {
    field: "env.OPENAI_API_KEY",
    change: { kind: "masked" as const, before: "••••abc1", after: "••••xyz9" },
  },
  { field: "env.BUZZ_LOG", change: { kind: "added" as const } },
  {
    field: "relay_url",
    change: { kind: "masked" as const, before: "••••", after: "••••" },
  },
  // Array value — args are atomic; rendered via JSON.stringify (position 6: last visible in tooltip)
  {
    field: "agent_args",
    change: {
      kind: "value" as const,
      before: ["acp"],
      after: ["acp", "--verbose"],
    },
  },
  // 7th entry — truncated in tooltip (cap is 6); visible in uncapped banner
  {
    field: "parallelism",
    change: { kind: "value" as const, before: 1, after: 4 },
  },
  // 8th entry — truncated in tooltip; visible in uncapped banner
  {
    field: "args",
    change: { kind: "masked" as const, before: "••••", after: "••••" },
  },
];

const STANDALONE_AGENT = {
  pubkey: TEST_IDENTITIES.alice.pubkey,
  name: "Local Agent",
  status: "running" as const,
  needsRestart: true,
  restartDiff: DIFF_ENTRIES,
};

const PERSONA_AGENT = {
  pubkey: TEST_IDENTITIES.bob.pubkey,
  name: "Persona Agent",
  personaId: "builtin:fizz",
  status: "running" as const,
  needsRestart: true,
  restartDiff: DIFF_ENTRIES,
};

/** Running agent with no config drift — badge must be absent. */
const NO_DRIFT_AGENT = {
  pubkey: TEST_IDENTITIES.tyler.pubkey,
  name: "Stable Agent",
  status: "running" as const,
  needsRestart: false,
};

/** Agent with only 3 diff entries — tooltip shows all, no "and N more". */
const SHORT_DIFF_AGENT = {
  pubkey: TEST_IDENTITIES.charlie.pubkey,
  name: "Short Diff Agent",
  status: "running" as const,
  needsRestart: true,
  restartDiff: [
    {
      field: "model",
      change: { kind: "value" as const, before: null, after: "gpt-5" },
    },
    {
      field: "some_unknown_field",
      change: { kind: "value" as const, before: "old", after: "new" },
    },
    { field: "env.MY_TOKEN", change: { kind: "removed" as const } },
  ],
};

/**
 * Inactive agent with a friendly error AND a restart diff. Opening its card
 * forces the panel to open on the Runtime tab (opensRuntimeTab logic).
 * Used to assert: Runtime tab is active, hero badge visible, uncapped banner.
 */
const INACTIVE_FRIENDLY_ERROR_AGENT = {
  pubkey: TEST_IDENTITIES.outsider.pubkey,
  name: "Error Restart Agent",
  status: "stopped" as const,
  needsRestart: true,
  restartDiff: DIFF_ENTRIES,
  lastError: "Agent reported error (code -32002): llm model not found",
  lastErrorCode: -32002,
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

test.describe("restart-diff screenshots", () => {
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

  // ── Badge presence ──────────────────────────────────────────────────────────

  test("01-grid-standalone-restart-badge", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT, NO_DRIFT_AGENT],
    });

    await gotoAgentsView(page);

    const agentCard = page.getByTestId(
      `managed-agent-${STANDALONE_AGENT.pubkey}`,
    );
    await expect(agentCard).toBeVisible({ timeout: 10_000 });
    await expect(
      agentCard.getByText("Restart required", { exact: true }),
    ).toBeVisible();

    const stableCard = page.getByTestId(
      `managed-agent-${NO_DRIFT_AGENT.pubkey}`,
    );
    await expect(stableCard).toBeVisible({ timeout: 10_000 });
    await expect(
      stableCard.getByText("Restart required", { exact: true }),
    ).toHaveCount(0);

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
    await expect(
      personaCard.getByText("Restart required", { exact: true }),
    ).toBeVisible();

    await waitForAnimations(page);
    await personaCard.screenshot({
      path: `${SHOTS}/02-grid-persona-restart-badge.png`,
    });
  });

  // ── Tooltip on card badge + no-button-ancestor (B4: badge is sibling of the overlay button) ─

  test("03-card-badge-tooltip-and-no-button-ancestor", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT, NO_DRIFT_AGENT],
    });

    await gotoAgentsView(page);

    const agentCard = page.getByTestId(
      `managed-agent-${STANDALONE_AGENT.pubkey}`,
    );
    await expect(agentCard).toBeVisible({ timeout: 10_000 });

    // B4: the tooltip trigger must have no <button> ancestor.
    // In AgentIdentityCard the badge lives in a z-30 container that is a
    // sibling of the z-10 overlay <button> — not a descendant of it.
    const badgeLocator = agentCard.getByTestId("restart-diff-badge");
    await expect(badgeLocator).toBeVisible();

    const hasButtonAncestor = await badgeLocator.evaluate((el) => {
      let node: Element | null = el.parentElement;
      while (node) {
        if (node.tagName === "BUTTON") return true;
        node = node.parentElement;
      }
      return false;
    });
    expect(hasButtonAncestor).toBe(false);

    // Hover to open tooltip
    await badgeLocator.hover();
    const tooltip = page.locator("[role=tooltip]");
    await expect(tooltip).toBeVisible({ timeout: 5_000 });

    // Tooltip is capped at 6 entries with "and 2 more" for 8-entry diff
    await expect(tooltip.getByText("Model:")).toBeVisible();
    await expect(tooltip.getByText("and 2 more")).toBeVisible();

    // Array value rendered as JSON.stringify in the value slot
    await expect(tooltip.getByText(/\["acp"\]/)).toBeVisible();

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/03-card-badge-tooltip.png` });
  });

  // ── Tooltip with short diff (no truncation) ─────────────────────────────────

  test("04-card-tooltip-short-diff-no-truncation", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [SHORT_DIFF_AGENT],
    });

    await gotoAgentsView(page);

    const agentCard = page.getByTestId(
      `managed-agent-${SHORT_DIFF_AGENT.pubkey}`,
    );
    const badge = agentCard.getByTestId("restart-diff-badge");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await badge.hover();
    const tooltip = page.locator("[role=tooltip]");
    await expect(tooltip).toBeVisible({ timeout: 5_000 });

    // All 3 entries visible
    await expect(tooltip.getByText("Model:")).toBeVisible();
    // Unknown field humanised generically
    await expect(tooltip.getByText("Some unknown field:")).toBeVisible();
    // No truncation line
    await expect(tooltip.getByText(/and \d+ more/)).toHaveCount(0);

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/04-short-diff-no-truncation.png` });
  });

  // ── Keyboard focus shows tooltip ──────────────────────────────────────────

  test("05-card-tooltip-keyboard-focus", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    const badge = page
      .getByTestId(`managed-agent-${STANDALONE_AGENT.pubkey}`)
      .getByTestId("restart-diff-badge");
    await badge.focus();
    const tooltip = page.locator("[role=tooltip]");
    await expect(tooltip).toBeVisible({ timeout: 5_000 });
  });

  // ── Persona card tooltip (AgentIdentityCard — pointer-events-auto) ─────────

  test("06-persona-card-tooltip", async ({ page }) => {
    await installMockBridge(page, {
      activePersonaIds: ["builtin:fizz"],
      managedAgents: [PERSONA_AGENT],
    });

    await gotoAgentsView(page);

    const personaCard = page.getByTestId(
      `persona-agent-row-${PERSONA_AGENT.personaId}`,
    );
    const badge = personaCard.getByTestId("restart-diff-badge");
    await expect(badge).toBeVisible({ timeout: 10_000 });
    await badge.hover();
    const tooltip = page.locator("[role=tooltip]");
    await expect(tooltip).toBeVisible({ timeout: 5_000 });

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/06-persona-card-tooltip.png` });
  });

  // ── Runtime-tab banner with full uncapped diff ────────────────────────────

  test("07-runtime-tab-restart-banner-with-diff", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    const agentButton = page.getByRole("button", {
      name: `${STANDALONE_AGENT.name} agent profile`,
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });

    // Switch to the Runtime tab
    await panel.getByRole("tab", { name: "Runtime" }).click();

    const banner = panel.getByTestId("needs-restart-banner");
    await expect(banner).toBeVisible({ timeout: 10_000 });

    // Banner shows the full uncapped diff list (8 entries, no "and N more")
    const diffList = banner.getByTestId("restart-diff-list");
    await expect(diffList).toBeVisible();
    // All 8 entries visible in banner (no cap).
    // exact: true prevents substring collision with "Agent args:" label.
    await expect(diffList.getByText("Args:", { exact: true })).toBeVisible();

    await waitForAnimations(page);
    await banner.screenshot({
      path: `${SHOTS}/07-runtime-tab-banner-with-diff.png`,
    });
  });

  test("08-runtime-tab-restart-banner-auto-on", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    const agentButton = page.getByRole("button", {
      name: `${STANDALONE_AGENT.name} agent profile`,
    });
    await agentButton.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });
    await panel.getByRole("tab", { name: "Runtime" }).click();

    const banner = panel.getByTestId("needs-restart-banner");
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await expect(
      banner.getByText("Buzz can restart it automatically"),
    ).toBeVisible();

    await waitForAnimations(page);
    await banner.screenshot({
      path: `${SHOTS}/08-runtime-tab-banner-auto-on.png`,
    });
  });

  test("09-runtime-tab-restart-banner-auto-off", async ({ page }) => {
    const agentAutoOff = {
      ...STANDALONE_AGENT,
      autoRestartOnConfigChange: false,
    };

    await installMockBridge(page, {
      managedAgents: [agentAutoOff],
    });

    await gotoAgentsView(page);

    const agentButton = page.getByRole("button", {
      name: `${agentAutoOff.name} agent profile`,
    });
    await agentButton.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });
    await panel.getByRole("tab", { name: "Runtime" }).click();

    const banner = panel.getByTestId("needs-restart-banner");
    await expect(banner).toBeVisible({ timeout: 10_000 });
    await expect(banner.getByText("Automatic restart is off")).toBeVisible();

    await waitForAnimations(page);
    await banner.screenshot({
      path: `${SHOTS}/09-runtime-tab-banner-auto-off.png`,
    });
  });

  // ── Side-panel badge on default (Info) tab ─────────────────────────────────

  test("10-profile-panel-badge-on-default-info-tab", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [STANDALONE_AGENT],
    });

    await gotoAgentsView(page);

    // Open profile panel via the agent card button — opens on Info tab by default
    const agentButton = page.getByRole("button", {
      name: `${STANDALONE_AGENT.name} agent profile`,
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });

    // Tab-independent badge: visible even on the Info tab (not only Runtime)
    const heroBadge = panel.getByTestId("restart-diff-badge");
    await expect(heroBadge).toBeVisible({ timeout: 5_000 });

    // Hero badge tooltip is functional: hover shows the diff list
    await heroBadge.hover();
    const heroTooltip = page.locator("[role=tooltip]");
    await expect(heroTooltip).toBeVisible({ timeout: 5_000 });
    await expect(heroTooltip.getByText("Model:")).toBeVisible();

    // The Runtime tab banner is NOT visible on Info tab
    await expect(panel.getByTestId("needs-restart-banner")).toHaveCount(0);

    await waitForAnimations(page);
    await panel.screenshot({
      path: `${SHOTS}/10-panel-badge-default-info-tab.png`,
    });
  });

  // ── Inactive + friendly-error: panel opens on Runtime, hero badge + banner ─

  test("11-inactive-friendly-error-panel-opens-runtime-tab", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [INACTIVE_FRIENDLY_ERROR_AGENT],
    });

    await gotoAgentsView(page);

    // The card for an inactive agent with a friendly error forces Runtime tab on open.
    const agentCard = page.getByTestId(
      `managed-agent-${INACTIVE_FRIENDLY_ERROR_AGENT.pubkey}`,
    );
    await expect(agentCard).toBeVisible({ timeout: 10_000 });
    await agentCard.click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });

    // Panel opens on Runtime tab (opensRuntimeTab = true for inactive+friendlyError)
    const runtimeTab = panel.getByRole("tab", { name: "Runtime" });
    await expect(runtimeTab).toHaveAttribute("aria-selected", "true", {
      timeout: 5_000,
    });

    // Hero badge is visible on the Runtime tab (tab-independent)
    const heroBadge = panel.getByTestId("restart-diff-badge");
    await expect(heroBadge).toBeVisible({ timeout: 5_000 });

    // Hero badge tooltip works on the Runtime tab too
    await heroBadge.hover();
    const heroTooltip = page.locator("[role=tooltip]");
    await expect(heroTooltip).toBeVisible({ timeout: 5_000 });
    await expect(heroTooltip.getByText("Model:")).toBeVisible();
    // Tooltip is capped at 6 + "and 2 more"
    await expect(heroTooltip.getByText("and 2 more")).toBeVisible();

    // Full uncapped banner also visible on Runtime tab
    const banner = panel.getByTestId("needs-restart-banner");
    await expect(banner).toBeVisible({ timeout: 5_000 });
    const diffList = banner.getByTestId("restart-diff-list");
    await expect(diffList).toBeVisible();
    // All 8 entries visible in banner (no cap) — last entry "args" is present.
    // exact: true prevents substring collision with "Agent args:" label.
    await expect(diffList.getByText("Args:", { exact: true })).toBeVisible();

    await waitForAnimations(page);
    await panel.screenshot({
      path: `${SHOTS}/11-inactive-error-panel-runtime-tab.png`,
    });
  });
});
