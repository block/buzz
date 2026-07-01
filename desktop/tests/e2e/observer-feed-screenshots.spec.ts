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
  await panel.evaluate((el) =>
    Promise.all(el.getAnimations({ subtree: true }).map((a) => a.finished)),
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

  test("01 — prompt context inline (collapsed-but-labeled)", async ({
    page,
  }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // session/prompt event: the per-turn prompt context that #1381 stopped
    // rendering inline. The payload contains sections that parsePromptText
    // extracts and the transcript renders as a collapsed PromptContextInline.
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

    // The inline context element should be visible with collapsed sections.
    await expect(
      feedPanel.getByTestId("transcript-prompt-context-inline"),
    ).toBeVisible({ timeout: 5_000 });
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/01-prompt-context-inline.png`,
    });
  });

  test("02 — system prompt title restored", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    const feedPanel = await openObserverFeedPanel(page, OBSERVER_AGENT_PUBKEY);

    // session/new event: the system prompt that RawRailActivity renders with the
    // real "System prompt" title (not "Captured N raw sections" as #1381 broke).
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

  test("05 — prompt context inline (sections expanded)", async ({ page }) => {
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

    await expect(
      feedPanel.getByTestId("transcript-prompt-context-inline"),
    ).toBeVisible({ timeout: 5_000 });

    // Click each section accordion button to expand it.
    const sectionButtons = feedPanel
      .getByTestId("transcript-prompt-context-sections")
      .getByRole("button");
    for (const btn of await sectionButtons.all()) {
      await btn.click();
    }
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
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
    for (const btn of await sectionButtons.all()) {
      await btn.click();
    }
    await settleAnimations(feedPanel);
    await feedPanel.screenshot({
      path: `${SHOTS}/06-system-prompt-expanded.png`,
    });
  });
});
