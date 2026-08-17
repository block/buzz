import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/doctor-cta";

// Channel ID for the seeded #general mock channel.
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

// Tyler is a managed agent seed in the mock bridge — used as the nudge sender.
const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "Tyler Agent";

/** Settle CSS / Web Animations before capture. */
async function settleAnimations(page: import("@playwright/test").Page) {
  await page.evaluate(() =>
    Promise.all(document.getAnimations().map((a) => a.finished)),
  );
}

/**
 * Wait for the Tauri invoke mock to be available, then call a mock command.
 * Mirrors the helper used in config-bridge-screenshots.spec.ts.
 */
async function invokeMockCommand(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<unknown> {
  await page.waitForFunction(
    () => {
      const w = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: { invoke?: unknown };
      };
      return (
        typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof w.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    null,
    { timeout: 5_000 },
  );
  return page.evaluate(
    async ({ command: cmd, payload: pl }) => {
      const w = window as Window & {
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
        w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ?? w.__TAURI_INTERNALS__?.invoke;
      if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
      return invoke(cmd, pl);
    },
    { command, payload },
  );
}

/**
 * Build a `buzz:config-nudge` sentinel body from a requirements array.
 * Mirrors the format nudge_body() in setup_mode.rs produces.
 */
function makeNudgeSentinel(
  agentName: string,
  agentPubkey: string,
  requirements: unknown[],
): string {
  const payload = JSON.stringify({
    agent_name: agentName,
    agent_pubkey: agentPubkey,
    requirements,
  });
  return `**${agentName}** needs configuration before it can respond.\n\n\`\`\`buzz:config-nudge\n${payload}\n\`\`\``;
}

/**
 * Inject a nudge message from the agent into #general, then navigate to the
 * channel so the card renders in the message list.
 */
async function injectNudgeAndNavigate(
  page: import("@playwright/test").Page,
  content: string,
): Promise<void> {
  await invokeMockCommand(page, "send_managed_agent_channel_message", {
    agentPubkey: AGENT_PUBKEY,
    channelId: GENERAL_CHANNEL_ID,
    content,
  });

  // Navigate to #general — the injected message is live-pushed so the card
  // renders in the current message list.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test.describe("doctor CTA nudge card screenshots", () => {
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

  /**
   * 01 — pure cli_login card (all requirements are cli_login, availability=available):
   * Tooling is installed but needs login — Doctor has no auth functionality
   * and would be a misleading dead-end. The card is purely informational:
   * no trigger, no CTA, no pointer/hover affordance. The inline copy
   * ("run `claude auth login` to authenticate") already tells the user
   * the exact command to run.
   */
  test("01-cli-login-available-informational", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "running" as const,
          channelNames: ["general"],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    const content = makeNudgeSentinel(AGENT_NAME, AGENT_PUBKEY, [
      {
        surface: "cli_login",
        probe_args: ["claude"],
        setup_copy: "run `claude auth login` to authenticate",
        availability: "available",
      },
    ]);

    await injectNudgeAndNavigate(page, content);

    // Wait for the nudge card to render.
    const card = page.locator("[data-config-nudge]").last();
    await expect(card).toBeVisible({ timeout: 10_000 });
    // Auth-only card is informational — no runtime settings CTA anywhere.
    await expect(card.getByText("Open Agent runtimes →")).toHaveCount(0);

    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/01-cli-login-available-informational.png`,
    });
  });

  /**
   * 02 — not_installed state: neither adapter nor CLI found.
   * Card shows "claude isn't installed" copy + an Agent runtimes CTA.
   */
  test("02-cli-login-not-installed-state", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped" as const,
          channelNames: ["general"],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    const content = makeNudgeSentinel(AGENT_NAME, AGENT_PUBKEY, [
      {
        surface: "cli_login",
        probe_args: ["claude"],
        setup_copy: "install Claude Code",
        availability: "not_installed",
      },
    ]);

    await injectNudgeAndNavigate(page, content);

    const card = page.locator("[data-config-nudge]").last();
    await expect(card).toBeVisible({ timeout: 10_000 });
    await expect(card.getByText(/isn't installed/)).toBeVisible();

    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/02-cli-login-not-installed-state.png`,
    });
  });

  /**
   * 03 — mixed card: one cli_login (adapter_missing) + one env_key requirement.
   * Each requirement row owns its CTA, right-aligned to a shared edge:
   * the cli_login row opens Agent runtimes and the env_key row shows
   * "Edit Agent →", both at the same x (vertically aligned).
   */
  test("03-mixed-requirements-inline-doctor-cta", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped" as const,
          channelNames: ["general"],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    const content = makeNudgeSentinel(AGENT_NAME, AGENT_PUBKEY, [
      {
        surface: "cli_login",
        probe_args: ["claude"],
        setup_copy: "install the Claude Code ACP adapter",
        availability: "adapter_missing",
      },
      {
        surface: "env_key",
        key: "ANTHROPIC_API_KEY",
      },
    ]);

    await injectNudgeAndNavigate(page, content);

    const card = page.locator("[data-config-nudge]").last();
    await expect(card).toBeVisible({ timeout: 10_000 });
    // Mixed card: cli_login opens Agent runtimes; env_key opens Edit Agent.
    await expect(card.getByText("Open Agent runtimes →")).toBeVisible();
    // Both per-row CTAs share the same right edge (vertically aligned).
    await expect(card.getByText("Edit Agent →", { exact: true })).toBeVisible();

    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/03-mixed-requirements-inline-doctor-cta.png`,
    });
  });

  /**
   * 04 — cli_missing state: ACP adapter present but underlying CLI absent.
   * Shows "claude CLI is missing" copy.
   */
  test("04-cli-login-cli-missing-state", async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped" as const,
          channelNames: ["general"],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });

    const content = makeNudgeSentinel(AGENT_NAME, AGENT_PUBKEY, [
      {
        surface: "cli_login",
        probe_args: ["claude"],
        setup_copy: "install the Claude CLI",
        availability: "cli_missing",
      },
    ]);

    await injectNudgeAndNavigate(page, content);

    const card = page.locator("[data-config-nudge]").last();
    await expect(card).toBeVisible({ timeout: 10_000 });
    await expect(card.getByText(/CLI is missing/)).toBeVisible();

    await card.scrollIntoViewIfNeeded();
    await settleAnimations(page);

    await card.screenshot({
      path: `${SHOTS}/04-cli-login-cli-missing-state.png`,
    });
  });
});
