import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/config-bridge";

// Use well-known test pubkeys that map to distinct config surface fixtures
const GOOSE_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CLAUDE_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const PRESPAWN_PUBKEY = TEST_IDENTITIES.bob.pubkey;
const CODEX_PUBKEY = TEST_IDENTITIES.charlie.pubkey;

const MANAGED_AGENTS = [
  { pubkey: GOOSE_PUBKEY, name: "Goose Agent", status: "running" as const },
  {
    pubkey: CLAUDE_PUBKEY,
    name: "Claude Code Agent",
    status: "running" as const,
  },
  {
    pubkey: PRESPAWN_PUBKEY,
    name: "Pre-Spawn Agent",
    status: "stopped" as const,
  },
  { pubkey: CODEX_PUBKEY, name: "Codex Agent", status: "running" as const },
];

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: { invoke?: unknown };
      };
      return (
        typeof tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof tauriWindow.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    null,
    { timeout: 5_000 },
  );
}

async function invokeMockCommand(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<unknown> {
  await waitForInvokeBridge(page);
  return page.evaluate(
    async ({ command: cmd, payload: pl }) => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };
      const invoke =
        tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        tauriWindow.__TAURI_INTERNALS__?.invoke;
      if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
      return invoke(cmd, pl);
    },
    { command, payload },
  );
}

async function activatePersonas(page: import("@playwright/test").Page) {
  for (const id of ["builtin:fizz"]) {
    await invokeMockCommand(page, "set_persona_active", { id, active: true });
  }
}

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await activatePersonas(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toBeVisible({
    timeout: 10_000,
  });
}

async function expandAgent(
  page: import("@playwright/test").Page,
  pubkey: string,
) {
  const agentRow = page.getByTestId(`managed-agent-${pubkey}`);
  await expect(agentRow).toBeVisible({ timeout: 5_000 });
  // Click the expandable button within the agent row
  await agentRow.locator("button").first().click();
  // Wait for the config panel to render (log row appears first, config is inside it)
  await expect(agentRow.getByTestId("managed-agent-log-row")).toBeVisible({
    timeout: 5_000,
  });
}

test.describe("config bridge screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("01 — goose full config panel", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    const logRow = page
      .getByTestId(`managed-agent-${GOOSE_PUBKEY}`)
      .getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/01-goose-full-config.png` });
  });

  test("02 — claude ACP config", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, CLAUDE_PUBKEY);

    const logRow = page
      .getByTestId(`managed-agent-${CLAUDE_PUBKEY}`)
      .getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/02-claude-acp-config.png` });
  });

  test("03 — pre-spawn state", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, PRESPAWN_PUBKEY);

    const logRow = page
      .getByTestId(`managed-agent-${PRESPAWN_PUBKEY}`)
      .getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/03-pre-spawn-state.png` });
  });

  test("04 — override visibility", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    // The goose fixture has model overridden from configFile by buzzExplicit.
    // Capture the config section (below the log content).
    const agentRow = page.getByTestId(`managed-agent-${GOOSE_PUBKEY}`);
    const configSection = agentRow.locator("text=Configuration").locator("..");
    await configSection.screenshot({
      path: `${SHOTS}/04-override-visibility.png`,
    });
  });

  test("05 — advanced section expanded", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    // Click the Advanced chevron button
    const agentRow = page.getByTestId(`managed-agent-${GOOSE_PUBKEY}`);
    const advancedButton = agentRow.getByRole("button", { name: /Advanced/i });
    await advancedButton.click();

    // Wait for advanced fields to appear
    await expect(agentRow.locator("text=Extension: developer")).toBeVisible();

    const logRow = agentRow.getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/05-advanced-expanded.png` });
  });

  test("06 — config provenance", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    // Each config value carries an inline provenance sentence below it.
    // The goose fixture's model is buzzExplicit ("Set in Buzz"); its provider
    // and thinking effort are configFile, so multiple rows render the same
    // "From config file (...)" sentence — assert the first match.
    const agentRow = page.getByTestId(`managed-agent-${GOOSE_PUBKEY}`);
    await expect(agentRow.getByText("Set in Buzz")).toBeVisible();
    await expect(
      agentRow
        .getByText("From config file (~/.config/goose/config.yaml)")
        .first(),
    ).toBeVisible();

    const logRow = agentRow.getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/06-config-provenance.png` });
  });

  test("07 — codex dual mode", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, CODEX_PUBKEY);

    const logRow = page
      .getByTestId(`managed-agent-${CODEX_PUBKEY}`)
      .getByTestId("managed-agent-log-row");
    await logRow.screenshot({ path: `${SHOTS}/07-codex-dual-mode.png` });
  });
});
