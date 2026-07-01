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
  {
    pubkey: GOOSE_PUBKEY,
    name: "Goose Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
  {
    pubkey: PRESPAWN_PUBKEY,
    name: "Pre-Spawn Agent",
    status: "stopped" as const,
    channelNames: ["agents"],
  },
  {
    pubkey: RUNTIME_OVERRIDE_PUBKEY,
    name: "Runtime Override Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
  {
    pubkey: MULTI_ORIGIN_PUBKEY,
    name: "Multi-Origin Agent",
    status: "running" as const,
    channelNames: ["agents"],
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

/**
 * Open the #agents channel and click an agent's avatar in the message list
 * to open the profile side panel, then navigate to the Runtime tab.
 */
async function openAgentProfileFromChannel(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await activatePersonas(page);

  // Navigate to the #agents channel
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Find the message row from this agent and click the avatar to open profile
  const messageRow = page.getByTestId("message-row").filter({
    has: page.getByText(agentName, { exact: false }),
  });
  await expect(messageRow.first()).toBeVisible({ timeout: 5_000 });
  await messageRow.first().getByRole("button").first().click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });

  // Click the Runtime tab to reveal the Configuration section.
  await panel.getByRole("tab", { name: "Runtime" }).click();

  // Wait for the first normalized config field ("Model") to appear.
  const configAnchor = panel.getByText("Model").first();
  await expect(configAnchor).toBeVisible({ timeout: 10_000 });

  // Scroll the panel's internal scroll container to the bottom so the
  // config section content is fully visible.
  await configAnchor.scrollIntoViewIfNeeded();
  await panel.evaluate((el) => {
    // The scrollable container is the profileBody div with overflow-y-auto.
    // Find it by checking which child actually scrolls.
    const scrollable =
      el.querySelector("[data-radix-scroll-area-viewport]") ??
      Array.from(el.querySelectorAll("*")).find(
        (child) => child.scrollHeight > child.clientHeight + 10,
      ) ??
      el;
    scrollable.scrollTop = scrollable.scrollHeight;
  });
  await panel.page().waitForTimeout(200);

  return panel;
}

// Settle any in-flight animations before capture.
async function settleAnimations(panel: import("@playwright/test").Locator) {
  await panel.evaluate((el) =>
    Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)),
  );
}

test.describe("config bridge screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 5).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 500));
      }
    });
  });

  test("01 — folded config panel", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });

    const panel = await openAgentProfileFromChannel(page, "Goose Agent");

    // The folded config panel: provenance sentences inline under each value.
    await expect(panel.getByText("Set in Buzz")).toBeVisible();
    await settleAnimations(panel);

    await panel.screenshot({ path: `${SHOTS}/01-folded-config-panel.png` });
  });

  test("02 — live runtime override", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });

    const panel = await openAgentProfileFromChannel(
      page,
      "Runtime Override Agent",
    );

    // A runtimeOverride model shows the live model, the persona baseline as a
    // NON-struck secondary value, and the "Live override" sentence.
    await expect(
      panel.getByText("Live override (this session only)"),
    ).toBeVisible();
    await expect(panel.getByText("gpt-4o", { exact: true })).toBeVisible();
    await settleAnimations(panel);

    await panel.screenshot({
      path: `${SHOTS}/02-live-runtime-override.png`,
    });
  });

  test("03 — provenance sentences", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });

    const panel = await openAgentProfileFromChannel(page, "Multi-Origin Agent");

    // Multiple distinct provenance origins visible at once.
    await expect(panel.getByText("Set in Buzz")).toBeVisible();
    await expect(panel.getByText("Inherited from persona")).toBeVisible();
    await expect(
      panel.getByText("From environment variable (GOOSE_MODE)"),
    ).toBeVisible();
    await expect(
      panel.getByText("From config file (~/.config/goose/config.yaml)").first(),
    ).toBeVisible();
    await settleAnimations(panel);

    await panel.screenshot({
      path: `${SHOTS}/03-provenance-sentences.png`,
    });
  });

  test("04 — pre-spawn state", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });

    const panel = await openAgentProfileFromChannel(page, "Pre-Spawn Agent");

    // ACP-only fields show "Available after agent starts" before spawn.
    await expect(
      panel.getByText("Available after agent starts").first(),
    ).toBeVisible();
    await settleAnimations(panel);

    await panel.screenshot({ path: `${SHOTS}/04-pre-spawn-state.png` });
  });

  test("05 — advanced expanded", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });

    const panel = await openAgentProfileFromChannel(page, "Goose Agent");

    const advancedButton = panel.getByRole("button", { name: /Advanced/i });
    await advancedButton.click();

    // Wait for advanced fields to appear.
    await expect(panel.getByText("Extension: developer")).toBeVisible();
    await settleAnimations(panel);

    await panel.screenshot({ path: `${SHOTS}/05-advanced-expanded.png` });
  });

  test("06 — profile side panel — Configuration section", async ({ page }) => {
    // charlie (554cef…) is the well-known test pubkey that the mock bridge
    // seeds as a bot owned by the test viewer, so isBot + isOwner + managedAgent
    // are all true — the Configuration section renders in the profile panel.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey:
            "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea",
          name: "Charlie",
          status: "running" as const,
          channelNames: ["agents"],
        },
      ],
    });
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForInvokeBridge(page);
    await page.getByTestId("channel-agents").click();
    await expect(page.getByTestId("chat-title")).toHaveText("agents");

    // Click an agent avatar in a full message row to open the profile side panel.
    await page
      .getByTestId("message-row")
      .filter({ has: page.locator('[data-testid^="message-avatar-"]') })
      .last()
      .getByRole("button")
      .first()
      .click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible({ timeout: 10_000 });

    // The Configuration section lives inside the Runtime tab — click it first.
    await panel.getByRole("tab", { name: "Runtime" }).click();

    // Wait for the config section to render and scroll it into view so
    // it is fully visible before capture.
    const configAnchor = panel.getByText("Model").first();
    await expect(configAnchor).toBeVisible({ timeout: 10_000 });
    await configAnchor.scrollIntoViewIfNeeded();
    // Scroll the panel's internal scroll container to the bottom so config
    // fields are fully visible, not just the heading at the edge.
    await panel.evaluate((el) => {
      const scrollable =
        el.querySelector("[data-radix-scroll-area-viewport]") ??
        Array.from(el.querySelectorAll("*")).find(
          (child) => child.scrollHeight > child.clientHeight + 10,
        ) ??
        el;
      scrollable.scrollTop = scrollable.scrollHeight;
    });
    await panel.page().waitForTimeout(200);

    // Settle any in-flight animations before capture.
    await panel.evaluate((el) =>
      Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)),
    );

    await panel.screenshot({
      path: `${SHOTS}/06-profile-side-panel-config.png`,
    });
  });
});
