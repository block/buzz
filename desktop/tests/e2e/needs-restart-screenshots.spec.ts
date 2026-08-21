/**
 * Screenshot spec for the needsRestart badge and config diff (PR #1853 + diff
 * overlay work).
 *
 * Exercises:
 *   - Agent grid restart actions without a duplicate status badge.
 *   - Profile badge tooltip with itemised before→after diff.
 *   - Runtime-tab banner with full uncapped diff list.
 *   - Side-panel badge visible on the default (Info) tab — not only Runtime.
 *   - DOM validity: tooltip trigger has no <button> ancestor.
 *   - Generic rendering: unknown field ids, number/array values, masked values.
 */

import { expect, test, bootstrapE2ePage } from "../helpers/test";

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

/** Running agent with no config drift — restart action must be absent. */
const NO_DRIFT_AGENT = {
  pubkey: TEST_IDENTITIES.tyler.pubkey,
  name: "Stable Agent",
  status: "running" as const,
  needsRestart: false,
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

const RESTART_AGENT = {
  pubkey: "cd".repeat(32),
  name: "Restart Agent",
  status: "running" as const,
  needsRestart: true,
  restartDiff: DIFF_ENTRIES,
};

const START_AGENT = {
  pubkey: "ef".repeat(32),
  name: "Start Agent",
  status: "stopped" as const,
  needsRestart: false,
};

async function gotoAgentsView(page: import("@playwright/test").Page) {
  await bootstrapE2ePage(page, "/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
}

const WCAG_AA_NORMAL_TEXT_CONTRAST = 4.5;

async function renderedTextContrast(
  locator: import("@playwright/test").Locator,
): Promise<number> {
  return locator.evaluate((element) => {
    type Rgba = { r: number; g: number; b: number; a: number };

    const canvas = document.createElement("canvas");
    canvas.width = 1;
    canvas.height = 1;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("Could not create contrast test canvas");

    const parseColor = (color: string): Rgba => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      const [r, g, b, alpha] = context.getImageData(0, 0, 1, 1).data;
      return { r, g, b, a: alpha / 255 };
    };

    const composite = (foreground: Rgba, background: Rgba): Rgba => {
      const alpha = foreground.a + background.a * (1 - foreground.a);
      if (alpha === 0) return { r: 0, g: 0, b: 0, a: 0 };

      return {
        r:
          (foreground.r * foreground.a +
            background.r * background.a * (1 - foreground.a)) /
          alpha,
        g:
          (foreground.g * foreground.a +
            background.g * background.a * (1 - foreground.a)) /
          alpha,
        b:
          (foreground.b * foreground.a +
            background.b * background.a * (1 - foreground.a)) /
          alpha,
        a: alpha,
      };
    };

    const backgroundLayers: Rgba[] = [];
    let current: Element | null = element;
    while (current) {
      backgroundLayers.push(
        parseColor(window.getComputedStyle(current).backgroundColor),
      );
      current = current.parentElement;
    }

    let renderedBackground: Rgba = { r: 255, g: 255, b: 255, a: 1 };
    for (const layer of backgroundLayers.reverse()) {
      renderedBackground = composite(layer, renderedBackground);
    }

    const renderedForeground = composite(
      parseColor(window.getComputedStyle(element).color),
      renderedBackground,
    );
    const luminance = (color: Rgba) => {
      const channels = [color.r, color.g, color.b].map((value) => {
        const channel = value / 255;
        return channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
    };
    const foregroundLuminance = luminance(renderedForeground);
    const backgroundLuminance = luminance(renderedBackground);

    return (
      (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
      (Math.min(foregroundLuminance, backgroundLuminance) + 0.05)
    );
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
    await expect(agentCard.getByText("Restart", { exact: true })).toBeVisible();
    await expect(agentCard.getByTestId("restart-diff-badge")).toHaveCount(0);

    const stableCard = page.getByTestId(
      `managed-agent-${NO_DRIFT_AGENT.pubkey}`,
    );
    await expect(stableCard).toBeVisible({ timeout: 10_000 });
    await expect(stableCard.getByText("Restart", { exact: true })).toHaveCount(
      0,
    );

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
      personaCard.getByText("Restart", { exact: true }),
    ).toBeVisible();
    await expect(personaCard.getByTestId("restart-diff-badge")).toHaveCount(0);

    await waitForAnimations(page);
    await personaCard.screenshot({
      path: `${SHOTS}/02-grid-persona-restart-badge.png`,
    });
  });

  test("03-running-restart-action", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [RESTART_AGENT],
    });

    await gotoAgentsView(page);

    const agentCard = page.getByTestId(`managed-agent-${RESTART_AGENT.pubkey}`);
    await expect(agentCard).toBeVisible({ timeout: 10_000 });
    const restartAction = page.getByTestId(
      `agent-runtime-start-${RESTART_AGENT.pubkey}`,
    );
    await expect(restartAction).toHaveText("Restart");
    await expect(restartAction).toHaveAttribute("aria-label", "Restart Agent");
    await expect(restartAction).toHaveClass(/bg-transparent/);
    await expect(restartAction).toHaveClass(/text-amber-800/);
    await expect(restartAction).toHaveClass(/dark:text-amber-400/);
    await expect(restartAction.locator("svg")).toHaveCount(0);
    await expect(restartAction).toHaveCSS("width", "72px");
    await expect(restartAction).toHaveCSS("height", "36px");
    await expect(agentCard.getByTestId("restart-diff-badge")).toHaveCount(0);
    await expect(page.locator("html")).toHaveClass(/light/);
    expect(await renderedTextContrast(restartAction)).toBeGreaterThanOrEqual(
      WCAG_AA_NORMAL_TEXT_CONTRAST,
    );

    await waitForAnimations(page);
    await agentCard.screenshot({
      path: `${SHOTS}/03-running-restart-action.png`,
    });
  });

  test("restart action meets WCAG AA contrast in dark mode", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem("buzz-theme", "catppuccin-mocha");
    });
    await installMockBridge(page, {
      managedAgents: [RESTART_AGENT],
    });

    await gotoAgentsView(page);

    const restartAction = page.getByTestId(
      `agent-runtime-start-${RESTART_AGENT.pubkey}`,
    );
    await expect(restartAction).toBeVisible();
    await expect(page.locator("html")).toHaveClass(/dark/);
    expect(await renderedTextContrast(restartAction)).toBeGreaterThanOrEqual(
      WCAG_AA_NORMAL_TEXT_CONTRAST,
    );
  });

  test("start and restart pills share geometry except for width", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [START_AGENT, RESTART_AGENT],
    });

    await gotoAgentsView(page);

    const startAction = page.getByTestId(
      `agent-runtime-start-${START_AGENT.pubkey}`,
    );
    const restartAction = page.getByTestId(
      `agent-runtime-start-${RESTART_AGENT.pubkey}`,
    );
    await expect(startAction).toHaveText("Start");
    await expect(restartAction).toHaveText("Restart");

    const relativeGeometry = async (locator: typeof startAction) =>
      locator.evaluate((element) => {
        const frame = element.parentElement?.parentElement?.parentElement;
        if (!frame) throw new Error("Could not resolve avatar frame");
        const bounds = element.getBoundingClientRect();
        const frameBounds = frame.getBoundingClientRect();
        return {
          centerX: bounds.x + bounds.width / 2 - frameBounds.x,
          centerY: bounds.y + bounds.height / 2 - frameBounds.y,
          height: bounds.height,
          width: bounds.width,
        };
      });

    const [start, restart] = await Promise.all([
      relativeGeometry(startAction),
      relativeGeometry(restartAction),
    ]);
    expect(start.height).toBeCloseTo(restart.height, 2);
    expect(start.centerX).toBeCloseTo(restart.centerX, 2);
    expect(start.centerY).toBeCloseTo(restart.centerY, 2);
    expect(start.width).toBeCloseTo(56, 2);
    expect(restart.width).toBeCloseTo(72, 2);
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
