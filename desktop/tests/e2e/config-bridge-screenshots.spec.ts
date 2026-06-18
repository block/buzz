import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/config-bridge";

// Use well-known test pubkeys that map to distinct config surface fixtures
const GOOSE_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const PRESPAWN_PUBKEY = TEST_IDENTITIES.bob.pubkey;
const RUNTIME_OVERRIDE_PUBKEY = TEST_IDENTITIES.outsider.pubkey;
// Synthetic agent whose config surface mixes four distinct provenance origins
// (matches PUBKEY_MULTI_ORIGIN in e2eBridge buildMockConfigSurface).
const MULTI_ORIGIN_PUBKEY =
  "abc1230000000000000000000000000000000000000000000000000000000def";

const MANAGED_AGENTS = [
  { pubkey: GOOSE_PUBKEY, name: "Goose Agent", status: "running" as const },
  {
    pubkey: PRESPAWN_PUBKEY,
    name: "Pre-Spawn Agent",
    status: "stopped" as const,
  },
  {
    pubkey: RUNTIME_OVERRIDE_PUBKEY,
    name: "Runtime Override Agent",
    status: "running" as const,
  },
  {
    pubkey: MULTI_ORIGIN_PUBKEY,
    name: "Multi-Origin Agent",
    status: "running" as const,
  },
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

// Settle any in-flight Radix/expand animations on the agent row before a
// capture so screenshots are deterministic (team-management-screenshots pattern).
async function settleAnimations(
  page: import("@playwright/test").Page,
  pubkey: string,
) {
  await page
    .getByTestId(`managed-agent-${pubkey}`)
    .evaluate((el) =>
      Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)),
    );
}

test.describe("config bridge screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("01 — folded config panel", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    // The folded config panel: provenance sentences inline under each value,
    // no origin badges, no sources footer.
    const agentRow = page.getByTestId(`managed-agent-${GOOSE_PUBKEY}`);
    await expect(agentRow.getByText("Set in Buzz")).toBeVisible();
    await settleAnimations(page, GOOSE_PUBKEY);

    await agentRow
      .getByTestId("managed-agent-log-row")
      .screenshot({ path: `${SHOTS}/01-folded-config-panel.png` });
  });

  test("02 — live runtime override", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, RUNTIME_OVERRIDE_PUBKEY);

    // The headline new behavior: a runtimeOverride model shows the live model,
    // the persona baseline as a NON-struck secondary value, and the
    // "Live override (this session only)" sentence.
    const agentRow = page.getByTestId(
      `managed-agent-${RUNTIME_OVERRIDE_PUBKEY}`,
    );
    await expect(
      agentRow.getByText("Live override (this session only)"),
    ).toBeVisible();
    await expect(agentRow.getByText("gpt-4o", { exact: true })).toBeVisible();
    await settleAnimations(page, RUNTIME_OVERRIDE_PUBKEY);

    await agentRow
      .getByTestId("managed-agent-log-row")
      .screenshot({ path: `${SHOTS}/02-live-runtime-override.png` });
  });

  test("03 — provenance sentences", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, MULTI_ORIGIN_PUBKEY);

    // Each row carries a DIFFERENT inline provenance sentence so the frame
    // witnesses multiple distinct origins at once: "Set in Buzz" (model),
    // "Inherited from persona" (provider), "From environment variable
    // (GOOSE_MODE)" (mode), and "From config file (...)" (thinking/effort).
    const agentRow = page.getByTestId(`managed-agent-${MULTI_ORIGIN_PUBKEY}`);
    await expect(agentRow.getByText("Set in Buzz")).toBeVisible();
    await expect(agentRow.getByText("Inherited from persona")).toBeVisible();
    await expect(
      agentRow.getByText("From environment variable (GOOSE_MODE)"),
    ).toBeVisible();
    await expect(
      agentRow
        .getByText("From config file (~/.config/goose/config.yaml)")
        .first(),
    ).toBeVisible();
    await settleAnimations(page, MULTI_ORIGIN_PUBKEY);

    await agentRow
      .getByTestId("managed-agent-log-row")
      .screenshot({ path: `${SHOTS}/03-provenance-sentences.png` });
  });

  test("04 — pre-spawn state", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, PRESPAWN_PUBKEY);

    // ACP-only fields show "Available after agent starts" before spawn.
    const agentRow = page.getByTestId(`managed-agent-${PRESPAWN_PUBKEY}`);
    await expect(
      agentRow.getByText("Available after agent starts").first(),
    ).toBeVisible();
    await settleAnimations(page, PRESPAWN_PUBKEY);

    await agentRow
      .getByTestId("managed-agent-log-row")
      .screenshot({ path: `${SHOTS}/04-pre-spawn-state.png` });
  });

  test("05 — advanced expanded", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await openAgentsView(page);
    await expandAgent(page, GOOSE_PUBKEY);

    const agentRow = page.getByTestId(`managed-agent-${GOOSE_PUBKEY}`);
    const advancedButton = agentRow.getByRole("button", { name: /Advanced/i });
    await advancedButton.click();

    // Wait for advanced fields to appear, then settle the expand animation.
    await expect(agentRow.getByText("Extension: developer")).toBeVisible();
    await settleAnimations(page, GOOSE_PUBKEY);

    await agentRow
      .getByTestId("managed-agent-log-row")
      .screenshot({ path: `${SHOTS}/05-advanced-expanded.png` });
  });
});
