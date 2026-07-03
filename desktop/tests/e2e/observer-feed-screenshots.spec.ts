import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/observer-feed";

// A running managed agent whose pubkey maps to the "agents" channel.
// Using tyler's pubkey so the mock bridge already knows this identity.
const OBSERVER_AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const NOW = new Date("2025-06-15T12:00:00Z").toISOString();

const MANAGED_AGENTS = [
  {
    pubkey: OBSERVER_AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

// Helper: wait until the seed hook is available in the page.
async function waitForSeedHook(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
}

// Helper: open the observer feed panel by navigating to #agents, clicking the
// agent avatar to open the profile panel, then clicking "View activity".
async function openObserverFeedPanel(
  page: import("@playwright/test").Page,
  agentPubkey: string,
) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForSeedHook(page);

  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  // Click any message row button to open the profile panel.
  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("Observer Agent", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();

  const profilePanel = page.getByTestId("user-profile-panel");
  await expect(profilePanel).toBeVisible({ timeout: 10_000 });

  // Click "View activity" to open the observer feed panel.
  const activityBtn = page.getByTestId(
    `user-profile-view-activity-${agentPubkey}`,
  );
  await expect(activityBtn).toBeVisible({ timeout: 5_000 });
  await activityBtn.click();

  const feedPanel = page.getByTestId("agent-session-thread-panel");
  await expect(feedPanel).toBeVisible({ timeout: 10_000 });
  return feedPanel;
}

// Helper: seed observer events into the store and wait for the panel to update.
async function seedObserverEvents(
  page: import("@playwright/test").Page,
  agentPubkey: string,
  events: Array<{
    seq: number;
    timestamp: string;
    kind: string;
    agentIndex: number | null;
    channelId: string | null;
    sessionId: string | null;
    turnId: string | null;
    payload: unknown;
  }>,
) {
  await page.evaluate(
    ({ pubkey, evts }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { pubkey: agentPubkey, evts: events },
  );
  // Let React re-render after the store update.
  await page.waitForTimeout(300);
}

async function settleAnimations(panel: import("@playwright/test").Locator) {
  // Only await finite animations — live surfaces (e.g. the turn liveness
  // indicator) run infinite loops whose `finished` promise never resolves.
  await panel.evaluate((el) =>
    Promise.all(
      el
        .getAnimations({ subtree: true })
        .filter((a) => {
          const timing = a.effect?.getTiming();
          return timing?.iterations !== Number.POSITIVE_INFINITY;
        })
        .map((a) => a.finished),
    ),
  );
}

test.describe("observer feed screenshots", () => {
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

  test("01 — prompt context dialog (via Checks ingress)", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // session/prompt event: per-turn prompt context. The payload contains
    // sections that parsePromptText extracts; the transcript keeps them out
    // of the feed until the CheckCheck footer toggle opens the
    // PromptContextDialog modal.
    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/prompt",
          params: {
            prompt: [
              {
                type: "text",
                text: "[Buzz event: Kind 9]\nContent: @Observer Agent help me debug this",
              },
              {
                type: "text",
                text: "[Thread context]\nThis is the thread history with 3 prior messages.",
              },
              {
                type: "text",
                text: "[Channel context]\nYou are in #agents, a channel for AI coordination.",
              },
            ],
          },
        },
      },
    ]);

    // The context stays out of the feed until the CheckCheck ingress opens
    // the dialog.
    await expect(feedPanel.getByText("Prompt context")).toHaveCount(0);
    const contextToggle = feedPanel.getByTestId(
      "transcript-prompt-context-toggle",
    );
    await expect(contextToggle).toBeVisible({ timeout: 5_000 });
    await contextToggle.click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    await expect(
      dialog.getByTestId("transcript-prompt-context-sections"),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(dialog);
    await dialog.screenshot({
      path: `${SHOTS}/01-prompt-context-dialog.png`,
    });
  });

  test("02 — system prompt title restored", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // session/new event without a subsequent session/prompt: the system-prompt
    // item is never consumed by a turn bucket, so the grouper emits it as a
    // standalone single rendered by RawRailActivity with the "System prompt"
    // title. (This is the isolated test; see shot 11 for the full turn bundle.)
    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: null,
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/new",
          params: {
            systemPrompt:
              "[Base]\nYou are a helpful AI assistant running in Buzz.\n\n[System]\nYou are Observer Agent. You coordinate multi-agent workflows in the #agents channel.",
          },
        },
      },
    ]);

    // The system prompt rail item should show the restored title.
    await expect(feedPanel.getByText("System prompt")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/02-system-prompt-title-restored.png`,
    });
  });

  test("03 — permission outcome (approved)", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // Permission request followed by a selected (approved) response,
    // correlated by JSON-RPC id (fix #3: was broken for numeric ids before).
    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      // Request
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 42,
          method: "session/request_permission",
          params: {
            title: "Read file system",
            message: "Agent wants to read ~/.config/goose/config.yaml",
            options: [
              { optionId: "opt-allow", kind: "allow_once", name: "Allow once" },
              { optionId: "opt-deny", kind: "reject_once", name: "Deny" },
            ],
          },
        },
      },
      // Response (numeric id 42 — the exact case fix #3 restores)
      {
        seq: 2,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 42,
          result: {
            outcome: {
              outcome: "selected",
              optionId: "opt-allow",
            },
          },
        },
      },
    ]);

    // The permission row should show the "Approved (allow_once)" outcome.
    await expect(feedPanel.getByText(/Approved.*allow_once/)).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/03-permission-approved.png`,
    });
  });

  test("04 — permission outcome (cancelled)", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      // Request
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: "perm-cancelled-1",
          method: "session/request_permission",
          params: {
            title: "Write to output file",
            options: [
              { optionId: "opt-yes", kind: "allow_once", name: "Allow" },
            ],
          },
        },
      },
      // Cancelled response
      {
        seq: 2,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: "perm-cancelled-1",
          result: {
            outcome: {
              outcome: "cancelled",
            },
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("Cancelled")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/04-permission-cancelled.png`,
    });
  });

  test("05 — prompt context dialog (sections expanded)", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/prompt",
          params: {
            prompt: [
              {
                type: "text",
                text: "[Buzz event: Kind 9]\nContent: @Observer Agent help me debug this",
              },
              {
                type: "text",
                text: "[Thread context]\nThis is the thread history with 3 prior messages.",
              },
              {
                type: "text",
                text: "[Channel context]\nYou are in #agents, a channel for AI coordination.",
              },
            ],
          },
        },
      },
    ]);

    // Open the dialog via the CheckCheck ingress, then expand every section.
    const contextToggle = feedPanel.getByTestId(
      "transcript-prompt-context-toggle",
    );
    await expect(contextToggle).toBeVisible({ timeout: 5_000 });
    await contextToggle.click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });

    const sectionButtons = dialog
      .getByTestId("transcript-prompt-context-sections")
      .getByRole("button");
    for (const btn of await sectionButtons.all()) {
      await btn.click();
    }
    await settleAnimations(dialog);
    await dialog.screenshot({
      path: `${SHOTS}/05-prompt-context-expanded.png`,
    });
  });

  test("06 — system prompt sections expanded", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: null,
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/new",
          params: {
            systemPrompt:
              "[Base]\nYou are a helpful AI assistant running in Buzz.\n\n[System]\nYou are Observer Agent. You coordinate multi-agent workflows in the #agents channel.",
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("System prompt")).toBeVisible({
      timeout: 5_000,
    });

    // Open the outer ActivityRow <details> to reveal the section content,
    // then click each section accordion button to expand it (mirrors shot 05 —
    // system-prompt sections now render as React button accordions, not native
    // <details> elements).
    await feedPanel.getByTestId("transcript-metadata-item").evaluate((el) => {
      if (el.tagName === "DETAILS") (el as HTMLDetailsElement).open = true;
      for (const details of el.querySelectorAll("details")) {
        details.open = true;
      }
    });
    const sectionButtons = feedPanel
      .getByTestId("transcript-metadata-item")
      .getByTestId("transcript-prompt-context-sections")
      .getByRole("button");
    const allSectionButtons = await sectionButtons.all();
    expect(allSectionButtons.length).toBeGreaterThan(0);
    for (const btn of allSectionButtons) {
      await btn.click();
    }
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/06-system-prompt-expanded.png`,
    });
  });

  test("07 — current_mode_update lifecycle status line", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-007",
        turnId: "turn-007",
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-007",
            update: {
              sessionUpdate: "current_mode_update",
              currentModeId: "plan",
            },
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("Mode")).toBeVisible({ timeout: 5_000 });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/07-current-mode-update.png`,
    });
  });

  test("08 — usage_update lifecycle status line (coalesced)", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      // Two usage frames — only the last should be visible.
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-008",
        turnId: "turn-008",
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-008",
            update: {
              sessionUpdate: "usage_update",
              used: 1200,
              size: 8192,
            },
          },
        },
      },
      {
        seq: 2,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-008",
        turnId: "turn-008",
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-008",
            update: {
              sessionUpdate: "usage_update",
              used: 3450,
              size: 8192,
              cost: { amount: 0.0018, currency: "USD" },
            },
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("Usage")).toBeVisible({ timeout: 5_000 });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/08-usage-update-coalesced.png`,
    });
  });

  test("09 — available_commands_update lifecycle status line", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-009",
        turnId: "turn-009",
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-009",
            update: {
              sessionUpdate: "available_commands_update",
              availableCommands: [
                {
                  name: "create_plan",
                  description: "Create a structured plan for the task",
                },
                {
                  name: "research_codebase",
                  description: "Research and understand the codebase",
                },
                {
                  name: "execute_steps",
                  description: "Execute the planned steps",
                },
              ],
            },
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("Commands")).toBeVisible({
      timeout: 5_000,
    });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/09-available-commands-update.png`,
    });
  });

  test("10 — config_option_update lifecycle status line", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "acp_read",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-010",
        turnId: "turn-010",
        payload: {
          method: "session/update",
          params: {
            sessionId: "session-010",
            update: {
              sessionUpdate: "config_option_update",
              configOptions: [
                {
                  id: "model",
                  name: "Model",
                  type: "select",
                  currentValue: "gpt-4o",
                },
                {
                  id: "mode",
                  name: "Mode",
                  type: "select",
                  currentValue: "auto",
                },
              ],
            },
          },
        },
      },
    ]);

    await expect(feedPanel.getByText("Config")).toBeVisible({ timeout: 5_000 });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/10-config-option-update.png`,
    });
  });

  test("11 — first-turn bundle: user bubble + Checks ingress dialog", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // Full realistic pool.rs first-turn wire sequence:
    // turn_started → session/new → session_resolved → session/prompt
    // Verifies the prompt bundle keeps context out of the feed: the system
    // prompt and prompt context both live in the dialog opened via the
    // CheckCheck footer ingress, with system-prompt sections listed first.
    await seedObserverEvents(page, OBSERVER_AGENT_PUBKEY, [
      {
        seq: 1,
        timestamp: NOW,
        kind: "turn_started",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: null,
        turnId: "turn-001",
        payload: { source: "channel", triggeringEventIds: [] },
      },
      {
        seq: 2,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: null,
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 1,
          method: "session/new",
          params: {
            systemPrompt:
              "[Base]\nYou are a helpful AI assistant running in Buzz.\n\n[System]\nYou are Observer Agent. You coordinate multi-agent workflows in the #agents channel.",
          },
        },
      },
      {
        seq: 3,
        timestamp: NOW,
        kind: "session_resolved",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: { sessionId: "session-001", isNewSession: true },
      },
      {
        seq: 4,
        timestamp: NOW,
        kind: "acp_write",
        agentIndex: 0,
        channelId: CHANNEL_ID,
        sessionId: "session-001",
        turnId: "turn-001",
        payload: {
          jsonrpc: "2.0",
          id: 2,
          method: "session/prompt",
          params: {
            prompt: [
              {
                type: "text",
                text: "[Buzz event: Kind 9]\nContent: @Observer Agent help me debug this",
              },
              {
                type: "text",
                text: "[Thread context]\nThis is the thread history with 3 prior messages.",
              },
            ],
          },
        },
      },
    ]);

    // User message bubble should be visible (anchors the prompt bundle).
    await expect(feedPanel.getByTestId("transcript-prompt-bundle")).toBeVisible(
      { timeout: 5_000 },
    );
    // Neither the system prompt nor the prompt context renders in the feed.
    await expect(feedPanel.getByText("System prompt")).toHaveCount(0);
    await expect(feedPanel.getByText("Prompt context")).toHaveCount(0);

    // Open the dialog: system-prompt sections (Base/System) come before the
    // prompt-context sections (Buzz event/Thread context).
    await feedPanel.getByTestId("transcript-prompt-context-toggle").click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible({ timeout: 5_000 });
    const sectionTitles = await dialog
      .getByTestId("transcript-prompt-context-sections")
      .locator("article")
      .allInnerTexts();
    expect(sectionTitles.length).toBe(4);
    expect(sectionTitles[0]).toContain("Base");
    expect(sectionTitles[1]).toContain("System");
    await settleAnimations(dialog);
    await dialog.screenshot({
      path: `${SHOTS}/11-first-turn-ordering.png`,
    });
  });
});
